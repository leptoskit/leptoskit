//! Custom domains: attaching one to an environment, proving it, and what a
//! release may route.
//!
//! A hostname is routed for one environment only. Three things keep that:
//!
//! - a domain routes nothing until its challenge record has been read from DNS,
//!   which only whoever runs the domain can publish;
//! - a verified row holds the hostname in `claim`, which is unique, so two
//!   verifications of one hostname can't both commit;
//! - a domain a release has carried to the cluster is `routed`, whether that
//!   release was applied or failed part-way, and stays so until a release
//!   without it is applied. Detached, or its environment deleted, while routed,
//!   it keeps its claim until then, or until the cluster's worker has removed
//!   the environment;
//! - a rollback's recorded routes are cut down to what the environment holds
//!   now, so no release routes a hostname someone else has since proved.

use std::collections::BTreeSet;
use std::future::Future;

use leptoskit_config::cloud::Domain as Hostname;
use leptoskit_db::Id;
use leptoskit_rpc as rpc;
use serde::Deserialize;
use serde::Serialize;

use super::audit;
use super::audit::Actor;
use super::authorize;
use super::dns::MAX_RECORD_LEN;
use super::dns::MAX_RECORDS;
use super::identity::client;
use super::projects::environment;
use super::services;
use crate::access::Permission;
use crate::domains::DnsRecord;
use crate::domains::DomainStatus;
use crate::domains::DomainSummary;
use crate::domains::MAX_DOMAINS;
use crate::domains::challenge_name;
use crate::domains::challenge_value;
use crate::tables::Domain;
use crate::tables::Environment;
use crate::tables::NewDomain;
use crate::tables::Placement;

/// How many random bytes a challenge token has.
const TOKEN_BYTES: usize = 16;

/// The longest custom domain: a DNS name's 253 characters, less the
/// challenge label and its dot.
const MAX_HOSTNAME_LEN: usize = 253 - crate::domains::CHALLENGE_LABEL.len() - 1;

fn invalid(why: impl std::fmt::Display) -> rpc::Error {
	rpc::Error::invalid(format!("invalid_domain: {why}"))
}

fn not_found(environment: &str, hostname: &str) -> rpc::Error {
	rpc::Error::not_found(format!(
		"domain_not_found: `{environment}` has no domain `{hostname}`"
	))
}

fn taken(hostname: &str) -> rpc::Error {
	rpc::Error::conflict(format!(
		"domain_taken: `{hostname}` is verified for another environment; detach it there first"
	))
}

/// The host of an origin such as `https://cloud.example.com:8443`.
fn origin_host(origin: &str) -> &str {
	let rest = origin
		.split_once("://")
		.map_or(origin, |(_, rest)| rest)
		.split('/')
		.next()
		.unwrap_or_default();
	rest.rsplit_once(':')
		.filter(|(_, port)| port.chars().all(|digit| digit.is_ascii_digit()))
		.map_or(rest, |(host, _)| host)
}

fn at_or_under(hostname: &str, domain: &str) -> bool {
	!domain.is_empty()
		&& (hostname == domain
			|| hostname
				.strip_suffix(domain)
				.is_some_and(|rest| rest.ends_with('.')))
}

/// Checks that `hostname` can be a custom domain here: a hostname by the
/// rules of `leptoskit.toml`, and nothing at or under the domain this
/// cloud serves environments on or is itself reached at.
fn check(hostname: &str, apps_domain: &str, origin: &str) -> rpc::Result<()> {
	// Its challenge record's name must fit in DNS too.
	if hostname.len() > MAX_HOSTNAME_LEN {
		return Err(invalid(format!(
			"a custom domain is at most {MAX_HOSTNAME_LEN} characters, such as `notes.example.com`"
		)));
	}

	Hostname::parse(hostname).map_err(invalid)?;

	let origin = origin_host(origin).to_ascii_lowercase();
	let apps_domain = apps_domain.to_ascii_lowercase();

	for own in [apps_domain.as_str(), origin.as_str()] {
		if at_or_under(hostname, own) {
			return Err(invalid(format!(
				"`{hostname}` is under `{own}`, which is the cloud's own"
			)));
		}
	}

	Ok(())
}

