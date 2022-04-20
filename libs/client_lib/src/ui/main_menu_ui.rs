use crate::{
    net::{
        auth::{AuthMessage, AuthRequest},
        MainMenuUiChannels, MatchmakerState, PersistenceMessagePayload, PersistenceRequest,
        ServerToConnect, TcpConnectionStatus,
    },
    ui::{
        widgets::list_menu::{button_panel, MenuListItem, PanelButton},
        without_item_spacing,
    },
    MuddleClientConfig, OfflineAuthConfig,
};
use bevy::{
    ecs::system::{Local, Res, ResMut, SystemParam},
    log,
    utils::{HashMap, Instant, Uuid},
    window::Windows,
};
use bevy_egui::{egui, egui::Widget, EguiContext};
use mr_messages_lib::{
    GameServerState, GetLevelResponse, GetLevelsRequest, GetLevelsUserFilter, InitLevel,
    LevelsListItem, LinkAccountLoginMethod, MatchmakerMessage, MatchmakerRequest, PaginationParams,
    Server,
};
use mr_shared_lib::net::{ConnectionState, ConnectionStatus, MessageId};
use std::{
    collections::BTreeMap,
    marker::PhantomData,
    ops::{Add, Mul, Sub},
};
use tokio::sync::mpsc::{error::TryRecvError, UnboundedSender};

const ERROR_COLOR: egui::Color32 = egui::Color32::RED;
const INVALID_EMAIL_ERROR: &str = "must be a valid email";
const SHORT_PASSWORD_ERROR: &str = "must be 8 characters or longer";
const EMPTY_DISPLAY_NAME_ERROR: &str = "must not be empty";
const LONG_DISPLAY_NAME_ERROR: &str = "must not be shorter than 255 characters";
const NON_ASCII_DISPLAY_NAME_ERROR: &str = "can contain only ASCII characters";

pub struct AuthUiState {
    screen: AuthUiScreen,
    email: InputField,
    password: InputField,
    display_name: InputField,
    #[cfg_attr(not(feature = "unstoppable_resolution"), allow(dead_code))]
    domain: String,
    error_message: String,
    handler_is_ready: bool,
    pending_request: bool,
    /// Is used only when [`AuthUiScreen::LinkAccount`] is active.
    available_login_methods: Vec<LinkAccountLoginMethod>,
    logged_in_as: Option<String>,
    linked_account: Option<String>,
}

impl Default for AuthUiState {
    fn default() -> Self {
        Self {
            screen: AuthUiScreen::SignIn,
            email: InputField {
                label: "Email",
                ..Default::default()
            },
            password: InputField {
                label: "Password",
                is_password: true,
                ..Default::default()
            },
            display_name: InputField {
                label: "Display name",
                ..Default::default()
            },
            domain: "".to_owned(),
            error_message: "".to_owned(),
            handler_is_ready: false,
            pending_request: false,
            available_login_methods: Vec::new(),
            logged_in_as: None,
            linked_account: None,
        }
    }
}

impl AuthUiState {
    pub fn validate(&mut self) {
        self.email.errors.clear();
        self.password.errors.clear();
        self.display_name.errors.clear();

        if !self.email.value.contains('@') {
            self.email.errors.push(INVALID_EMAIL_ERROR.to_owned());
        }

        if self.password.value.len() < 8 {
            self.password.errors.push(SHORT_PASSWORD_ERROR.to_owned());
        }

        let display_name = self.display_name.value.trim();
        if display_name.is_empty() {
            self.display_name
                .errors
                .push(EMPTY_DISPLAY_NAME_ERROR.to_owned());
        }
        if display_name.len() > 255 {
            self.display_name
                .errors
                .push(LONG_DISPLAY_NAME_ERROR.to_owned());
        }
        if !display_name.is_ascii() {
            self.display_name
                .errors
                .push(NON_ASCII_DISPLAY_NAME_ERROR.to_owned());
        }
    }

    pub fn respond_with_error(&mut self, msg: &str) {
        self.pending_request = false;
        self.error_message = msg.to_owned();
    }

    pub fn switch_screen(&mut self, new_screen: AuthUiScreen) {
        self.screen = new_screen;
        self.reset_form();
    }

    pub fn reset_form(&mut self) {
        self.email.reset();
        self.password.reset();
        self.display_name.reset();
        self.domain.clear();
        self.error_message.clear();
    }

    pub fn login_method_is_available(&self, method: &str) -> bool {
        // Temporarily disables any login method except for auth0.
        if method != "auth0" {
            return false;
        }

        if self.linked_account.is_none() {
            return true;
        }

        self.available_login_methods
            .iter()
            .any(|available_method| available_method.issuer.contains(method))
    }

    pub fn has_any_login_method_available(&self) -> bool {
        if self.linked_account.is_none() {
            return true;
        }

        self.login_method_is_available("google") || cfg!(feature = "unstoppable_resolution")
    }
}

const AUTH_INPUT_FIELD_WIDTH: f32 = 350.0;

#[derive(Default)]
pub struct InputField {
    label: &'static str,
    is_password: bool,
    value: String,
    errors: Vec<String>,
    was_focused: bool,
}

impl InputField {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.label(self.label);
        let resp = egui::widgets::TextEdit::singleline(&mut self.value)
            .desired_width(AUTH_INPUT_FIELD_WIDTH)
            .password(self.is_password)
            .ui(ui);
        self.was_focused = self.was_focused || resp.lost_focus();
        if self.was_focused && !self.errors.is_empty() {
            ui.scope(|ui| {
                ui.style_mut()
                    .visuals
                    .widgets
                    .noninteractive
                    .fg_stroke
                    .color = ERROR_COLOR;
                ui.style_mut().override_text_style = Some(egui::TextStyle::Button);
                for error in &self.errors {
                    ui.label(format!(" • {}", error));
                }
            });
        }
    }

    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn reset(&mut self) {
        self.value.clear();
        self.errors.clear();
        self.was_focused = false;
    }
}

