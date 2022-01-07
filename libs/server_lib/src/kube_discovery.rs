use bevy::log;
use k8s_openapi::api::core::v1::Pod;
use kube::{api::ListParams, Api, Client};
use reqwest::Url;

pub async fn discover_persistence() -> Option<Url> {
    let client = Client::try_default().await.map_err(|err| {
        log::warn!("Unable to detect kubernetes environment: {:?}", err);
        err
    }).ok()?;
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
    let persistence_url = format!("http://{}:8083", pod_ip).parse().unwrap();
    log::info!("Using \"{}\" as MUDDLE_PERSISTENCE_URL", persistence_url);

    Some(persistence_url)
}
