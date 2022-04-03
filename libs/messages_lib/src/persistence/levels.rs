use crate::PaginationParams;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetLevelsRequest {
    pub user_filter: Option<GetLevelsUserFilter>,
    #[serde(flatten)]
    pub pagination: PaginationParams,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GetLevelsUserFilter {
    AuthorId(i64),
    BuilderId(i64),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LevelsListItem {
    pub id: i64,
    pub title: String,
    pub user_id: i64,
    pub user_name: Option<String>,
    pub parent_id: Option<i64>,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LevelDto {
    pub id: i64,
    pub title: String,
    pub data: serde_json::Value,
    pub user_id: i64,
    pub user_name: Option<String>,
    pub parent_id: Option<i64>,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GetLevelResponse {
    #[serde(flatten)]
    pub level: LevelDto,
    pub autosaved_versions: Vec<LevelsListItem>,
    pub level_permissions: Vec<LevelPermissionDto>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LevelPermissionDto {
    pub user_id: i64,
    pub user_name: Option<String>,
    pub created_at: chrono::NaiveDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PostLevelRequest {
    pub title: String,
    pub user_id: i64,
    pub data: LevelData,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PostLevelResponse {
    pub id: i64,
    pub data: serde_json::Value,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LevelData {
    Forked {
        parent_id: i64,
    },
    Autosaved {
        autosaved_level_id: i64,
        data: serde_json::Value,
    },
    Data {
        data: serde_json::Value,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PatchLevelRequest {
    pub title: Option<String>,
    pub builder_ids: Option<Vec<i64>>,
}
