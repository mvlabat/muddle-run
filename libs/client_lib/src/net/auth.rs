use crate::{
    config_storage,
    config_storage::{OfflineAuthConfig, AUTH_CONFIG_KEY},
    utils::parse_jwt,
};
use bevy::{ecs::system::ResMut, log};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use core::slice::SlicePattern;
use mr_messages_lib::JwtAuthClaims;
use reqwest::IntoUrl;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
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

#[derive(Debug, Deserialize)]
pub struct OAuthResponse {
    pub state: String,
    pub code: String,
}

const CODE_VERIFIER_CHARS: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-.~_";
const AUTH0_TOKEN_ENDPOINT: &str = "https://muddle-run.eu.auth0.com/oauth/token";

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

#[derive(Serialize, Debug)]
pub enum AuthorizationCodeGrantType {
    #[serde(rename = "authorization_code")]
    Grant,
}

#[derive(Debug, Serialize)]
pub struct AuthTokenRequest {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub code: String,
    pub code_verifier: String,
    pub grant_type: AuthorizationCodeGrantType,
    pub redirect_uri: String,
}

#[derive(Serialize, Debug)]
pub enum RefreshTokenGrantType {
    #[serde(rename = "refresh_token")]
    Grant,
}

#[derive(Debug, Serialize)]
pub struct RefreshAuthTokenRequestParams {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub grant_type: RefreshTokenGrantType,
    pub refresh_token: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthTokenResponse {
    pub access_token: String,
    pub expires_in: u64,
    pub refresh_token: Option<String>,
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
    RefreshAuth(OfflineAuthConfig),
    HandleOAuthResponse {
        state: String,
        code: String,
    },
}

#[derive(Debug)]
pub enum AuthMessage {
    AuthHandlerIsReady,
    Success {
        id_token: String,
    },
    WrongPasswordError,
    SignUpFailedError,
    #[cfg(feature = "unstoppable_resolution")]
    InvalidDomainError,
    UnavailableError,
    InvalidOrExpiredAuthError,
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
    username: Option<String>,
    client_id: String,
    client_secret: Option<String>,
    state_token: String,
    code_verifier: String,
    token_uri: Url,
    redirect_uri: String,
}

pub fn read_offline_auth_config(mut offline_auth_config: ResMut<OfflineAuthConfig>) {
    let config: OfflineAuthConfig = match config_storage::read(AUTH_CONFIG_KEY) {
        Ok(config) => config,
        Err(err) => {
            log::error!("Failed to read auth config: {:?}", err);
            return;
        }
    };
    *offline_auth_config = config;
}

pub async fn serve_auth_requests(
    auth_config: AuthConfig,
    auth_request_rx: UnboundedReceiver<AuthRequest>,
    auth_message_tx: UnboundedSender<AuthMessage>,
) {
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

    let mut handler = AuthRequestsHandler {
        client,
        auth_config,
        auth_request_rx,
        auth_message_tx,
        pending_request: None,
        req_redirect_uri: None,
        #[cfg(feature = "unstoppable_resolution")]
        resolution,
    };
    handler.serve().await
}

pub struct AuthRequestsHandler {
    client: reqwest::Client,
    auth_config: AuthConfig,
    auth_request_rx: UnboundedReceiver<AuthRequest>,
    auth_message_tx: UnboundedSender<AuthMessage>,
    pending_request: Option<PendingOAuthRequest>,
    req_redirect_uri: Option<String>,
    #[cfg(feature = "unstoppable_resolution")]
    resolution: unstoppable_resolution::UnsResolutionProvider,
}

enum RequestParams<T> {
    Json(T),
    UrlEncoded(T),
}

impl AuthRequestsHandler {
    async fn serve(&mut self) {
        loop {
            match self.auth_request_rx.recv().await {
                Some(AuthRequest::Password {
                    username,
                    password,
                    is_sign_up: false,
                }) => {
                    self.sign_in(&SignInRequestParams::new(
                        self.auth_config.auth0_client_id.clone(),
                        username,
                        password,
                    ))
                    .await;
                }
                Some(AuthRequest::Password {
                    username,
                    password,
                    is_sign_up: true,
                }) => {
                    let params = SignUpRequestParams {
                        client_id: self.auth_config.auth0_client_id.clone(),
                        email: username,
                        password,
                        connection: AUTH0_DB_CONNECTION.to_owned(),
                    };

                    self.sign_up(&params).await;
                }
                Some(AuthRequest::RedirectUrlServerPort(_port)) => {
                    log::trace!("Initialized redirect_uri");
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        self.req_redirect_uri = Some(format!("http://localhost:{}", _port));
                    }

                    #[cfg(target_arch = "wasm32")]
                    {
                        self.req_redirect_uri = Some(format!(
                            "{}/auth",
                            web_sys::window().unwrap().location().origin().unwrap()
                        ));
                    }

                    self.send_auth_message(AuthMessage::AuthHandlerIsReady);
                }
                Some(AuthRequest::CancelOpenIDRequest) => {
                    self.pending_request = None;
                }
                Some(AuthRequest::RequestGoogleAuth) => {
                    let (code_verifier, code_challenge) = code_challenge();
                    let request = PendingOAuthRequest {
                        username: None,
                        client_id: self.auth_config.google_client_id.clone(),
                        client_secret: self.auth_config.google_client_secret.clone(),
                        state_token: state_token(),
                        code_verifier,
                        token_uri: Url::parse("https://oauth2.googleapis.com/token").unwrap(),
                        redirect_uri: self.req_redirect_uri.clone().unwrap(),
                    };

                    let params = AuthRequestParams {
                        client_id: request.client_id.clone(),
                        login_hint: None,
                        redirect_uri: self.req_redirect_uri.clone().unwrap(),
                        response_type: "code".to_owned(),
                        scope: "openid email".to_owned(),
                        code_challenge,
                        code_challenge_method: "S256".to_owned(),
                        state: request.state_token.clone(),
                        access_type: "offline".to_owned(),
                    };

                    self.pending_request = Some(request);

                    let url = format!(
                        "https://accounts.google.com/o/oauth2/v2/auth?{}",
                        serde_urlencoded::to_string(params).unwrap()
                    );

                    webbrowser::open(&url).expect("Failed to open a URL in browser");
                }
                #[cfg(feature = "unstoppable_resolution")]
                Some(AuthRequest::RequestUnstoppableDomainsAuth { username }) => {
                    self.request_ud_auth(username).await;
                }
                Some(AuthRequest::RefreshAuth(offline_auth_config)) => {
                    let token_data = offline_auth_config.parse_token_data();
                    let is_actual = token_data.map_or(false, |token_data| {
                        let expires_at = DateTime::<Utc>::from_utc(
                            NaiveDateTime::from_timestamp(token_data.claims.exp, 0),
                            Utc,
                        );
                        expires_at > Utc::now() - Duration::minutes(1)
                    });
                    if is_actual {
                        self.send_auth_message(AuthMessage::Success {
                            id_token: offline_auth_config.id_token,
                        });
                        continue;
                    }

                    let (client_id, client_secret) =
                        if offline_auth_config.token_uri.contains("google") {
                            (
                                self.auth_config.google_client_id.clone(),
                                self.auth_config.google_client_secret.clone(),
                            )
                        } else if offline_auth_config.token_uri.contains("auth0") {
                            (self.auth_config.auth0_client_id.clone(), None)
                        } else {
                            #[allow(unused_mut, unused_assignments)]
                            let mut result = None;
                            #[cfg(feature = "unstoppable_resolution")]
                            {
                                result = Some((
                                    self.auth_config.ud_client_id.clone(),
                                    Some(self.auth_config.ud_secret_id.clone()),
                                ));
                            }
                            match result {
                                Some(result) => result,
                                None => {
                                    self.send_auth_message(AuthMessage::InvalidOrExpiredAuthError);
                                    continue;
                                }
                            }
                        };

                    self.refresh_auth_token(
                        &offline_auth_config,
                        &RefreshAuthTokenRequestParams {
                            client_id,
                            client_secret,
                            grant_type: RefreshTokenGrantType::Grant,
                            refresh_token: offline_auth_config.refresh_token.clone(),
                        },
                    )
                    .await;
                }
                Some(AuthRequest::HandleOAuthResponse { state, code }) => {
                    let Some(request) = &self.pending_request else {
                        log::warn!("Ignoring unexpected OAuth response");
                        continue;
                    };

                    if request.state_token != state {
                        log::warn!("Ignoring OAuth response: invalid state token");
                        continue;
                    }

                    let success = self.exchange_auth_code(request, code).await;

                    if success {
                        self.pending_request = None;
                    }
                }
                None => {
                    return;
                }
            }
        }
    }