pub enum AuthUiScreen {
    RefreshAuth,
    SignIn,
    SignUp,
    LinkAccount,
    SetDisplayName,
    GoogleOpenID,
    UnstoppableDomainsOpenID,
}

impl Default for AuthUiScreen {
    fn default() -> Self {
        Self::SignIn
    }
}

#[derive(Default)]
pub struct MatchmakerUiState {
    observed_empty_servers_at: Option<Instant>,
    servers: HashMap<String, Server>,
    // Contains a sever name, as we don't want selection to jump every time a server is
    // added/removed.
    selected_server: Option<String>,
    levels: BTreeMap<i64, LevelsListItem>,
    levels_list_filter: LevelsListFilter,
    selected_level: SelectedLevel,
    selected_level_data: Option<GetLevelResponse>,
    screen: MatchmakerUiScreen,
    request_id_counter: MessageId,
    current_request_id: Option<MessageId>,
    request_error_message: Option<String>,
}

#[derive(Clone, PartialEq)]
enum SelectedLevel {
    NewLevel(String),
    Existing(i64),
    None,
}

impl Default for SelectedLevel {
    fn default() -> Self {
        Self::None
    }
}

pub enum MatchmakerUiScreen {
    ServersList,
    CreateServer,
}

impl Default for MatchmakerUiScreen {
    fn default() -> Self {
        Self::ServersList
    }
}

#[derive(PartialEq)]
pub enum LevelsListFilter {
    All,
    Owned,
    Builder,
}

impl Default for LevelsListFilter {
    fn default() -> Self {
        Self::All
    }
}

#[derive(Default)]
pub struct MainMenuUiState {
    initialized: bool,
    screen: MainMenuUiScreen,
    auth: AuthUiState,
    matchmaker: MatchmakerUiState,
}

pub enum MainMenuUiScreen {
    Auth,
    Matchmaker,
}

impl Default for MainMenuUiScreen {
    fn default() -> Self {
        Self::Auth
    }
}

#[derive(SystemParam)]
pub struct Configs<'w, 's> {
    client_config: Res<'w, MuddleClientConfig>,
    offline_auth_config: Res<'w, OfflineAuthConfig>,
    #[system_param(ignore)]
    _marker: PhantomData<&'s ()>,
}

#[derive(SystemParam)]
pub struct UiContext<'w, 's> {
    egui_context: ResMut<'w, EguiContext>,
    windows: Res<'w, Windows>,
    #[system_param(ignore)]
    _marker: PhantomData<&'s ()>,
}

pub fn main_menu_ui(
    mut main_menu_ui_state: Local<MainMenuUiState>,
    mut ui_context: UiContext,
    matchmaker_state: Option<ResMut<MatchmakerState>>,
    configs: Configs,
    main_menu_ui_channels: Option<ResMut<MainMenuUiChannels>>,
    mut server_to_connect: ResMut<Option<ServerToConnect>>,
    connection_state: Res<ConnectionState>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();

    // If matchmaker address is not configured (which means that the state and the channels aren't
    // initialized either), we don't want to render this menu.
    let (mut matchmaker_state, mut main_menu_ui_channels) =
        match matchmaker_state.zip(main_menu_ui_channels) {
            Some(resources) => resources,
            None => return,
        };

    if !main_menu_ui_state.initialized {
        if configs.offline_auth_config.exists() {
            main_menu_ui_state.auth.screen = AuthUiScreen::RefreshAuth;
            main_menu_ui_state.auth.logged_in_as =
                Some(configs.offline_auth_config.username.clone());
        } else {
            main_menu_ui_state.auth.screen = AuthUiScreen::SignIn;
        }
        main_menu_ui_state.initialized = true;
    }

    process_auth_messages(
        &mut main_menu_ui_state,
        &mut matchmaker_state,
        &mut main_menu_ui_channels,
    );
    process_matchmaker_messages(
        &mut main_menu_ui_state,
        &configs,
        &mut main_menu_ui_channels,
    );
    process_persistence_messages(&mut main_menu_ui_state, &mut main_menu_ui_channels);

    if !matches!(matchmaker_state.status, TcpConnectionStatus::Connected) {
        main_menu_ui_state.matchmaker.servers.clear();
    }

    if !matches!(connection_state.status(), ConnectionStatus::Uninitialized)
        || server_to_connect.is_some()
    {
        return;
    }

    if main_menu_ui_state.matchmaker.servers.is_empty()
        && matches!(matchmaker_state.status, TcpConnectionStatus::Connected)
        && main_menu_ui_state
            .matchmaker
            .observed_empty_servers_at
            .is_none()
    {
        main_menu_ui_state.matchmaker.observed_empty_servers_at = Some(Instant::now());
    }

    let screen_height = ui_context.windows.get_primary().unwrap().height();

    let window_width = 400.0;
    let window_height = 600.0;
    let offset_y = (200.0 - (200.0 + window_height - screen_height).max(0.0)).max(0.0);

    let ctx = ui_context.egui_context.ctx_mut();
    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(egui::Color32::from_black_alpha(200)))
        .show(ctx, |ui| {
            ui.spacing_mut().window_margin =
                egui::style::Margin::same(ui.visuals().window_stroke().width);
            egui::Window::new("Server browser")
                .title_bar(false)
                .collapsible(false)
                .resizable(false)
                .frame(egui::Frame::window(ui.style()))
                .anchor(egui::Align2::CENTER_TOP, egui::Vec2::new(0.0, offset_y))
                .fixed_size(egui::Vec2::new(window_width, window_height))
                .show(ui.ctx(), |ui| {
                    let MainMenuUiState {
                        initialized: _,
                        screen: ref mut main_menu_ui_screen,
                        auth: auth_ui_state,
                        matchmaker: matchmaker_ui_state,
                    } = &mut *main_menu_ui_state;

                    match matchmaker_state.status {
                        TcpConnectionStatus::Connected => {
                            // Header.
                            if matchmaker_ui_state.servers.is_empty() {
                                let progress = Instant::now()
                                    .duration_since(
                                        matchmaker_ui_state.observed_empty_servers_at.unwrap(),
                                    )
                                    .as_secs_f32()
                                    / 120.0;
                                status_bar(
                                    ui,
                                    "Spinning up a game server (might take a couple of minutes)...",
                                    progress.min(0.95),
                                );
                            } else {
                                status_bar(ui, "Matchmaker server: connected", 1.0);
                            }
                        }
                        TcpConnectionStatus::Connecting | TcpConnectionStatus::Disconnected => {
                            status_bar(ui, "Connecting to the matchmaker...", 0.0);
                        }
                    }

                    match main_menu_ui_screen {
                        MainMenuUiScreen::Auth => {
                            egui::containers::Frame::none()
                                .margin(egui::style::Margin::symmetric(25.0, 15.0))
                                .show(ui, |ui| {
                                    let confirm = authentication_screen(
                                        ui,
                                        &mut main_menu_ui_channels.auth_request_tx,
                                        auth_ui_state,
                                        &configs.offline_auth_config,
                                    );
                                    if confirm {
                                        *main_menu_ui_screen = MainMenuUiScreen::Matchmaker;
                                    }
                                });
                        }
                        MainMenuUiScreen::Matchmaker => {
                            matchmaker_screen(
                                ui,
                                &matchmaker_state,
                                matchmaker_ui_state,
                                &mut server_to_connect,
                                &mut main_menu_ui_channels,
                            );
                        }
                    }
                });
        });
}

