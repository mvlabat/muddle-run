use bevy::log;
use core::slice::SlicePattern;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use url::Url;

const AUTH0_DB_CONNECTION: &str = "Username-Password-Authentication";

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
pub struct SignUpRequestParams {
    pub client_id: String,
    pub email: String,
    pub password: String,
    pub connection: String,
}

#[derive(Debug, Deserialize)]
pub struct SignUpErrorResponse {
    pub code: String,
}

#[derive(Debug, Serialize)]
pub struct SignInRequestParams {
    pub client_id: String,
    pub grant_type: String,
    pub username: String,
    pub password: String,
    pub scope: String,
    pub device: String,
}

#[derive(Debug, Deserialize)]
pub struct SignInErrorResponse {
    pub error: String,
}

impl SignInRequestParams {
    pub fn new(client_id: String, username: String, password: String) -> Self {
        Self {
            client_id,
            grant_type: "password".to_owned(),
            username,
            password,
            scope: "openid email offline_access".to_owned(),
            device: format!("{} {}", whoami::devicename(), whoami::desktop_env()),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AuthRequestParams {
    pub client_id: String,
    pub login_hint: Option<String>,
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
    Password {
        username: String,
        password: String,
        is_sign_up: bool,
    },
    RedirectUrlServerPort(u16),
    CancelOpenIDRequest,
    RequestGoogleAuth,
    #[cfg(feature = "unstoppable_resolution")]
    RequestUnstoppableDomainsAuth {
        username: String,
    },
    HandleOAuthResponse {
        state: String,
        code: String,
    },
}

#[derive(Debug)]
pub enum AuthMessage {
    RedirectUrlServerIsReady,
    Success,
    WrongPasswordError,
    SignUpFailedError,
    #[cfg(feature = "unstoppable_resolution")]
    InvalidDomainError,
    UnavailableError,
}

pub struct AuthConfig {
    pub google_client_id: String,
    // Google OAuth requires it for desktop clients.
    pub google_client_secret: Option<String>,
    pub auth0_client_id: String,
    #[cfg(feature = "unstoppable_resolution")]
    pub ud_client_id: String,
    #[cfg(feature = "unstoppable_resolution")]
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

    #[cfg(feature = "unstoppable_resolution")]
    let resolution = {
        let ethereum_rpc_url =
            Url::parse("https://mainnet.infura.io/v3/c4bb906ed6904c42b19c95825fe55f39").unwrap();
        let polygon_rpc_url =
            Url::parse("https://polygon-mainnet.infura.io/v3/c4bb906ed6904c42b19c95825fe55f39")
                .unwrap();
        unstoppable_resolution::UnsResolutionProvider {
            http_client: client.clone(),
            ethereum_rpc_url: std::sync::Arc::new(ethereum_rpc_url),
            polygon_rpc_url: std::sync::Arc::new(polygon_rpc_url),
        }
    };

    loop {
        match auth_request_rx.recv().await {
            Some(AuthRequest::Password {
                username,
                password,
                is_sign_up: false,
            }) => {
                sign_in(
                    &client,
                    &SignInRequestParams::new(
                        auth_config.auth0_client_id.clone(),
                        username,
                        password,
                    ),
                    &auth_message_tx,
                )
                .await;
            }
            Some(AuthRequest::Password {
                username,
                password,
                is_sign_up: true,
            }) => {
                let params = SignUpRequestParams {
                    client_id: auth_config.auth0_client_id.clone(),
                    email: username,
                    password,
                    connection: AUTH0_DB_CONNECTION.to_owned(),
                };

                sign_up(&client, &params, &auth_message_tx).await;
            }
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
                    login_hint: None,
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
            #[cfg(feature = "unstoppable_resolution")]
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
                    login_hint: Some(domain.to_owned()),
                    redirect_uri: req_redirect_uri.clone().unwrap(),
                    response_type: "code".to_owned(),
                    scope: "openid email wallet offline_access".to_owned(),
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

#[cfg(feature = "unstoppable_resolution")]
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

async fn sign_up(
    client: &reqwest::Client,
    params: &SignUpRequestParams,
    auth_message_tx: &UnboundedSender<AuthMessage>,
) {
    let result = client
        .post("https://muddle-run.eu.auth0.com/dbconnections/signup")
        .json(params)
        .send()
        .await;

    let (data, success) = match result {
        Ok(result) => {
            let is_success = result.status().is_success();
            (result.bytes().await, is_success)
        }
        Err(err) => {
            log::error!("Failed to sign up: {:?}", err);
            auth_message_tx
                .send(AuthMessage::UnavailableError)
                .expect("Failed to send an auth update");
            return;
        }
    };

    if !success {
        log::error!("Failed to sign up");
        match data
            .ok()
            .and_then(|data| serde_json::from_slice::<SignUpErrorResponse>(data.as_slice()).ok())
        {
            Some(response) => {
                if response.code == "invalid_signup" {
                    auth_message_tx
                        .send(AuthMessage::SignUpFailedError)
                        .expect("Failed to send an auth update");
                    return;
                }
            }
            None => {
                log::error!("Failed to parse sign up error code");
            }
        }
        auth_message_tx
            .send(AuthMessage::UnavailableError)
            .expect("Failed to send an auth update");
        return;
    }

    sign_in(
        client,
        &SignInRequestParams::new(
            params.client_id.clone(),
            params.email.clone(),
            params.password.clone(),
        ),
        auth_message_tx,
    )
    .await;
}

async fn sign_in(
    client: &reqwest::Client,
    body: &SignInRequestParams,
    auth_message_tx: &UnboundedSender<AuthMessage>,
) {
    let result = client
        .post("https://muddle-run.eu.auth0.com/oauth/token")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(serde_urlencoded::to_string(body).unwrap())
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
            return;
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
        log::error!("Failed to sign in");
        match data
            .ok()
            .and_then(|data| serde_json::from_slice::<SignInErrorResponse>(data.as_slice()).ok())
        {
            Some(response) => {
                if response.error == "invalid_grant" {
                    auth_message_tx
                        .send(AuthMessage::WrongPasswordError)
                        .expect("Failed to send an auth update");
                    return;
                }
            }
            None => {
                log::error!("Failed to parse sign in error");
            }
        }
        auth_message_tx
            .send(AuthMessage::UnavailableError)
            .expect("Failed to send an auth update");
    }
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

pub fn auth0_client_id() -> Option<String> {
    std::option_env!("MUDDLE_AUTH0_CLIENT_ID").map(str::to_owned)
}

#[cfg(feature = "unstoppable_resolution")]
pub fn ud_client_id() -> Option<String> {
    std::option_env!("MUDDLE_UD_CLIENT_ID").map(str::to_owned)
}

#[cfg(feature = "unstoppable_resolution")]
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