fn status(row: &Domain) -> DomainStatus {
	if row.removing {
		DomainStatus::Removing
	} else if row.claim.is_some() {
		DomainStatus::Verified
	} else {
		DomainStatus::Pending
	}
}

fn summary(row: &Domain, target: Option<&str>) -> DomainSummary {
	let mut records = vec![DnsRecord {
		kind: "TXT".to_owned(),
		name: challenge_name(&row.hostname),
		value: challenge_value(&row.token),
	}];

	if let Some(target) = target {
		records.push(DnsRecord {
			kind: "CNAME".to_owned(),
			name: row.hostname.clone(),
			value: target.to_owned(),
		});
	}

	DomainSummary {
		hostname: row.hostname.clone(),
		status: status(row),
		records,
		verified_at: row.verified_at,
		created_at: row.created_at,
	}
}

/// The hostname the cloud serves `environment` at, once it has been
/// deployed: where a custom domain points.
async fn target(
	db: &impl leptoskit_db::Read,
	environment: Id<Environment>,
) -> leptoskit_db::Result<Option<String>> {
	Ok(Placement::query()
		.filter(|placement| placement.environment_id.eq(environment))
		.first(db)
		.await?
		.map(|placement| placement.hostname))
}

async fn named(
	db: &impl leptoskit_db::Read,
	environment: Id<Environment>,
	hostname: &str,
) -> leptoskit_db::Result<Option<Domain>> {
	Domain::query()
		.filter(|row| row.environment_id.eq(environment) & row.hostname.eq(hostname))
		.first(db)
		.await
}

pub(crate) async fn domains(
	cx: &rpc::QueryCx,
	organization: &str,
	project: &str,
	environment_name: &str,
) -> rpc::Result<Vec<DomainSummary>> {
	let (_, project) = authorize::project(
		cx.db(),
		cx.context(),
		organization,
		project,
		Permission::ViewProjects,
	)
	.await?;
	let environment_id = environment(cx.db(), &project, environment_name).await?.id;
	let target = target(cx.db(), environment_id).await?;
	let rows = Domain::query()
		.filter(|row| row.environment_id.eq(environment_id))
		.order_by(|row| row.hostname.asc())
		.all(cx.db())
		.await?;
	Ok(rows
		.iter()
		.map(|row| summary(row, target.as_deref()))
		.collect())
}

pub(crate) async fn attach_domain(
	cx: &rpc::MutationCx,
	organization: &str,
	project: &str,
	environment_name: &str,
	hostname: &str,
) -> rpc::Result<DomainSummary> {
	let (access, project) = authorize::project(
		cx.tx(),
		cx.context(),
		organization,
		project,
		Permission::ManageConfig,
	)
	.await?;
	let environment_id = environment(cx.tx(), &project, environment_name).await?.id;
	let services = services(cx.context())?;
	check(
		hostname,
		&services.shipping().apps_domain,
		services.origin(),
	)?;
	let target = target(cx.tx(), environment_id).await?;
	let rows = Domain::query()
		.filter(|row| row.environment_id.eq(environment_id))
		.all(cx.tx())
		.await?;

	if let Some(mut row) = rows.into_iter().find(|row| row.hostname == hostname) {
		// Attached again while it was going: it stays, still verified.
		if row.removing {
			row.removing = false;
			row.save(cx.tx()).await?;
			audit::record(
				cx.tx(),
				&Actor::from(&access.caller),
				"domain.attach",
				Some(access.organization.id),
				Some(project.id),
				Some(format!("{environment_name} {hostname}")),
				&client(cx.context()),
			)
			.await?;
		}

		return Ok(summary(&row, target.as_deref()));
	}

	super::billing::refuse_another_domain(cx.tx(), services.shipping(), &access.organization)
		.await?;
	let count = Domain::query()
		.filter(|row| row.environment_id.eq(environment_id))
		.count(cx.tx())
		.await?;

	if count >= MAX_DOMAINS as u64 {
		return Err(rpc::Error::conflict(format!(
			"too_many_domains: `{environment_name}` has {MAX_DOMAINS} domains; detach some first"
		)));
	}

	let token = super::secret::random_hex(TOKEN_BYTES)
		.map_err(|_| rpc::Error::server("no randomness to make a challenge with"))?;
	let row = NewDomain {
		environment_id,
		hostname: hostname.to_owned(),
		token,
		claim: None,
		verified_at: None,
		routed: false,
		removing: false,
		created_by: access.caller.account(),
	}
	.insert(cx.tx())
	.await?;
	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"domain.attach",
		Some(access.organization.id),
		Some(project.id),
		Some(format!("{environment_name} {hostname}")),
		&client(cx.context()),
	)
	.await?;
	Ok(summary(&row, target.as_deref()))
}

