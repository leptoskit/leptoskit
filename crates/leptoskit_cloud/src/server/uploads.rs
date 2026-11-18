//! Uploads: `begin_upload` and `complete_upload`.
//!
//! An archive is the organisation's, under a key made from its ID and the
//! archive's digest: two organisations uploading the same bytes each store
//! them, and neither learns the other has. Within one organisation,
//! bytes it already has are not uploaded again.
//!
//! The bytes never pass through the control plane's API. `begin_upload`
//! answers a presigned `PUT` bound to the size and SHA-256 promised, and
//! `complete_upload` checks what the store holds before any deployment may
//! use it: the size, and the SHA-256, from the store's own checksum or by
//! streaming the object a chunk at a time.

use leptoskit_db::Id;
use leptoskit_rpc as rpc;
use serde::Deserialize;
use serde::Serialize;

use super::authorize;
use super::delta;
use super::services;
use super::storage::Bucket;
use super::storage::Verification;
use super::storage::archive_key;
use crate::access::Permission;
use crate::archive::Digest;
use crate::deployments::ArchiveSummary;
use crate::deployments::BeganUpload;
use crate::deployments::MAX_ARCHIVE_BYTES;
use crate::deployments::UploadTicket;
use crate::tables::NewSourceArchive;
use crate::tables::Organization;
use crate::tables::SourceArchive;

fn summary(archive: &SourceArchive, digest: &Digest) -> ArchiveSummary {
	ArchiveSummary {
		digest: digest.clone(),
		size: u64::try_from(archive.size).unwrap_or_default(),
		verified_at: archive.verified_at.unwrap_or(archive.created_at),
		expires_at: archive.expires_at,
	}
}

/// The organisation's archive with `digest`, verified or not.
async fn archive(
	db: &impl leptoskit_db::Read,
	organization: Id<Organization>,
	digest: &Digest,
) -> rpc::Result<Option<SourceArchive>> {
	Ok(SourceArchive::query()
		.filter(|archive| {
			archive.organization_id.eq(organization) & archive.digest.eq(digest.as_str())
		})
		.first(db)
		.await?)
}

pub(crate) async fn begin_upload(
	cx: &rpc::ActionCx,
	organization: String,
	project: String,
	digest: Digest,
	size: u64,
) -> rpc::Result<BeganUpload> {
	let began = cx
		.mutate(BeginUpload {
			organization,
			project,
			digest: digest.clone(),
			size,
		})
		.await?;
	let services = services(cx.context())?;
	let shipping = services.shipping();

	match began {
		Began::Stored(archive) => Ok(BeganUpload::Stored { archive }),
		Began::Upload {
			organization,
			archive,
		} => {
			let key = archive_key(organization, archive, &digest);
			let presigned = shipping.store.presign_put(
				Bucket::Sources,
				&key,
				size,
				&digest,
				shipping.settings.upload,
			)?;
			Ok(BeganUpload::Upload {
				ticket: UploadTicket {
					url: presigned.url,
					method: "PUT".to_owned(),
					headers: presigned.headers,
					expires_at: presigned.expires_at,
				},
			})
		}
	}
}

/// What [`BeginUpload`] decided.
#[derive(Clone, Serialize, Deserialize)]
enum Began {
	Stored(ArchiveSummary),
	Upload {
		organization: uuid::Uuid,
		archive: uuid::Uuid,
	},
}

/// `begin_upload`'s checks and row. Never registered.
#[derive(Clone, Serialize, Deserialize)]
struct BeginUpload {
	organization: String,
	project: String,
	digest: Digest,
	size: u64,
}

impl rpc::Operation for BeginUpload {
	type Context = rpc::MutationCx;
	type Output = Began;

	const KIND: rpc::Kind = rpc::Kind::Mutation;
	const NAME: &'static str = "begin_upload";

	fn run(self, cx: &rpc::MutationCx) -> impl Future<Output = rpc::Result<Began>> + Send {
		begin(cx, self)
	}
}

