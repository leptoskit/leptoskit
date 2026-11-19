//! `environment_status`: what an environment's cluster last said it runs.

use leptoskit_cloud_runtime::ProcessState;
use leptoskit_rpc as rpc;

use super::authorize;
use super::projects::environment;
use crate::access::Permission;
use crate::deployments::EnvironmentStatus;
use crate::deployments::ProcessCondition;
use crate::deployments::ProcessStatus;
use crate::tables::Cluster;
use crate::tables::Placement;
use crate::tables::StatusReport;

pub(crate) async fn environment_status(
	cx: &rpc::QueryCx,
	organization: &str,
	project: &str,
	environment_name: &str,
) -> rpc::Result<EnvironmentStatus> {
	let (_, project) = authorize::project(
		cx.db(),
		cx.context(),
		organization,
		project,
		Permission::ViewProjects,
	)
	.await?;
	let environment = environment(cx.db(), &project, environment_name).await?;
	let environment_id = environment.id;
	let placement = Placement::query()
		.filter(|placement| placement.environment_id.eq(environment_id))
		.first(cx.db())
		.await?;
	let cluster = match &placement {
		Some(placement) => {
			let cluster_id = placement.cluster_id;
			Cluster::query()
				.filter(|cluster| cluster.id.eq(cluster_id))
				.first(cx.db())
				.await?
				.map(|cluster| cluster.name)
		}
		None => None,
	};
	let report = StatusReport::query()
		.filter(|report| report.environment_id.eq(environment_id))
		.first(cx.db())
		.await?;
	let processes = report
		.as_ref()
		.map(|report| {
			report
				.observed
				.0
				.processes
				.iter()
				.map(|process| {
					ProcessStatus {
						name: process.name.clone(),
						ready: process.ready_replicas,
						desired: process.desired_replicas,
						image: process.image.clone(),
						restarts: process.observed_restarts,
						last_oom_kill_at: process.last_oom_kill,
						state: match &process.state {
							ProcessState::Sleeping => ProcessCondition::Sleeping,
							ProcessState::Pending => ProcessCondition::Pending,
							ProcessState::Ready => ProcessCondition::Ready,
							ProcessState::Failed { reason } => {
								ProcessCondition::Failed {
									reason: reason.clone(),
								}
							}
						},
					}
				})
				.collect()
		})
		.unwrap_or_default();
	Ok(EnvironmentStatus {
		environment: environment.name.clone(),
		cluster,
		hostname: placement.map(|placement| placement.hostname),
		reported_at: report.map(|report| report.reported_at),
		processes,
	})
}