/// What `verify_domain` needs to know before it asks DNS, read as the
/// caller: who may not change the environment's domains never causes a
/// lookup.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Challenge {
	organization: String,
	project: String,
	environment: String,
	hostname: String,
}

impl rpc::Operation for Challenge {
	type Context = rpc::QueryCx;
	type Output = Option<DomainSummary>;

	const KIND: rpc::Kind = rpc::Kind::Query;
	const NAME: &'static str = "verify_domain";

	fn run(
		self,
		cx: &rpc::QueryCx,
	) -> impl Future<Output = rpc::Result<Option<DomainSummary>>> + Send {
		challenge(cx, self)
	}
}

/// The domain as it is when it needs no lookup: already verified.
async fn challenge(cx: &rpc::QueryCx, args: Challenge) -> rpc::Result<Option<DomainSummary>> {
	let (_, project) = authorize::project(
		cx.db(),
		cx.context(),
		&args.organization,
		&args.project,
		Permission::ManageConfig,
	)
	.await?;
	let environment_id = environment(cx.db(), &project, &args.environment).await?.id;
	let Some(row) = named(cx.db(), environment_id, &args.hostname).await? else {
		return Err(not_found(&args.environment, &args.hostname));
	};

	if row.claim.is_none() {
		return Ok(None);
	}

	let target = target(cx.db(), environment_id).await?;
	Ok(Some(summary(&row, target.as_deref())))
}

/// What DNS answered, taken to the row in one transaction. Only
/// [`verify_domain`] runs it: it isn't served, so no client says what DNS
/// holds.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Verify {
	organization: String,
	project: String,
	environment: String,
	hostname: String,
	records: Vec<String>,
}

impl rpc::Operation for Verify {
	type Context = rpc::MutationCx;
	type Output = DomainSummary;

	const KIND: rpc::Kind = rpc::Kind::Mutation;
	const NAME: &'static str = "verify_domain";

	fn run(self, cx: &rpc::MutationCx) -> impl Future<Output = rpc::Result<DomainSummary>> + Send {
		verify(cx, self)
	}
}

async fn verify(cx: &rpc::MutationCx, args: Verify) -> rpc::Result<DomainSummary> {
	let Verify {
		organization,
		project,
		environment: environment_name,
		hostname,
		records,
	} = args;
	let (access, project) = authorize::project(
		cx.tx(),
		cx.context(),
		&organization,
		&project,
		Permission::ManageConfig,
	)
	.await?;
	let environment_id = environment(cx.tx(), &project, &environment_name).await?.id;
	let Some(mut row) = named(cx.tx(), environment_id, &hostname).await? else {
		return Err(not_found(&environment_name, &hostname));
	};
	let target = target(cx.tx(), environment_id).await?;

	if row.claim.is_some() {
		return Ok(summary(&row, target.as_deref()));
	}

	// The row's token as it is now, not as it was when DNS was asked.
	let wanted = challenge_value(&row.token);

	if !records.iter().any(|record| record.trim() == wanted) {
		return Err(rpc::Error::conflict(format!(
			"domain_not_verified: no TXT record at `{}` holds `{wanted}` yet; DNS changes can \
			 take a while to be seen",
			challenge_name(&hostname)
		)));
	}

	let held = Domain::query()
		.filter(|other| other.claim.eq(Some(hostname.clone())))
		.first(cx.tx())
		.await?;

	if held.is_some() {
		return Err(taken(&hostname));
	}

	row.claim = Some(hostname.clone());
	row.verified_at = Some(leptoskit_db::now());
	// Two verifications that both saw the hostname free: the unique claim
	// lets one commit.
	row.save(cx.tx()).await.map_err(|error| {
		if error.is_conflict() {
			taken(&hostname)
		} else {
			error.into()
		}
	})?;
	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"domain.verify",
		Some(access.organization.id),
		Some(project.id),
		Some(format!("{environment_name} {hostname}")),
		&client(cx.context()),
	)
	.await?;
	Ok(summary(&row, target.as_deref()))
}