fn process_auth_messages(
    main_menu_ui_state: &mut MainMenuUiState,
    matchmaker_state: &mut MatchmakerState,
    main_menu_ui_channels: &mut MainMenuUiChannels,
) {
    loop {
        match main_menu_ui_channels.auth_message_rx.try_recv() {
            Ok(AuthMessage::AuthHandlerIsReady) => {
                main_menu_ui_state.auth.handler_is_ready = true;
            }
            Ok(AuthMessage::Success { id_token, user_id }) => {
                log::debug!("Successful auth");
                main_menu_ui_state.screen = MainMenuUiScreen::Matchmaker;
                main_menu_ui_state.auth.pending_request = false;
                main_menu_ui_state.auth.reset_form();
                matchmaker_state.id_token = Some(id_token);
                matchmaker_state.user_id = Some(user_id);
            }
            #[cfg(feature = "unstoppable_resolution")]
            Ok(AuthMessage::InvalidDomainError) => {
                main_menu_ui_state
                    .auth
                    .respond_with_error("The requested domain isn't registered");
            }
            Ok(AuthMessage::DisplayNameTakenError) => {
                log::debug!("Display name is already taken");
                main_menu_ui_state
                    .auth
                    .respond_with_error("Display name is already taken");
            }
            Ok(AuthMessage::UnavailableError) => {
                log::debug!("Authentication unavailable");
                main_menu_ui_state
                    .auth
                    .respond_with_error("The service is unavailable. Please try again later");
            }
            Ok(AuthMessage::WrongPasswordError) => {
                log::debug!("Wrong password");
                main_menu_ui_state
                    .auth
                    .respond_with_error("Incorrect username or password");
            }
            Ok(AuthMessage::SignUpFailedError) => {
                log::debug!("Bad Sign Up");
                main_menu_ui_state
                    .auth
                    .respond_with_error("Signing Up failed (email might be taken)");
            }
            Ok(AuthMessage::InvalidOrExpiredAuthError)
                if main_menu_ui_state.auth.linked_account.is_some() =>
            {
                log::debug!("Failed to link accounts");
                main_menu_ui_state.screen = MainMenuUiScreen::Auth;
                main_menu_ui_state.auth.screen = AuthUiScreen::LinkAccount;
                main_menu_ui_state.auth.reset_form();
                main_menu_ui_state
                    .auth
                    .respond_with_error("Failed to link accounts (email mismatch)");
            }
            Ok(AuthMessage::InvalidOrExpiredAuthError) => {
                log::debug!("Invalid or expired auth");
                main_menu_ui_state.screen = MainMenuUiScreen::Auth;
                main_menu_ui_state.auth.screen = AuthUiScreen::SignIn;
                main_menu_ui_state.auth.reset_form();
                main_menu_ui_state
                    .auth
                    .respond_with_error("Invalid or expired session");
            }
            Ok(AuthMessage::LinkAccount {
                email,
                login_methods,
            }) => {
                main_menu_ui_state.screen = MainMenuUiScreen::Auth;
                main_menu_ui_state.auth.screen = AuthUiScreen::LinkAccount;
                main_menu_ui_state.auth.available_login_methods = login_methods;
                main_menu_ui_state.auth.pending_request = false;
                main_menu_ui_state.auth.reset_form();
                main_menu_ui_state.auth.linked_account = Some(email);
            }
            Ok(AuthMessage::SetDisplayName) => {
                main_menu_ui_state.screen = MainMenuUiScreen::Auth;
                main_menu_ui_state.auth.screen = AuthUiScreen::SetDisplayName;
                main_menu_ui_state.auth.pending_request = false;
                main_menu_ui_state.auth.reset_form();
            }
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                panic!("Failed to read from a channel (auth messages)")
            }
        }
    }
}

