use crate::utils::parse_jwt;
use jwt_compact::Claims;
use mr_utils_lib::JwtAuthClaims;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::fmt::{Debug, Formatter};

pub const AUTH_CONFIG_KEY: &str = "auth";

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct OfflineAuthConfig {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub token_uri: String,
    #[serde(default)]
    pub id_token: String,
    #[serde(default)]
    pub refresh_token: String,
}

impl Debug for OfflineAuthConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OfflineAuthConfig")
            .field("username", &self.username)
            .field("token_uri", &self.token_uri)
            .field(
                "id_token",
                if self.id_token.is_empty() {
                    &""
                } else {
                    &"[sensitive]"
                },
            )
            .field(
                "refresh_token",
                if self.refresh_token.is_empty() {
                    &""
                } else {
                    &"[sensitive]"
                },
            )
            .finish()
    }
}

impl OfflineAuthConfig {
    pub fn exists(&self) -> bool {
        !self.username.is_empty()
            && !self.token_uri.is_empty()
            && !self.refresh_token.is_empty()
            && !self.id_token.is_empty()
    }

    pub fn parse_token_data(&self) -> Result<Claims<JwtAuthClaims>, jwt_compact::ParseError> {
        parse_jwt(&self.id_token)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn write(name: &str, value: &impl Serialize) -> anyhow::Result<()> {
    let Some(project_dirs) = directories::ProjectDirs::from("", "", "muddle-run") else {
        return Err(anyhow::Error::msg("Failed to determine a project directory"));
    };
    let config_dir = project_dirs.config_dir();
    bevy::log::debug!("Writing \"{}\" config to {:?}", name, config_dir.join(name));
    std::fs::create_dir_all(config_dir)?;
    std::fs::write(config_dir.join(name), serde_json::to_string(value)?)?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn read<T: DeserializeOwned + Default>(name: &str) -> anyhow::Result<T> {
    let Some(project_dirs) = directories::ProjectDirs::from("", "", "muddle-run") else {
        return Err(anyhow::Error::msg("Failed to determine a project directory"));
    };
    let content = match std::fs::read(project_dirs.config_dir().join(name)) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(err) => return Err(err.into()),
    };
    let value = serde_json::from_slice(&content).unwrap_or_default();
    Ok(value)
}

#[cfg(target_arch = "wasm32")]
pub fn write(name: &str, value: &impl Serialize) -> anyhow::Result<()> {
    let window = web_sys::window().unwrap();
    let Some(local_storage) = window.local_storage().map_err(from_js_err)? else {
        return Err(anyhow::Error::msg("Failed to access local storage"));
    };
    local_storage
        .set_item(name, &serde_json::to_string(value)?)
        .map_err(from_js_err)?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn read<T: DeserializeOwned + Default>(name: &str) -> anyhow::Result<T> {
    let window = web_sys::window().unwrap();
    let Some(local_storage) = window.local_storage().map_err(from_js_err)? else {
        return Err(anyhow::Error::msg("Failed to access local storage"));
    };
    let value = local_storage
        .get_item(name)
        .map_err(from_js_err)?
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default();
    Ok(value)
}

#[cfg(target_arch = "wasm32")]
fn from_js_err(err: wasm_bindgen::JsValue) -> anyhow::Error {
    #[derive(Deserialize)]
    struct JsError {
        message: String,
    }

    let message = match serde_wasm_bindgen::from_value::<JsError>(err.clone()) {
        Ok(err) => err.message,
        _ => js_sys::JSON::stringify(&err)
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_else(|| "Unknown JS error".to_owned()),
    };
    anyhow::Error::msg(message)
}
