use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetUserResponse {
    pub id: i64,
    pub display_name: Option<String>,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

// Is returned in the response to `GetRegisteredUserQuery`.
// Note: don't expose it to other clients as emails are sensitive.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisteredUser {
    pub id: i64,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "code", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RegisterAccountError {
    UserWithEmailAlreadyExists(LinkAccount),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkAccountRequest {
    pub existing_account_jwt: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkAccount {
    pub user: RegisteredUser,
    pub login_methods: Vec<LinkAccountLoginMethod>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkAccountLoginMethod {
    pub issuer: String,
    pub login_hint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LinkAccountError {
    ClaimsMismatch,
    AlreadyLinked,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PatchUserRequest {
    pub display_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PatchUserError {
    DisplayNameTaken,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetRegisteredUserQuery {
    pub subject: String,
    pub issuer: String,
}