/// `verify_domain`'s body: who may, then DNS, then the row. The lookup is
/// outside any transaction, so a slow resolver holds nothing.
pub(crate) async fn verify_domain(
	cx: &rpc::ActionCx,
	organization: String,
	project: String,
	environment: String,
	hostname: String,
) -> rpc::Result<DomainSummary> {
	let args = Challenge {
		organization,
		project,
		environment,
		hostname,
	};

	if let Some(verified) = cx.query(args.clone()).await? {
		return Ok(verified);
	}

	let resolver = services(cx.context())?.shipping().resolver.clone();
	let records = resolver
		.txt(&challenge_name(&args.hostname))
		.await
		.map_err(|_| {
			rpc::Error::conflict(format!(
				"dns_unavailable: DNS couldn't be asked about `{}` just now; try again",
				args.hostname
			))
		})?;
	// Whatever the resolver sent, the transaction is given little of it.
	let records = records
		.into_iter()
		.take(MAX_RECORDS)
		.filter(|record| record.len() <= MAX_RECORD_LEN)
		.collect();
	cx.mutate(Verify {
		organization: args.organization,
		project: args.project,
		environment: args.environment,
		hostname: args.hostname,
		records,
	})
	.await
}

pub(crate) async fn detach_domain(
	cx: &rpc::MutationCx,
	organization: &str,
	project: &str,
	environment_name: &str,
	hostname: &str,
) -> rpc::Result<()> {
	let (access, project) = authorize::project(
		cx.tx(),
		cx.context(),
		organization,
		project,
		Permission::ManageConfig,
	)
	.await?;
	let environment_id = environment(cx.tx(), &project, environment_name).await?.id;
	let Some(mut row) = named(cx.tx(), environment_id, hostname).await? else {
		return Err(not_found(environment_name, hostname));
	};

	if row.removing {
		return Ok(());
	}

	// A domain the cluster may route keeps its claim until a release
	// without it is applied: otherwise someone else could verify the
	// hostname while this environment's routes still answer on it.
	if row.claim.is_some() && row.routed {
		row.removing = true;
		row.save(cx.tx()).await?;
	} else {
		row.delete(cx.tx()).await?;
	}

	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"domain.detach",
		Some(access.organization.id),
		Some(project.id),
		Some(format!("{environment_name} {hostname}")),
		&client(cx.context()),
	)
	.await?;
	Ok(())
}

/// The hostnames a release of `environment` may route besides its own:
/// its verified domains that aren't being removed, in order.
pub(crate) async fn for_release(
	db: &impl leptoskit_db::Read,
	environment: Id<Environment>,
) -> leptoskit_db::Result<Vec<String>> {
	let mut hostnames: Vec<String> = Domain::query()
		.filter(|row| row.environment_id.eq(environment))
		.all(db)
		.await?
		.into_iter()
		.filter(|row| row.claim.is_some() && !row.removing)
		.map(|row| row.hostname)
		.collect();
	hostnames.sort_unstable();
	Ok(hostnames)
}

/// A release of `environment` that routes `hostnames` is being handed to
/// its cluster: from now the cluster may answer on them, whatever becomes
/// of the release.
pub(crate) async fn routing(
	tx: &impl leptoskit_db::Connection,
	environment: Id<Environment>,
	hostnames: &[String],
) -> leptoskit_db::Result<()> {
	if hostnames.is_empty() {
		return Ok(());
	}

	let rows = Domain::query()
		.filter(|row| row.environment_id.eq(environment))
		.all(tx)
		.await?;

	for mut row in rows {
		if !row.routed && hostnames.contains(&row.hostname) {
			row.routed = true;
			row.save(tx).await?;
		}
	}

	Ok(())
}