fn process_matchmaker_messages(
    main_menu_ui_state: &mut MainMenuUiState,
    configs: &Configs,
    main_menu_ui_channels: &mut MainMenuUiChannels,
) {
    loop {
        match main_menu_ui_channels.matchmaker_message_rx.try_recv() {
            Ok(MatchmakerMessage::Init {
                servers: mut init_list,
            }) => {
                if let Some(server_addr) = configs.client_config.server_addr {
                    init_list.push(Server {
                        name: "localhost".to_string(),
                        state: GameServerState::Allocated,
                        addr: server_addr,
                        player_capacity: 0,
                        player_count: 0,
                        request_id: Default::default(),
                    })
                }

                log::debug!("Initialize servers list: {:?}", init_list);
                main_menu_ui_state.matchmaker.servers = init_list
                    .into_iter()
                    .map(|server| (server.name.clone(), server))
                    .collect();
            }
            Ok(MatchmakerMessage::ServerUpdated(server)) => {
                log::debug!("Server updated: {:?}", server);
                main_menu_ui_state
                    .matchmaker
                    .servers
                    .insert(server.name.clone(), server);
            }
            Ok(MatchmakerMessage::ServerRemoved(server_name)) => {
                log::debug!("Server removed: {:?}", server_name);
                main_menu_ui_state.matchmaker.servers.remove(&server_name);
            }
            Ok(MatchmakerMessage::InvalidJwt(request_id)) => {
                log::debug!("InvalidJwt response: {:?}", request_id);
                main_menu_ui_state.screen = MainMenuUiScreen::Auth;
                main_menu_ui_state.auth.screen = AuthUiScreen::SignIn;
            }
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                panic!("Failed to read from a channel (matchmaker messages)")
            }
        }
    }
}

fn process_persistence_messages(
    main_menu_ui_state: &mut MainMenuUiState,
    main_menu_ui_channels: &mut MainMenuUiChannels,
) {
    loop {
        let payload = match main_menu_ui_channels.persistence_message_rx.try_recv() {
            Ok(message) => {
                if Some(message.request_id) != main_menu_ui_state.matchmaker.current_request_id {
                    log::debug!(
                        "Skipping response (message request id: {}, current: {:?})",
                        message.request_id,
                        main_menu_ui_state.matchmaker.current_request_id
                    );
                    continue;
                }
                main_menu_ui_state.matchmaker.current_request_id = None;
                message.payload
            }
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                panic!("Failed to read from a channel (persistence messages)")
            }
        };
        match payload {
            PersistenceMessagePayload::GetLevelsResponse(levels) => {
                log::debug!("New levels list: {levels:?}");
                main_menu_ui_state.matchmaker.levels =
                    levels.into_iter().map(|level| (level.id, level)).collect();
            }
            PersistenceMessagePayload::GetLevelResponse(response) => {
                log::debug!("Selected level details: {response:?}");
                main_menu_ui_state.matchmaker.selected_level_data = Some(response);
            }
            PersistenceMessagePayload::RequestFailed(error) => {
                log::warn!("Get level request failed: {error}");
                main_menu_ui_state.matchmaker.request_error_message = Some(error);
            }
        }
    }
}