    #[cfg(feature = "unstoppable_resolution")]
    async fn request_ud_auth(&mut self, username: String) {
        let rel = "http://openid.net/specs/connect/1.0/issuer";
        let (user, domain) = username.split_once('@').unwrap_or(("", username.as_str()));
        let jrd = match self.resolution.domain_jrd(domain, user, rel, None).await {
            Ok(jrd) => jrd,
            Err(unstoppable_resolution::WebFingerResponseError::InvalidDomainName) => {
                self.send_auth_message(AuthMessage::InvalidDomainError);
                return;
            }
            Err(err) => {
                log::error!("WebFinger error: {:?}", err);
                return;
            }
        };
        log::debug!("Domain JRD: {:#?}", jrd);
        let Some(openid_config) = self.fetch_openid_config(rel, &jrd).await else {
            self.send_auth_message(AuthMessage::UnavailableError);
            return;
        };

        let Some(token_uri) = openid_config.token_endpoint else {
            self.send_auth_message(AuthMessage::UnavailableError);
            return;
        };

        let (code_verifier, code_challenge) = code_challenge();
        let request = PendingOAuthRequest {
            username: Some(username.clone()),
            client_id: self.auth_config.ud_client_id.clone(),
            client_secret: None,
            state_token: state_token(),
            code_verifier,
            token_uri,
            redirect_uri: self.req_redirect_uri.clone().unwrap(),
        };

        let params = AuthRequestParams {
            client_id: request.client_id.clone(),
            login_hint: Some(domain.to_owned()),
            redirect_uri: self.req_redirect_uri.clone().unwrap(),
            response_type: "code".to_owned(),
            scope: "openid email wallet offline_access".to_owned(),
            code_challenge,
            code_challenge_method: "S256".to_owned(),
            state: request.state_token.clone(),
            access_type: "offline".to_owned(),
        };

        self.pending_request = Some(request);

        let url = format!(
            "{}?{}",
            openid_config.authorization_endpoint,
            serde_urlencoded::to_string(params).unwrap()
        );

        webbrowser::open(&url).expect("Failed to open a URL in browser");
    }