/// A release of `environment` was applied, routing `routed`: its other
/// domains are off the cluster. Those waiting for that go, and their
/// hostnames are free.
pub(crate) async fn released(
	tx: &impl leptoskit_db::Connection,
	environment: Id<Environment>,
	routed: &BTreeSet<&str>,
) -> leptoskit_db::Result<()> {
	let rows = Domain::query()
		.filter(|row| row.environment_id.eq(environment) & row.routed.eq(true))
		.all(tx)
		.await?;

	for mut row in rows {
		if routed.contains(row.hostname.as_str()) {
			continue;
		}

		if row.removing {
			row.delete(tx).await?;
		} else {
			row.routed = false;
			row.save(tx).await?;
		}
	}

	Ok(())
}

/// `environment` is being deleted. Its domains go with it, except those
/// its cluster may still route when a removal was queued (`on_cluster`):
/// they keep their hostnames until the cluster's worker has removed the
/// environment ([`environment_destroyed`]).
pub(crate) async fn environment_removed(
	tx: &impl leptoskit_db::Connection,
	environment: Id<Environment>,
	on_cluster: bool,
) -> leptoskit_db::Result<()> {
	let rows = Domain::query()
		.filter(|row| row.environment_id.eq(environment))
		.all(tx)
		.await?;

	for mut row in rows {
		if on_cluster && row.routed && row.claim.is_some() {
			row.removing = true;
			row.save(tx).await?;
		} else {
			row.delete(tx).await?;
		}
	}

	Ok(())
}

/// The cluster no longer has `environment`: whatever hostnames it held are
/// free.
pub(crate) async fn environment_destroyed(
	tx: &impl leptoskit_db::Connection,
	environment: Id<Environment>,
) -> leptoskit_db::Result<()> {
	let rows = Domain::query()
		.filter(|row| row.environment_id.eq(environment))
		.all(tx)
		.await?;

	for mut row in rows {
		row.delete(tx).await?;
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_clouds_own_names_are_refused() {
		let origin = "https://cloud.example.com:8443";
		assert_eq!(origin_host(origin), "cloud.example.com");
		assert_eq!(origin_host("http://127.0.0.1:4000"), "127.0.0.1");
		assert_eq!(
			origin_host("https://cloud.example.com/"),
			"cloud.example.com"
		);

		for good in ["notes.example.org", "example.org", "a-b.c-d.example.net"] {
			assert!(check(good, "apps.example.com", origin).is_ok(), "{good}");
		}

		for bad in [
			"apps.example.com",
			"notes.apps.example.com",
			"cloud.example.com",
			"x.cloud.example.com",
			"notes.leptoskit.app",
			"*.example.org",
			"Notes.example.org",
			"notes.example.org.",
			"localhost",
			"10.0.0.1",
			"notes.example.org:443",
			"",
		] {
			let error = check(bad, "apps.example.com", origin).expect_err(bad);
			assert!(error.to_string().contains("invalid_domain"), "{error}");
		}

		// A name that only ends like the cloud's isn't under it.
		assert!(check("notapps.example.com", "apps.example.com", origin).is_ok());
		assert!(
			check(
				&format!("{}.org", "a".repeat(300)),
				"apps.example.com",
				origin
			)
			.is_err()
		);

		// However the operator spelled their own names.
		assert!(
			check(
				"x.cloud.example.com",
				"Apps.Example.com",
				"https://Cloud.Example.com"
			)
			.is_err()
		);
		assert!(check("x.apps.example.com", "Apps.Example.com", origin).is_err());

		// The longest that leaves room for its challenge record's name.
		let label = "a".repeat(63);
		let longest = format!("{label}.{label}.{label}.{}", "b".repeat(232 - 3 * 64));
		assert_eq!(longest.len(), 232);
		assert!(check(&longest, "apps.example.com", origin).is_ok());
		assert!(challenge_name(&longest).len() <= 253);
		assert!(check(&format!("c{longest}"), "apps.example.com", origin).is_err());
	}
}
