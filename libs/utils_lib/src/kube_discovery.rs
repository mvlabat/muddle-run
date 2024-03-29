#[cfg(feature = "bevy_logging")]
use bevy::log;
use k8s_openapi::api::core::v1::Pod;
use kube::{api::ListParams, Api, Client};
use reqwest::Url;

pub async fn discover_persistence(client: Client) -> Option<(Url, Url)> {
    log::info!("Kubernetes environment detected, trying to fetch mr-persistence pods...");

    let pods: Api<Pod> = Api::namespaced(client, "default");
    let lp = ListParams::default()
        .labels("app=muddle-run,service=mr-persistence")
        .timeout(0);
    let pods_list = pods
        .list(&lp)
        .await
        .map_err(|err| {
            log::warn!("Failed to fetch kubernetes pods: {:?}", err);
            err
        })
        .ok()?;

    let pod_ip = pods_list.items.first()?.status.as_ref()?.pod_ip.as_ref()?;
    let public_persistence_url = format!("http://{pod_ip}:8082").parse().unwrap();
    let private_persistence_url = format!("http://{pod_ip}:8083").parse().unwrap();
    log::info!(
        "Using \"{}\" as MUDDLE_PUBLIC_PERSISTENCE_URL",
        public_persistence_url
    );
    log::info!(
        "Using \"{}\" as MUDDLE_PRIVATE_PERSISTENCE_URL",
        private_persistence_url
    );

    Some((public_persistence_url, private_persistence_url))
}