    #[cfg(feature = "unstoppable_resolution")]
    async fn fetch_openid_config(
        &self,
        rel: &str,
        jrd: &unstoppable_resolution::JrdDocument,
    ) -> Option<OpenIdConnectConfig> {
        let link = jrd.links.iter().find(|link| link.rel == rel)?;
        let url = link
            .href
            .as_ref()?
            .join(".well-known/openid-configuration")
            .ok()?;
        self.client.get(url).send().await.ok()?.json().await.ok()
    }

    async fn sign_up(&self, params: &SignUpRequestParams) {
        let Some(response) = self
            .request::<(), SignUpErrorResponse, _, _>(
                "https://muddle-run.eu.auth0.com/dbconnections/signup",
                RequestParams::Json(params),
            )
            .await else {
            log::error!("Failed to sign up");
            return;
        };

        if let Err(err) = response {
            if err.code == "invalid_signup" {
                self.send_auth_message(AuthMessage::SignUpFailedError);
                return;
            }
        }

        self.sign_in(&SignInRequestParams::new(
            params.client_id.clone(),
            params.email.clone(),
            params.password.clone(),
        ))
        .await;
    }

    async fn sign_in(&self, params: &SignInRequestParams) {
        let username = params.username.clone();

        let response = self
            .request::<AuthTokenResponse, SignInErrorResponse, _, _>(
                AUTH0_TOKEN_ENDPOINT,
                RequestParams::UrlEncoded(params),
            )
            .await;

        let Some(response) = response else {
            log::error!("Failed to sign in");
            return;
        };

        match response {
            Ok(response) => {
                if parse_jwt::<JwtAuthClaims>(&response.id_token).is_err() {
                    log::error!("Failed to parse id_token from the response");
                    self.send_auth_message(AuthMessage::UnavailableError);
                    return;
                };

                self.send_auth_message(AuthMessage::Success {
                    id_token: response.id_token.clone(),
                });

                if let Some(refresh_token) = response.refresh_token {
                    if let Err(err) = config_storage::write(
                        AUTH_CONFIG_KEY,
                        &OfflineAuthConfig {
                            username,
                            token_uri: AUTH0_TOKEN_ENDPOINT.to_owned(),
                            id_token: response.id_token,
                            refresh_token,
                        },
                    ) {
                        log::error!("Failed to save auth config: {:?}", err);
                    }
                }
            }
            Err(err) => {
                if err.error == "invalid_grant" {
                    self.send_auth_message(AuthMessage::WrongPasswordError);
                }
            }
        }
    }

