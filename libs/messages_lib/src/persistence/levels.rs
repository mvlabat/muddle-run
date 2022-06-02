use crate::PaginationParams;
use serde::{Deserialize, Serialize};
use serde_with::rust::display_fromstr::deserialize as deserialize_fromstr;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetLevelsRequest {
    #[serde(flatten)]
    pub user_filter: Option<GetLevelsUserFilter>,
    #[serde(flatten)]
    pub pagination: PaginationParams,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GetLevelsUserFilter {
    #[serde(deserialize_with = "deserialize_fromstr")]
    AuthorId(i64),
    #[serde(deserialize_with = "deserialize_fromstr")]
    BuilderId(i64),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LevelsListItem {
    pub id: i64,
    pub title: String,
    pub user_id: i64,
    pub user_name: Option<String>,
    pub parent_id: Option<i64>,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetLevelResponse {
    #[serde(flatten)]
    pub level: LevelDto,
    pub autosaved_versions: Vec<LevelsListItem>,
    pub level_permissions: Vec<LevelPermissionDto>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_levels_request_query() {
        let query = GetLevelsRequest {
            user_filter: Some(GetLevelsUserFilter::AuthorId(1)),
            pagination: PaginationParams {
                offset: 0,
                limit: 20,
            },
        };
        let serialized = serde_urlencoded::to_string(&query).unwrap();
        assert_eq!(&serialized, "author_id=1&offset=0&limit=20");
        let deserialized: GetLevelsRequest = serde_urlencoded::from_str(&serialized).unwrap();
        assert_eq!(deserialized, query);

        let query = GetLevelsRequest {
            user_filter: None,
            pagination: PaginationParams {
                offset: 0,
                limit: 20,
            },
        };
        let serialized = serde_urlencoded::to_string(&query).unwrap();
        assert_eq!(&serialized, "offset=0&limit=20");
        let deserialized: GetLevelsRequest = serde_urlencoded::from_str(&serialized).unwrap();
        assert_eq!(deserialized, query);
    }
}
