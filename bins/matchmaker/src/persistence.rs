use crate::Config;
use mr_messages_lib::{GetRegisteredUserQuery, RegisteredUser};
use reqwest::Client;

pub async fn get_registered_user(
    client: &Client,
    config: &Config,
    request: GetRegisteredUserQuery,
) -> anyhow::Result<Option<RegisteredUser>> {
    let result = client
        .get(config.private_persistence_url.join("user").unwrap())
        .query(&request)
        .send()
        .await;

    let response = match result {
        Ok(response) => response,
        Err(err) => {
            log::error!("Failed to get a user: {:?}", err);
            anyhow::bail!(err);
        }
    };

    let registered_user: RegisteredUser = match response.json().await {
        Ok(user) => user,
        Err(err) => {
            log::error!("Failed to get a user: {:?}", err);
            anyhow::bail!(err);
        }
    };
    Ok(Some(registered_user))
}