async fn begin(cx: &rpc::MutationCx, args: BeginUpload) -> rpc::Result<Began> {
	let (access, _) = authorize::project(
		cx.tx(),
		cx.context(),
		&args.organization,
		&args.project,
		Permission::Deploy,
	)
	.await?;
	let services = services(cx.context())?;
	let settings = services.shipping().settings;

	if args.size == 0 {
		return Err(rpc::Error::invalid("archive_empty: an archive has bytes"));
	}

	if args.size > MAX_ARCHIVE_BYTES {
		return Err(rpc::Error::invalid(format!(
			"archive_too_large: an archive is at most {} MiB",
			MAX_ARCHIVE_BYTES / 1024 / 1024
		)));
	}

	let size = i64::try_from(args.size)
		.map_err(|_| rpc::Error::invalid("archive_too_large: that is too many bytes"))?;
	let organization = access.organization.id;
	let now = leptoskit_db::now();
	let existing = match archive(cx.tx(), organization, &args.digest).await? {
		Some(mut archive) if archive.verified_at.is_some() => {
			let keep = now + delta(settings.archive_keep);

			if archive.expires_at < keep {
				archive.expires_at = keep;
				archive.save(cx.tx()).await?;
			}

			return Ok(Began::Stored(summary(&archive, &args.digest)));
		}
		// A digest names one sequence of bytes, so it has one size. A
		// ticket already given out is bound to the size it was begun with:
		// changing the size here would leave that ticket able to upload
		// more than the reservation counts.
		Some(archive) if archive.size != size => {
			return Err(rpc::Error::conflict(format!(
				"archive_size_conflict: an upload of {} was begun with {} bytes, not {}; begin it \
				 with the size it has",
				args.digest, archive.size, args.size
			)));
		}
		existing => existing,
	};

	// Added up here rather than with `Sum`: Postgres sums a `bigint` column
	// as `numeric`, which `leptoskit_db` doesn't read yet.
	let used: i64 = SourceArchive::query()
		.filter(|archive| archive.organization_id.eq(organization))
		.all(cx.tx())
		.await?
		.iter()
		.map(|archive| archive.size)
		.sum();
	let replaced = existing.as_ref().map_or(0, |archive| archive.size);
	let cap = i64::try_from(settings.storage_cap).unwrap_or(i64::MAX);

	if used - replaced + size > cap {
		return Err(rpc::Error::conflict(format!(
			"storage_full: `{}` keeps {} MiB of archives, and this one would take it past its {} \
			 MiB; archives no deployment uses expire after a while",
			access.organization.slug,
			used / 1024 / 1024,
			cap / 1024 / 1024
		)));
	}

	let expires_at = now + delta(settings.upload);

	let archive = match existing {
		Some(mut archive) => {
			archive.expires_at = expires_at;
			archive.save(cx.tx()).await?;
			archive
		}
		None => {
			NewSourceArchive {
				organization_id: organization,
				digest: args.digest.as_str().to_owned(),
				size,
				verified_at: None,
				expires_at,
				created_by: access.caller.account(),
			}
			.insert(cx.tx())
			.await?
		}
	};

	Ok(Began::Upload {
		organization: organization.uuid(),
		archive: archive.id.uuid(),
	})
}

pub(crate) async fn complete_upload(
	cx: &rpc::ActionCx,
	organization: String,
	project: String,
	digest: Digest,
) -> rpc::Result<ArchiveSummary> {
	let found = cx
		.mutate(Completing {
			organization: organization.clone(),
			project: project.clone(),
			digest: digest.clone(),
			step: Step::Find,
		})
		.await?;
	let (organization_id, archive, size) = match found {
		Completed::Done(summary) => return Ok(summary),
		Completed::Check {
			organization,
			archive,
			size,
		} => (organization, archive, size),
	};
	let services = services(cx.context())?;
	let store = &services.shipping().store;
	let key = archive_key(organization_id, archive, &digest);

	match store.verify(Bucket::Sources, &key, size, &digest).await? {
		Verification::Matches => {
			match cx
				.mutate(Completing {
					organization,
					project,
					digest,
					step: Step::Verified { archive, size },
				})
				.await?
			{
				Completed::Done(summary) => Ok(summary),
				Completed::Check { .. } => Err(rpc::Error::server("the upload wasn't recorded")),
			}
		}
		Verification::Missing => {
			Err(rpc::Error::not_found(format!(
				"upload_missing: nothing was uploaded for {digest}; PUT the archive to the \
				 ticket's address first"
			)))
		}
		Verification::Mismatch(why) => {
			store.delete(Bucket::Sources, &key).await?;
			cx.mutate(Completing {
				organization,
				project,
				digest: digest.clone(),
				step: Step::Drop { archive, size },
			})
			.await?;
			Err(rpc::Error::conflict(format!(
				"upload_mismatch: what was uploaded isn't the archive promised: {why}; upload it \
				 again"
			)))
		}
	}
}