fn authentication_screen(
    ui: &mut egui::Ui,
    auth_request_tx: &mut UnboundedSender<AuthRequest>,
    auth_ui_state: &mut AuthUiState,
    offline_auth_config: &OfflineAuthConfig,
) -> bool {
    ui.style_mut().spacing.item_spacing = egui::Vec2::new(10.0, 5.0);
    let mut confirm_auth = false;
    let mut new_screen = None;

    auth_ui_state.validate();

    match auth_ui_state.screen {
        AuthUiScreen::RefreshAuth => {
            ui.with_layout(
                egui::Layout::top_down_justified(egui::Align::Center),
                |ui| {
                    ui.label("You've been logged in as");
                    ui.style_mut().override_text_style = Some(egui::TextStyle::Heading);
                    ui.label(auth_ui_state.logged_in_as.as_ref().unwrap());
                    ui.style_mut().override_text_style = None;

                    ui.add_space(10.0);

                    ui.set_enabled(!auth_ui_state.pending_request);
                    if ui.button("Continue").clicked() {
                        auth_ui_state.pending_request = true;
                        auth_request_tx
                            .send(AuthRequest::RefreshAuth(offline_auth_config.clone()))
                            .expect("Failed to write to a channel (auth request)");
                    }
                    if ui.button("Use different account").clicked() {
                        new_screen = Some(AuthUiScreen::SignIn);
                        auth_ui_state.logged_in_as = None;
                    }

                    ui.style_mut()
                        .visuals
                        .widgets
                        .noninteractive
                        .fg_stroke
                        .color = ERROR_COLOR;
                    ui.label(&auth_ui_state.error_message);
                },
            );
        }
        AuthUiScreen::LinkAccount if !auth_ui_state.has_any_login_method_available() => {
            if ui.button("Use different account").clicked() {
                new_screen = Some(AuthUiScreen::SignIn);
                auth_request_tx
                    .send(AuthRequest::UseDifferentAccount)
                    .expect("Failed to write to a channel (auth request)");
            }

            ui.add_space(5.0);

            ui.label(format!(
                "Account with an email {} already exists. The login method used for this account is not available.",
                auth_ui_state
                    .linked_account
                    .as_ref()
                    .expect("Expected an email when linking accounts")
            ));
        }
        AuthUiScreen::SignIn | AuthUiScreen::SignUp | AuthUiScreen::LinkAccount => {
            ui.set_enabled(!auth_ui_state.pending_request);

            if matches!(auth_ui_state.screen, AuthUiScreen::LinkAccount) {
                if ui.button("Use different account").clicked() {
                    new_screen = Some(AuthUiScreen::SignIn);
                    auth_request_tx
                        .send(AuthRequest::UseDifferentAccount)
                        .expect("Failed to write to a channel (auth request)");
                }

                ui.add_space(5.0);

                ui.label(format!(
                    "Account with an email {} already exists. Please sign in to link the accounts.",
                    auth_ui_state
                        .linked_account
                        .as_ref()
                        .expect("Expected an email when linking accounts")
                ));
                ui.add_space(5.0);
            }

            if auth_ui_state.login_method_is_available("auth0") {
                if matches!(auth_ui_state.screen, AuthUiScreen::LinkAccount) {
                    let email = auth_ui_state
                        .linked_account
                        .as_mut()
                        .expect("Expected an email when linking accounts");
                    egui::widgets::TextEdit::singleline(email)
                        .desired_width(AUTH_INPUT_FIELD_WIDTH)
                        .interactive(false)
                        .ui(ui);
                } else {
                    auth_ui_state.email.ui(ui);
                }
                auth_ui_state.password.ui(ui);

                ui.add_space(5.0);

                let is_sign_up = matches!(auth_ui_state.screen, AuthUiScreen::SignUp);
                let is_valid = (matches!(auth_ui_state.screen, AuthUiScreen::LinkAccount)
                    || auth_ui_state.email.is_valid())
                    && auth_ui_state.password.is_valid();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !auth_ui_state.pending_request && is_valid,
                            egui::widgets::Button::new(if is_sign_up {
                                "Sign Up"
                            } else {
                                "Sign In"
                            }),
                        )
                        .clicked()
                    {
                        auth_ui_state.pending_request = true;
                        auth_request_tx
                            .send(AuthRequest::Password {
                                username: auth_ui_state
                                    .linked_account
                                    .clone()
                                    .unwrap_or_else(|| auth_ui_state.email.value.clone()),
                                password: auth_ui_state.password.value.clone(),
                                is_sign_up,
                            })
                            .expect("Failed to write to a channel (auth request)");
                        auth_ui_state.password.reset();
                    }

                    ui.style_mut()
                        .visuals
                        .widgets
                        .noninteractive
                        .fg_stroke
                        .color = ERROR_COLOR;
                    ui.label(auth_ui_state.error_message.clone());
                });

                ui.add_space(5.0);
            }

            let google_is_available = auth_ui_state.login_method_is_available("google");
            // TODO: such kind of method check won't work for custom OIDC providers.
            let ud_is_available = cfg!(feature = "unstoppable_resolution")
                && auth_ui_state.login_method_is_available("unstoppable");
            if google_is_available || ud_is_available {
                ui.separator();
                ui.label("Continue with an auth provider");

                ui.horizontal(|ui| {
                    ui.set_enabled(
                        !auth_ui_state.pending_request && auth_ui_state.handler_is_ready,
                    );

                    if google_is_available && ui.button("Google").clicked() {
                        auth_ui_state.pending_request = true;
                        new_screen = Some(AuthUiScreen::GoogleOpenID);
                        auth_request_tx
                            .send(AuthRequest::RequestGoogleAuth)
                            .expect("Failed to write to a channel (auth request)");
                    }

                    if ud_is_available && ui.button("Unstoppable Domains").clicked() {
                        new_screen = Some(AuthUiScreen::UnstoppableDomainsOpenID);
                    }
                });
            }

            match auth_ui_state.screen {
                AuthUiScreen::SignIn => {
                    ui.separator();
                    ui.label("Don't have an account?");
                    ui.horizontal(|ui| {
                        ui.style_mut().spacing.item_spacing = egui::Vec2::new(8.0, 5.0);
                        if ui.button("Sign Up").clicked() {
                            new_screen = Some(AuthUiScreen::SignUp);
                        }
                        ui.label("or");
                        if ui.button("Play anonymously").clicked() {
                            confirm_auth = true;
                        }
                    });
                }
                AuthUiScreen::SignUp => {
                    ui.separator();
                    ui.label("Already have an account?");
                    ui.horizontal(|ui| {
                        ui.style_mut().spacing.item_spacing = egui::Vec2::new(8.0, 5.0);
                        if ui.button("Sign In").clicked() {
                            new_screen = Some(AuthUiScreen::SignIn);
                        }
                        ui.label("or");
                        if ui.button("Play anonymously").clicked() {
                            confirm_auth = true;
                        }
                    });
                }
                AuthUiScreen::LinkAccount => {}
                _ => unreachable!(),
            }
        }
        AuthUiScreen::SetDisplayName => {
            ui.set_enabled(!auth_ui_state.pending_request);
            if ui.button("Use different account").clicked() {
                new_screen = Some(AuthUiScreen::SignIn);
                auth_request_tx
                    .send(AuthRequest::UseDifferentAccount)
                    .expect("Failed to write to a channel (auth request)");
            }
            ui.add_space(5.0);

            ui.label("Marvelous! You've created a brand new account. The last step is picking a display name, to show off your awesome profile.");
            ui.add_space(5.0);

            auth_ui_state.display_name.ui(ui);
            ui.add_space(5.0);

            ui.with_layout(
                egui::Layout::top_down_justified(egui::Align::Center),
                |ui| {
                    if !auth_ui_state.display_name.is_valid() || auth_ui_state.pending_request {
                        ui.set_enabled(false);
                    }
                    if ui.button("Continue").clicked() {
                        auth_ui_state.pending_request = true;
                        auth_request_tx
                            .send(AuthRequest::SetDisplayName(
                                auth_ui_state.display_name.value.clone(),
                            ))
                            .expect("Failed to write to a channel (auth request)");
                    }

                    if !auth_ui_state.error_message.is_empty() {
                        ui.style_mut()
                            .visuals
                            .widgets
                            .noninteractive
                            .fg_stroke
                            .color = ERROR_COLOR;
                        ui.label(&auth_ui_state.error_message);
                    }
                },
            );
        }
        AuthUiScreen::GoogleOpenID | AuthUiScreen::UnstoppableDomainsOpenID => {
            ui.horizontal(|ui| {
                if ui.button("Back").clicked() {
                    if auth_ui_state.linked_account.is_some() {
                        new_screen = Some(AuthUiScreen::LinkAccount);
                    } else {
                        new_screen = Some(AuthUiScreen::SignIn);
                    }
                    auth_ui_state.pending_request = false;
                    auth_request_tx
                        .send(AuthRequest::CancelOpenIDRequest)
                        .expect("Failed to write to a channel (auth request)");
                }
                ui.style_mut()
                    .visuals
                    .widgets
                    .noninteractive
                    .fg_stroke
                    .color = ERROR_COLOR;
                ui.label(&auth_ui_state.error_message);
            });
            ui.add_space(20.0);

            ui.set_enabled(!auth_ui_state.pending_request);

            #[cfg(feature = "unstoppable_resolution")]
            if !auth_ui_state.pending_request
                && matches!(auth_ui_state.screen, AuthUiScreen::UnstoppableDomainsOpenID)
            {
                ui.label("Domain name");

                ui.with_layout(
                    egui::Layout::top_down_justified(egui::Align::Center),
                    |ui| {
                        ui.text_edit_singleline(&mut auth_ui_state.domain);
                        ui.add_space(5.0);
                        if ui.button("Continue").clicked() {
                            auth_ui_state.pending_request = true;
                            auth_request_tx
                                .send(AuthRequest::RequestUnstoppableDomainsAuth {
                                    username: auth_ui_state.domain.clone(),
                                })
                                .expect("Failed to write to a channel (auth request)");
                        }
                    },
                );
            }
            if let Some(logged_in_as) = auth_ui_state.logged_in_as.as_ref() {
                ui.with_layout(
                    egui::Layout::top_down_justified(egui::Align::Center),
                    |ui| {
                        ui.label(format!("Logged in as {}", logged_in_as));
                        confirm_auth = ui.button("Continue").clicked();
                    },
                );
            } else if auth_ui_state.pending_request {
                ui.with_layout(
                    egui::Layout::top_down_justified(egui::Align::Center),
                    |ui| {
                        ui.label("Please complete the Sign In");
                        let _ = ui.button("Continue");
                    },
                );
            }
        }
    }

    ui.add_space(5.0);
    if let Some(new_screen) = new_screen {
        auth_ui_state.switch_screen(new_screen);
    }
    confirm_auth
}