    async fn refresh_auth_token(
        &self,
        offline_auth_config: &OfflineAuthConfig,
        params: &RefreshAuthTokenRequestParams,
    ) -> bool {
        let Some(Ok(response)) = self
            .request::<AuthTokenResponse, serde_json::Value, _, _>(
                &offline_auth_config.token_uri,
                RequestParams::UrlEncoded(params)
            ).await else
        {
            log::error!("Failed to refresh an auth token");
            self.send_auth_message(AuthMessage::UnavailableError);
            return false;
        };

        if parse_jwt::<JwtAuthClaims>(&response.id_token).is_err() {
            log::error!("Failed to parse id_token from the response");
            self.send_auth_message(AuthMessage::UnavailableError);
            return false;
        };

        self.send_auth_message(AuthMessage::Success {
            id_token: response.id_token.clone(),
        });

        if let Err(err) = config_storage::write(
            AUTH_CONFIG_KEY,
            &OfflineAuthConfig {
                username: offline_auth_config.username.clone(),
                token_uri: offline_auth_config.token_uri.clone(),
                id_token: response.id_token,
                refresh_token: if let Some(refresh_token) = response.refresh_token {
                    refresh_token
                } else {
                    offline_auth_config.refresh_token.clone()
                },
            },
        ) {
            log::error!("Failed to save auth config: {:?}", err);
        }

        true
    }

    async fn exchange_auth_code(&self, request: &PendingOAuthRequest, code: String) -> bool {
        let Some(Ok(response)) = self
            .request::<AuthTokenResponse, serde_json::Value, _, _>(
                request.token_uri.clone(),
                RequestParams::UrlEncoded(&AuthTokenRequest {
                    client_id: request.client_id.clone(),
                    client_secret: request.client_secret.clone(),
                    code,
                    code_verifier: request.code_verifier.clone(),
                    grant_type: AuthorizationCodeGrantType::Grant,
                    redirect_uri: request.redirect_uri.to_string(),
                },
            )).await else
        {
            log::error!("Failed to exchange auth code");
            return false;
        };

        let Ok(token_data) = parse_jwt::<JwtAuthClaims>(&response.id_token) else {
            log::error!("Failed to parse id_token from the response");
            self.send_auth_message(AuthMessage::UnavailableError);
            return false;
        };

        self.send_auth_message(AuthMessage::Success {
            id_token: response.id_token.clone(),
        });

        let username = request
            .username
            .clone()
            .or(token_data.claims.email)
            .expect("Expected username in either request or id_token");

        if let Some(refresh_token) = response.refresh_token {
            if let Err(err) = config_storage::write(
                AUTH_CONFIG_KEY,
                &OfflineAuthConfig {
                    username,
                    token_uri: request.token_uri.to_string(),
                    id_token: response.id_token,
                    refresh_token,
                },
            ) {
                log::error!("Failed to save auth config: {:?}", err);
            }
        };

        true
    }

    async fn request<R: DeserializeOwned, E: DeserializeOwned, B: Serialize, U: IntoUrl>(
        &self,
        uri: U,
        params: RequestParams<&B>,
    ) -> Option<Result<R, E>> {
        let uri = uri.into_url().expect("Expected a valid url");
        let mut request = self.client.post(uri.clone());
        request = match params {
            RequestParams::Json(params) => request.json(params),
            RequestParams::UrlEncoded(params) => request
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(serde_urlencoded::to_string(params).unwrap()),
        };
        let result = request.send().await;

        let (data, status) = match result {
            Ok(result) => {
                let is_success = result.status();
                (result.bytes().await, is_success)
            }
            Err(err) => {
                log::error!("Failed to send a request: {:?}", err);
                self.send_auth_message(AuthMessage::UnavailableError);
                return None;
            }
        };

        #[cfg(debug_assertions)]
        if let Ok(data) = &data {
            log::debug!(
                "HTTP response: {}",
                String::from_utf8_lossy(data.as_slice())
            );
        }

        if status.is_success() {
            match data
                .ok()
                .and_then(|data| serde_json::from_slice::<R>(data.as_slice()).ok())
            {
                Some(response) => Some(Ok(response)),
                None => {
                    log::error!(
                        "Failed to parse a body response ({:?}, status: {})",
                        uri.to_string(),
                        status.as_u16()
                    );
                    self.send_auth_message(AuthMessage::UnavailableError);
                    None
                }
            }
        } else {
            match data
                .ok()
                .and_then(|data| serde_json::from_slice::<E>(data.as_slice()).ok())
            {
                Some(response) => Some(Err(response)),
                None => {
                    log::error!(
                        "Failed to parse a body response ({:?}, status: {})",
                        uri.to_string(),
                        status.as_u16()
                    );
                    self.send_auth_message(AuthMessage::UnavailableError);
                    None
                }
            }
        }
    }

    fn send_auth_message(&self, message: AuthMessage) {
        self.auth_message_tx
            .send(message)
            .expect("Failed to send an auth update");
    }
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