/// Where `complete_upload` is.
#[derive(Clone, Copy, Serialize, Deserialize)]
enum Step {
	/// Find the upload.
	Find,
	/// The store holds what was promised for the archive row `archive` at
	/// `size` bytes: the archive may be deployed, if the row is still that
	/// one at that size.
	Verified { archive: uuid::Uuid, size: u64 },
	/// It doesn't: forget the upload, if the row is still the one checked.
	Drop { archive: uuid::Uuid, size: u64 },
}

/// What [`Completing`] found.
#[derive(Clone, Serialize, Deserialize)]
enum Completed {
	/// The archive is verified.
	Done(ArchiveSummary),
	/// It waits to be checked: its organisation, its row, and its promised
	/// size.
	Check {
		organization: uuid::Uuid,
		archive: uuid::Uuid,
		size: u64,
	},
}

/// One step of `complete_upload`, in its own transaction, as the caller.
/// Never registered.
#[derive(Clone, Serialize, Deserialize)]
struct Completing {
	organization: String,
	project: String,
	digest: Digest,
	step: Step,
}

impl rpc::Operation for Completing {
	type Context = rpc::MutationCx;
	type Output = Completed;

	const KIND: rpc::Kind = rpc::Kind::Mutation;
	const NAME: &'static str = "complete_upload";

	fn run(self, cx: &rpc::MutationCx) -> impl Future<Output = rpc::Result<Completed>> + Send {
		complete(cx, self)
	}
}

async fn complete(cx: &rpc::MutationCx, args: Completing) -> rpc::Result<Completed> {
	let (access, _) = authorize::project(
		cx.tx(),
		cx.context(),
		&args.organization,
		&args.project,
		Permission::Deploy,
	)
	.await?;
	let services = services(cx.context())?;
	let organization = access.organization.id;
	let not_found = || {
		rpc::Error::not_found(format!(
			"upload_not_found: no upload of {} was begun, or it expired; call `begin_upload` again",
			args.digest
		))
	};
	let mut archive = archive(cx.tx(), organization, &args.digest)
		.await?
		.ok_or_else(not_found)?;

	if archive.verified_at.is_some() {
		return Ok(Completed::Done(summary(&archive, &args.digest)));
	}

	let checked = |archive_id: uuid::Uuid, size: u64| {
		archive.id.uuid() == archive_id && u64::try_from(archive.size).ok() == Some(size)
	};

	match args.step {
		Step::Find => {
			Ok(Completed::Check {
				organization: organization.uuid(),
				archive: archive.id.uuid(),
				size: u64::try_from(archive.size).unwrap_or_default(),
			})
		}
		Step::Verified { archive: id, size } => {
			// The row changed while the store was checked: what was checked
			// isn't what this row promises, so it is checked again.
			if !checked(id, size) {
				return Err(rpc::Error::conflict(format!(
					"upload_changed: the upload of {} changed while it was checked; call \
					 `complete_upload` again",
					args.digest
				)));
			}

			let now = leptoskit_db::now();
			archive.verified_at = Some(now);
			archive.expires_at = now + delta(services.shipping().settings.archive_keep);
			archive.save(cx.tx()).await?;
			Ok(Completed::Done(summary(&archive, &args.digest)))
		}
		Step::Drop { archive: id, size } => {
			if checked(id, size) {
				archive.destroy(cx.tx()).await?;
			}

			Ok(Completed::Check {
				organization: organization.uuid(),
				archive: id,
				size,
			})
		}
	}
}