fn matchmaker_screen(
    ui: &mut egui::Ui,
    matchmaker_state: &MatchmakerState,
    matchmaker_ui_state: &mut MatchmakerUiState,
    server_to_connect: &mut Option<ServerToConnect>,
    main_menu_ui_channels: &mut MainMenuUiChannels,
) {
    match matchmaker_ui_state.screen {
        MatchmakerUiScreen::ServersList => matchmaker_servers_list_screen(
            ui,
            matchmaker_state,
            server_to_connect,
            matchmaker_ui_state,
            main_menu_ui_channels.persistence_request_tx.clone(),
        ),
        MatchmakerUiScreen::CreateServer => matchmaker_create_server_screen(
            ui,
            matchmaker_state,
            matchmaker_ui_state,
            main_menu_ui_channels.matchmaker_request_tx.clone(),
            main_menu_ui_channels.persistence_request_tx.clone(),
        ),
    }
}

fn matchmaker_servers_list_screen(
    ui: &mut egui::Ui,
    matchmaker_state: &MatchmakerState,
    server_to_connect: &mut Option<ServerToConnect>,
    matchmaker_ui_state: &mut MatchmakerUiState,
    persistence_requests_tx: UnboundedSender<PersistenceRequest>,
) {
    if let TcpConnectionStatus::Connected = matchmaker_state.status {
        // Server list.
        let mut sorted_servers = matchmaker_ui_state.servers.values().collect::<Vec<_>>();
        sorted_servers.sort_by(|a, b| a.name.cmp(&b.name));

        without_item_spacing(ui, |ui| {
            egui::containers::ScrollArea::vertical()
                .max_height(500.0)
                .show(ui, |ui| {
                    let response = MenuListItem::new("Create a server")
                        .secondary_widget(|ui| {
                            ui.label("Create a server to let other players join");
                        })
                        .image_widget(plus_image)
                        .show(ui);
                    if response.item.clicked() {
                        matchmaker_ui_state.selected_server = None;
                        matchmaker_ui_state.screen = MatchmakerUiScreen::CreateServer;
                        let request_id = matchmaker_ui_state.request_id_counter.increment();
                        matchmaker_ui_state.current_request_id = Some(request_id);
                        persistence_requests_tx
                            .send(PersistenceRequest::GetLevels {
                                request_id,
                                body: GetLevelsRequest {
                                    user_filter: None,
                                    pagination: PaginationParams {
                                        offset: 0,
                                        limit: 20,
                                    },
                                },
                            })
                            .expect("Failed to write to a channel (persistence request)");
                    }

                    server_list(
                        ui,
                        &sorted_servers,
                        &mut matchmaker_ui_state.selected_server,
                    );
                });
        });
    }

    let is_selected = matchmaker_ui_state
        .selected_server
        .as_ref()
        .map_or(false, |selected| {
            matchmaker_ui_state.servers.contains_key(selected)
        });

    let [play_response] = button_panel(
        ui,
        100.0,
        [PanelButton::new(egui::widgets::Button::new("Play"))
            .enabled(is_selected)
            .on_disabled_hover_text("Select a server from the list or Create a new one")],
    );
    if play_response.clicked() {
        *server_to_connect = Some(ServerToConnect(
            matchmaker_ui_state.servers[matchmaker_ui_state.selected_server.as_ref().unwrap()]
                .clone(),
        ));
    }
}

