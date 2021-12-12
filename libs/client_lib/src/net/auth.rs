use bevy::log;
use core::slice::SlicePattern;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use url::Url;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpenIdConnectConfig {
    pub issuer: Url,
    pub authorization_endpoint: Url,
    pub token_endpoint: Option<Url>,
    pub token_introspection_endpoint: Option<Url>,
    pub userinfo_endpoint: Option<Url>,
    pub end_session_endpoint: Option<Url>,
    pub jwks_uri: Url,
    pub registration_endpoint: Option<Url>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    #[serde(default)]
    pub grant_types_supported: Vec<String>,
}

// The code is definitely read. A clippy bug?
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct OAuthResponse {
    pub state: String,
    pub code: String,
}

const CODE_VERIFIER_CHARS: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-.~_";

#[derive(Debug, Serialize)]
pub struct AuthCodeRequest {
    pub client_id: String,
    pub scope: String,
}

#[derive(Debug, Serialize)]
pub struct AuthRequestParams {
    pub client_id: String,
    pub redirect_uri: String,
    pub response_type: String,
    pub scope: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub state: String,
    pub access_type: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthCodeResponse {
    pub device_code: String,
    pub expires_in: u64,
    pub interval: u64,
    pub user_code: String,
    pub verification_url: String,
}

#[derive(Debug)]
pub struct AuthCodeErrorResponse {
    pub error_code: String,
}

#[derive(Debug, Serialize)]
pub struct AuthTokenRequest {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub code: String,
    pub code_verifier: String,
    pub grant_type: String,
    pub redirect_uri: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthTokenResponse {
    pub access_token: String,
    pub expires_in: u64,
    pub refresh_token: String,
    pub scope: String,
    pub token_type: String,
    pub id_token: String,
}

#[derive(Debug)]
pub enum AuthRequest {
    PasswordSignIn { username: String, password: String },
    RedirectUrlServerPort(u16),
    CancelOpenIDRequest,
    RequestGoogleAuth,
    RequestUnstoppableDomainsAuth { username: String },
    HandleOAuthResponse { state: String, code: String },
}

#[derive(Debug)]
pub enum AuthMessage {
    RedirectUrlServerIsReady,
    Success,
    WrongPasswordError,
    InvalidDomainError,
    UnavailableError,
}

pub struct AuthConfig {
    pub google_client_id: String,
    // Google OAuth requires it for desktop clients.
    pub google_client_secret: Option<String>,
    pub ud_client_id: String,
    pub ud_secret_id: String,
}

pub struct PendingOAuthRequest {
    client_id: String,
    client_secret: Option<String>,
    state_token: String,
    code_verifier: String,
    token_uri: Url,
    redirect_uri: String,
}

pub async fn serve_auth_requests(
    auth_config: AuthConfig,
    mut auth_request_rx: UnboundedReceiver<AuthRequest>,
    auth_message_tx: UnboundedSender<AuthMessage>,
) {
    let mut pending_request: Option<PendingOAuthRequest> = None;
    let mut req_redirect_uri = None;

    let client = reqwest::Client::new();
    let ethereum_rpc_url =
        Url::parse("https://mainnet.infura.io/v3/c4bb906ed6904c42b19c95825fe55f39").unwrap();
    let polygon_rpc_url =
        Url::parse("https://polygon-mainnet.infura.io/v3/c4bb906ed6904c42b19c95825fe55f39")
            .unwrap();
    let resolution = unstoppable_resolution::UnsResolutionProvider {
        http_client: client.clone(),
        ethereum_rpc_url: Arc::new(ethereum_rpc_url),
        polygon_rpc_url: Arc::new(polygon_rpc_url),
    };

    loop {
        match auth_request_rx.recv().await {
            Some(AuthRequest::PasswordSignIn { .. }) => unimplemented!(),
            Some(AuthRequest::RedirectUrlServerPort(_port)) => {
                log::trace!("Initialized redirect_uri");
                #[cfg(not(target_arch = "wasm32"))]
                {
                    req_redirect_uri = Some(format!("http://localhost:{}", _port));
                }

                #[cfg(target_arch = "wasm32")]
                {
                    req_redirect_uri = Some(format!(
                        "{}/auth",
                        web_sys::window().unwrap().location().origin().unwrap()
                    ));
                }

                auth_message_tx
                    .send(AuthMessage::RedirectUrlServerIsReady)
                    .expect("Failed to send an auth update");
            }
            Some(AuthRequest::CancelOpenIDRequest) => {
                pending_request = None;
            }
            Some(AuthRequest::RequestGoogleAuth) => {
                let (code_verifier, code_challenge) = code_challenge();
                let request = PendingOAuthRequest {
                    client_id: auth_config.google_client_id.clone(),
                    client_secret: auth_config.google_client_secret.clone(),
                    state_token: state_token(),
                    code_verifier,
                    token_uri: Url::parse("https://oauth2.googleapis.com/token").unwrap(),
                    redirect_uri: req_redirect_uri.clone().unwrap(),
                };

                let params = AuthRequestParams {
                    client_id: request.client_id.clone(),
                    redirect_uri: req_redirect_uri.clone().unwrap(),
                    response_type: "code".to_owned(),
                    scope: "openid email".to_owned(),
                    code_challenge,
                    code_challenge_method: "S256".to_owned(),
                    state: request.state_token.clone(),
                    access_type: "offline".to_owned(),
                };

                pending_request = Some(request);

                let url = format!(
                    "https://accounts.google.com/o/oauth2/v2/auth?{}",
                    serde_urlencoded::to_string(params).unwrap()
                );

                webbrowser::open(&url).expect("Failed to open a URL in browser");
            }
            Some(AuthRequest::RequestUnstoppableDomainsAuth { username }) => {
                let rel = "http://openid.net/specs/connect/1.0/issuer";
                let (user, domain) = username.split_once('@').unwrap_or(("", username.as_str()));
                let jrd = match resolution.domain_jrd(domain, user, rel, None).await {
                    Ok(jrd) => jrd,
                    Err(unstoppable_resolution::WebFingerResponseError::InvalidDomainName) => {
                        auth_message_tx
                            .send(AuthMessage::InvalidDomainError)
                            .expect("Failed to send an auth update");
                        continue;
                    }
                    Err(err) => {
                        log::error!("WebFinger error: {:?}", err);
                        continue;
                    }
                };
                log::debug!("Domain JRD: {:#?}", jrd);
                let Some(openid_config) = fetch_openid_config(&client, rel, &jrd).await else {
                    auth_message_tx
                        .send(AuthMessage::UnavailableError)
                        .expect("Failed to send an auth update");
                    continue;
                };

                let Some(token_uri) = openid_config.token_endpoint else {
                    auth_message_tx
                        .send(AuthMessage::UnavailableError)
                        .expect("Failed to send an auth update");
                    continue;
                };

                let (code_verifier, code_challenge) = code_challenge();
                let request = PendingOAuthRequest {
                    client_id: auth_config.ud_client_id.clone(),
                    client_secret: Some(auth_config.ud_secret_id.clone()),
                    state_token: state_token(),
                    code_verifier,
                    token_uri,
                    redirect_uri: req_redirect_uri.clone().unwrap(),
                };

                let params = AuthRequestParams {
                    client_id: request.client_id.clone(),
                    redirect_uri: req_redirect_uri.clone().unwrap(),
                    response_type: "code".to_owned(),
                    scope: "openid email wallet".to_owned(),
                    code_challenge,
                    code_challenge_method: "S256".to_owned(),
                    state: request.state_token.clone(),
                    access_type: "offline".to_owned(),
                };

                pending_request = Some(request);

                let url = format!(
                    "{}?{}",
                    openid_config.authorization_endpoint,
                    serde_urlencoded::to_string(params).unwrap()
                );

                webbrowser::open(&url).expect("Failed to open a URL in browser");
            }
            Some(AuthRequest::HandleOAuthResponse { state, code }) => {
                let Some(request) = &pending_request else {
                    log::warn!("Ignoring unexpected OAuth response");
                    continue;
                };

                if request.state_token != state {
                    log::warn!("Ignoring OAuth response: invalid state token");
                    continue;
                }

                let success = exchange_auth_code(&client, request, code, &auth_message_tx).await;

                if success {
                    pending_request = None;
                }
            }
            None => {
                return;
            }
        }
    }
}

async fn fetch_openid_config(
    client: &reqwest::Client,
    rel: &str,
    jrd: &unstoppable_resolution::JrdDocument,
) -> Option<OpenIdConnectConfig> {
    let link = jrd.links.iter().find(|link| link.rel == rel)?;
    let url = link
        .href
        .as_ref()?
        .join(".well-known/openid-configuration")
        .ok()?;
    client.get(url).send().await.ok()?.json().await.ok()
}

async fn exchange_auth_code(
    client: &reqwest::Client,
    request: &PendingOAuthRequest,
    code: String,
    auth_message_tx: &UnboundedSender<AuthMessage>,
) -> bool {
    let result = client
        .post(request.token_uri.clone())
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(
            serde_urlencoded::to_string(AuthTokenRequest {
                client_id: request.client_id.clone(),
                client_secret: request.client_secret.clone(),
                code,
                code_verifier: request.code_verifier.clone(),
                grant_type: "authorization_code".to_owned(),
                redirect_uri: request.redirect_uri.to_string(),
            })
            .unwrap(),
        )
        .send()
        .await;

    let (data, success) = match result {
        Ok(result) => {
            let is_success = result.status().is_success();
            (result.bytes().await, is_success)
        }
        Err(err) => {
            log::error!("Failed to fetch token: {:?}", err);
            auth_message_tx
                .send(AuthMessage::UnavailableError)
                .expect("Failed to send an auth update");
            return false;
        }
    };

    if success {
        let response = data.map_err(anyhow::Error::msg).and_then(|data| {
            serde_json::from_slice::<AuthTokenResponse>(data.as_slice()).map_err(anyhow::Error::msg)
        });

        match response {
            Ok(_response) => {
                auth_message_tx
                    .send(AuthMessage::Success)
                    .expect("Failed to send an auth update");
            }
            Err(err) => {
                log::error!("Failed to serialize token body: {:?}", err);
                auth_message_tx
                    .send(AuthMessage::UnavailableError)
                    .expect("Failed to send an auth update");
            }
        }
    } else {
        log::debug!(
            "{:?}",
            serde_json::from_slice::<serde_json::Value>(data.unwrap().as_slice())
        );
        log::error!("Auth token exchange failed");
        auth_message_tx
            .send(AuthMessage::UnavailableError)
            .expect("Failed to send an auth update");
    }

    success
}

pub fn google_client_id() -> Option<String> {
    std::option_env!("MUDDLE_GOOGLE_CLIENT_ID").map(str::to_owned)
}

pub fn google_client_secret() -> Option<String> {
    std::option_env!("MUDDLE_GOOGLE_CLIENT_SECRET").map(str::to_owned)
}

pub fn ud_client_id() -> Option<String> {
    std::option_env!("MUDDLE_UD_CLIENT_ID").map(str::to_owned)
}

pub fn ud_client_secret() -> Option<String> {
    std::option_env!("MUDDLE_UD_CLIENT_SECRET").map(str::to_owned)
}

fn code_challenge() -> (String, String) {
    use rand::{thread_rng, Rng};
    use sha2::Digest;

    let mut rng = thread_rng();
    let code_verifier: Vec<u8> = (0..128)
        .map(|_| {
            let i = rng.gen_range(0..CODE_VERIFIER_CHARS.len());
            CODE_VERIFIER_CHARS[i]
        })
        .collect();

    let mut sha = sha2::Sha256::new();
    sha.update(&code_verifier);
    let result = sha.finalize();

    let b64 = base64::encode(result);
    let challenge = b64
        .chars()
        .filter_map(|c| match c {
            '=' => None,
            '+' => Some('-'),
            '/' => Some('_'),
            x => Some(x),
        })
        .collect();

    (String::from_utf8(code_verifier).unwrap(), challenge)
}

fn state_token() -> String {
    use rand::{thread_rng, Rng};
    use sha2::Digest;

    let mut rng = thread_rng();
    let random_bytes = rng.gen::<[u8; 16]>();

    let mut sha = sha2::Sha256::new();
    sha.update(random_bytes);
    format!("{:x}", sha.finalize())
}
