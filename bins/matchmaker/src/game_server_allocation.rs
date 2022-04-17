use kube::Client;
use kube_derive::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(CustomResource, Debug, Serialize, Deserialize, Default, Clone, JsonSchema)]
#[kube(
    group = "allocation.agones.dev",
    version = "v1",
    kind = "GameServerAllocation",
    namespaced
)]
pub struct GameServerAllocationSpec {
    pub selectors: Vec<GameServerSelector>,
    pub scheduling: Option<String>,
    pub metadata: GameServerMetadata,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GameServerSelector {
    pub match_labels: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GameServerMetadata {
    #[serde(default)]
    pub labels: HashMap<String, String>,
    #[serde(default)]
    pub annotations: HashMap<String, String>,
}

#[derive(Clone)]
pub struct PostGameServerAllocationParams {
    pub user_id: Option<i64>,
    pub level_title: Option<String>,
    pub level_parent_id: Option<i64>,
    pub level_id: Option<i64>,
}

pub async fn post_game_server_allocation(
    client: Client,
    params: PostGameServerAllocationParams,
) -> kube::Result<()> {
    let api = kube::Api::namespaced(client, "default");
    api.create(
        &Default::default(),
        &GameServerAllocation {
            metadata: Default::default(),
            spec: GameServerAllocationSpec {
                selectors: vec![GameServerSelector {
                    match_labels: [("agones.dev/fleet".to_owned(), "mr-server".to_owned())]
                        .into_iter()
                        .collect(),
                }],
                scheduling: None,
                metadata: GameServerMetadata {
                    labels: Default::default(),
                    annotations: {
                        let mut metadata = HashMap::new();
                        if let Some(user_id) = params.user_id {
                            metadata.insert("user_id".to_owned(), user_id.to_string());
                        }
                        if let Some(level_title) = params.level_title {
                            metadata.insert("level_title".to_owned(), level_title);
                        }
                        if let Some(level_parent_id) = params.level_parent_id {
                            metadata
                                .insert("level_parent_id".to_owned(), level_parent_id.to_string());
                        }
                        if let Some(level_id) = params.level_id {
                            metadata.insert("level_id".to_owned(), level_id.to_string());
                        }
                        metadata
                    },
                },
            },
        },
    )
    .await?;
    Ok(())
}