fn matchmaker_create_server_screen(
    ui: &mut egui::Ui,
    matchmaker_state: &MatchmakerState,
    matchmaker_ui_state: &mut MatchmakerUiState,
    matchmaker_requests_tx: UnboundedSender<MatchmakerRequest>,
    persistence_requests_tx: UnboundedSender<PersistenceRequest>,
) {
    ui.set_enabled(matchmaker_ui_state.current_request_id.is_none());

    let padding = egui::Vec2::new(10.0, 5.0);
    let panel_height = 30.0;
    let mut panel_ui = ui.child_ui(
        egui::Rect::from_min_size(
            ui.min_rect().left_bottom() + padding,
            egui::Vec2::new(ui.available_width(), panel_height) - padding,
        ),
        egui::Layout::left_to_right(),
    );
    if panel_ui
        .selectable_value(
            &mut matchmaker_ui_state.levels_list_filter,
            LevelsListFilter::All,
            "All",
        )
        .clicked()
    {
        matchmaker_ui_state.selected_level = SelectedLevel::None;
        let request_id = matchmaker_ui_state.request_id_counter.increment();
        matchmaker_ui_state.current_request_id = Some(request_id);
        persistence_requests_tx
            .send(PersistenceRequest::GetLevels {
                request_id,
                body: GetLevelsRequest {
                    user_filter: None,
                    pagination: PaginationParams {
                        offset: 0,
                        limit: 20,
                    },
                },
            })
            .expect("Failed to write to a channel (persistence request)");
    }
    panel_ui.set_enabled(matchmaker_state.user_id.is_some());
    if panel_ui
        .selectable_value(
            &mut matchmaker_ui_state.levels_list_filter,
            LevelsListFilter::Owned,
            "Owned",
        )
        .clicked()
    {
        matchmaker_ui_state.selected_level = SelectedLevel::None;
        let request_id = matchmaker_ui_state.request_id_counter.increment();
        matchmaker_ui_state.current_request_id = Some(request_id);
        persistence_requests_tx
            .send(PersistenceRequest::GetLevels {
                request_id,
                body: GetLevelsRequest {
                    user_filter: Some(GetLevelsUserFilter::AuthorId(
                        matchmaker_state.user_id.unwrap(),
                    )),
                    pagination: PaginationParams {
                        offset: 0,
                        limit: 20,
                    },
                },
            })
            .expect("Failed to write to a channel (persistence request)");
    }
    if panel_ui
        .selectable_value(
            &mut matchmaker_ui_state.levels_list_filter,
            LevelsListFilter::Builder,
            "Builder",
        )
        .clicked()
    {
        matchmaker_ui_state.selected_level = SelectedLevel::None;
        let request_id = matchmaker_ui_state.request_id_counter.increment();
        matchmaker_ui_state.current_request_id = Some(request_id);
        persistence_requests_tx
            .send(PersistenceRequest::GetLevels {
                request_id,
                body: GetLevelsRequest {
                    user_filter: Some(GetLevelsUserFilter::BuilderId(
                        matchmaker_state.user_id.unwrap(),
                    )),
                    pagination: PaginationParams {
                        offset: 0,
                        limit: 20,
                    },
                },
            })
            .expect("Failed to write to a channel (persistence request)");
    }

    without_item_spacing(ui, |ui| {
        ui.allocate_rect(panel_ui.min_rect().expand2(padding), egui::Sense::hover());
        ui.separator();
    });

    let response = MenuListItem::new("New level")
        .selected(matches!(
            matchmaker_ui_state.selected_level,
            SelectedLevel::NewLevel(_)
        ))
        .image_widget(plus_image)
        .secondary_widget(|ui| {
            if let SelectedLevel::NewLevel(ref mut level_title) = matchmaker_ui_state.selected_level
            {
                ui.style_mut().visuals.widgets.inactive.bg_stroke =
                    ui.style_mut().visuals.window_stroke();
                ui.text_edit_singleline(level_title);
            }
        })
        .show(ui);
    if response.item.clicked()
        && !matches!(
            matchmaker_ui_state.selected_level,
            SelectedLevel::NewLevel(_)
        )
    {
        matchmaker_ui_state.selected_level = SelectedLevel::NewLevel("My new level".to_owned());
    }

    for level in matchmaker_ui_state.levels.values() {
        let selected = matchmaker_ui_state.selected_level == SelectedLevel::Existing(level.id);
        let response = MenuListItem::new(&level.title)
            .with_id(level.id)
            .selected(selected)
            .secondary_widget(|ui| {
                ui.label(format!(
                    "Author: {}",
                    level.user_name.as_deref().unwrap_or_default()
                ));
            })
            .collapsing_widget(|ui| {
                let builders = matchmaker_ui_state
                    .selected_level_data
                    .as_ref()
                    .map(|level| level.level_permissions.clone())
                    .unwrap_or_default();
                if !builders.is_empty() {
                    let builders = builders
                        .into_iter()
                        .map(|user| user.user_name.unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join(", ");
                    ui.label(format!("Builders: {builders}"));
                }
                ui.label(format!(
                    "Created at: {}",
                    level.created_at.format("%Y-%m-%d %H:%M:%S")
                ));
                ui.label(format!(
                    "Updated at: {}",
                    level.updated_at.format("%Y-%m-%d %H:%M:%S")
                ));
            })
            .show(ui);

        if response.item.clicked()
            && matchmaker_ui_state.selected_level != SelectedLevel::Existing(level.id)
        {
            matchmaker_ui_state.selected_level = SelectedLevel::Existing(level.id);
            matchmaker_ui_state.selected_level_data = None;
            let request_id = matchmaker_ui_state.request_id_counter.increment();
            matchmaker_ui_state.current_request_id = Some(request_id);
            persistence_requests_tx
                .send(PersistenceRequest::GetLevel {
                    request_id,
                    level_id: level.id,
                })
                .expect("Failed to write to a channel (persistence request)");
        }
    }

    let host_enabled = match &matchmaker_ui_state.selected_level {
        SelectedLevel::NewLevel(title) if !title.is_empty() => true,
        SelectedLevel::Existing(_) => true,
        _ => false,
    };
    let fork_enabled = match &matchmaker_ui_state.selected_level {
        SelectedLevel::NewLevel(_) => false,
        SelectedLevel::Existing(_) => true,
        SelectedLevel::None => false,
    };

    let [back_response, host_response, fork_response] = button_panel(
        ui,
        70.0,
        [
            PanelButton::new(egui::Button::new("Back")),
            PanelButton::new(egui::Button::new("Create"))
                .enabled(host_enabled)
                .on_disabled_hover_text("Select a level to create a server"),
            PanelButton::new(egui::Button::new("Fork"))
                .enabled(fork_enabled)
                .on_hover_text("Clone and host the selected level")
                .on_disabled_hover_text("Clone and host the selected level"),
        ],
    );

    if back_response.clicked() {
        matchmaker_ui_state.screen = MatchmakerUiScreen::ServersList;
        matchmaker_ui_state.selected_level = SelectedLevel::None;
    }

    if host_response.clicked() {
        let init_level = match &matchmaker_ui_state.selected_level {
            SelectedLevel::NewLevel(level_title) => InitLevel::Create {
                title: level_title.clone(),
                parent_id: None,
            },
            SelectedLevel::Existing(level_id) => InitLevel::Existing(*level_id),
            SelectedLevel::None => unreachable!(),
        };
        matchmaker_requests_tx
            .send(MatchmakerRequest::CreateServer {
                init_level,
                request_id: Uuid::new_v4(),
                id_token: matchmaker_state.id_token.clone(),
            })
            .expect("Failed to write to a channel (matchmaker request)");
    }

    if fork_response.clicked() {
        let init_level = match &matchmaker_ui_state.selected_level {
            SelectedLevel::Existing(level_id) => InitLevel::Create {
                title: matchmaker_ui_state.levels[level_id].title.clone(),
                parent_id: Some(*level_id),
            },
            _ => unreachable!(),
        };
        matchmaker_requests_tx
            .send(MatchmakerRequest::CreateServer {
                init_level,
                request_id: Uuid::new_v4(),
                id_token: matchmaker_state.id_token.clone(),
            })
            .expect("Failed to write to a channel (matchmaker request)");
    }
}

fn server_list(ui: &mut egui::Ui, servers: &[&Server], selected: &mut Option<String>) {
    for server in servers {
        let is_selected = selected
            .as_ref()
            .map_or(false, |selected| &server.name == selected);
        let response = MenuListItem::new(&server.name)
            .secondary_widget(|ui| {
                ui.label(format!(
                    "Players: {}/{}",
                    server.player_count, server.player_capacity
                ));
            })
            .selected(is_selected)
            .show(ui);
        if response.item.clicked() {
            *selected = Some(server.name.clone());
        }
    }
}

fn status_bar(ui: &mut egui::Ui, label: impl ToString, progress: f32) {
    let desired_width = ui.max_rect().width();
    let height = ui.spacing().interact_size.y;
    let (outer_rect, _response) =
        ui.allocate_exact_size(egui::Vec2::new(desired_width, height), egui::Sense::hover());

    if progress > 0.0 {
        let rounding = ui.style().visuals.window_rounding;
        let size = outer_rect
            .size()
            .mul(egui::Vec2::new(progress, 1.0))
            .max(egui::Vec2::new(rounding.ne * 2.0, outer_rect.height()));
        let fill = egui::Color32::from_rgb(23, 98, 3);
        let stroke = egui::Stroke::new(0.0, egui::Color32::TRANSPARENT);

        ui.painter().add(egui::Shape::Rect(egui::epaint::RectShape {
            rect: egui::Rect::from_min_size(outer_rect.min, size),
            rounding,
            fill,
            stroke,
        }));
        ui.painter().add(egui::Shape::Rect(egui::epaint::RectShape {
            rect: egui::Rect::from_min_size(
                outer_rect.min.add(egui::Vec2::new(0.0, size.y / 2.0)),
                size.mul(egui::Vec2::new(1.0, 0.5)),
            ),
            rounding: egui::Rounding::same(0.0),
            fill,
            stroke,
        }));
        if size.x < outer_rect.size().sub(egui::Vec2::new(rounding.ne, 0.0)).x {
            ui.painter().add(egui::Shape::Rect(egui::epaint::RectShape {
                rect: egui::Rect::from_min_size(
                    outer_rect.min.add(egui::Vec2::new(rounding.ne, 0.0)),
                    size.sub(egui::Vec2::new(rounding.ne, 0.0)),
                ),
                rounding: egui::Rounding::same(0.0),
                fill,
                stroke,
            }));
        }
    }

    ui.painter().text(
        outer_rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::TextStyle::Body.resolve(ui.style()),
        ui.visuals().text_color(),
    );
}

fn plus_image(ui: &mut egui::Ui) {
    let horizontal =
        egui::Rect::from_center_size(ui.max_rect().center(), egui::Vec2::new(16.0, 4.0));
    let vertical = egui::Rect::from_center_size(ui.max_rect().center(), egui::Vec2::new(4.0, 16.0));
    ui.painter()
        .rect_filled(horizontal, 0.0, ui.visuals().widgets.inactive.bg_fill);
    ui.painter()
        .rect_filled(vertical, 0.0, ui.visuals().widgets.inactive.bg_fill);
}
