use crate::{
    net::{
        auth::{AuthMessage, AuthRequest},
        MainMenuUiChannels, MatchmakerState, PersistenceMessagePayload, PersistenceRequest,
        ServerToConnect, TcpConnectionStatus,
    },
    ui::{
        widgets::list_menu::{button_panel, MenuListItem, MenuListItemResponse, PanelButton},
        without_item_spacing,
    },
    OfflineAuthConfig,
};
use bevy::{
    ecs::{
        query::With,
        schedule::{IntoSystemConfigs, SystemConfigs},
        system::{Query, Res, ResMut, Resource, SystemParam},
    },
    log,
    utils::{HashMap, Instant, Uuid},
    window::{PrimaryWindow, Window},
};
use bevy_egui::{
    egui,
    egui::{Ui, Widget},
    EguiContexts,
};
use mr_messages_lib::{
    GameServerState, GetLevelResponse, GetLevelsRequest, GetLevelsUserFilter, InitLevel,
    LevelsListItem, LinkAccountLoginMethod, MatchmakerMessage, MatchmakerRequest, PaginationParams,
    Server,
};
use mr_shared_lib::net::MessageId;
use std::{
    collections::BTreeMap,
    marker::PhantomData,
    net::SocketAddr,
    ops::{Add, Mul, Sub},
    time::Duration,
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

        self.login_method_is_available("google")
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
                    ui.label(format!(" • {error}"));
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

#[derive(Default)]
pub enum AuthUiScreen {
    RefreshAuth,
    #[default]
    SignIn,
    SignUp,
    LinkAccount,
    SetDisplayName,
    GoogleOpenID,
}

pub struct MatchmakerUiState {
    observed_no_ready_servers_at: Option<Instant>,
    servers: HashMap<String, Server>,
    connect_manually_is_active: bool,
    connect_manually_ip_addr: String,
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
    pending_create_server_request: Option<MatchmakerRequest>,
    // We don't immediately send a request, we first wait for a `Ready` server to spin up.
    create_server_request_sent_at: Option<Instant>,
    request_error_message: Option<String>,
}

impl MatchmakerUiState {
    fn has_ready_server(&self) -> bool {
        self.servers
            .values()
            .any(|server| server.state == GameServerState::Ready)
    }
}

#[derive(Clone, PartialEq, Eq)]
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

#[derive(Clone, Copy)]
pub enum MatchmakerUiScreen {
    ServersList,
    CreateServer,
}

impl Default for MatchmakerUiScreen {
    fn default() -> Self {
        Self::ServersList
    }
}

#[derive(PartialEq, Eq)]
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

#[derive(Resource)]
pub struct MainMenuUiState {
    screen: MainMenuUiScreen,
    auth: AuthUiState,
    matchmaker: MatchmakerUiState,
}

impl MainMenuUiState {
    pub fn new(connect_manually_ip_addr: String) -> Self {
        Self {
            screen: Default::default(),
            auth: Default::default(),
            matchmaker: MatchmakerUiState {
                observed_no_ready_servers_at: None,
                servers: Default::default(),
                connect_manually_is_active: false,
                connect_manually_ip_addr,
                selected_server: None,
                levels: Default::default(),
                levels_list_filter: Default::default(),
                selected_level: Default::default(),
                selected_level_data: None,
                screen: Default::default(),
                request_id_counter: Default::default(),
                current_request_id: None,
                pending_create_server_request: None,
                create_server_request_sent_at: None,
                request_error_message: None,
            },
        }
    }
}

#[derive(Copy, Clone)]
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
    offline_auth_config: Res<'w, OfflineAuthConfig>,
    #[system_param(ignore)]
    _marker: PhantomData<&'s ()>,
}

#[derive(SystemParam)]
pub struct UiContext<'w, 's> {
    egui_contexts: EguiContexts<'w, 's>,
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
}

pub fn process_io_messages_system_set() -> SystemConfigs {
    (
        process_auth_messages_system,
        process_matchmaker_messages_system,
        process_persistence_messages_system,
    )
        .into_configs()
        .distributive_run_if(matchmaker_is_initialised)
}

pub fn matchmaker_is_initialised(
    matchmaker_state: Option<Res<MatchmakerState>>,
    main_menu_ui_channels: Option<Res<MainMenuUiChannels>>,
) -> bool {
    // If matchmaker address is not configured (which means that the state and the
    // channels aren't initialized either), we don't want to render the main menu.
    matchmaker_state.is_some() && main_menu_ui_channels.is_some()
}

pub fn init_menu_auth_state_system(
    mut main_menu_ui_state: ResMut<MainMenuUiState>,
    configs: Configs,
) {
    if configs.offline_auth_config.exists() {
        main_menu_ui_state.auth.screen = AuthUiScreen::RefreshAuth;
        main_menu_ui_state.auth.logged_in_as = Some(configs.offline_auth_config.username.clone());
    } else {
        main_menu_ui_state.auth.screen = AuthUiScreen::SignIn;
    }
}

pub fn main_menu_ui_system(
    mut ui_context: UiContext,
    configs: Configs,
    mut main_menu_ui_state: ResMut<MainMenuUiState>,
    matchmaker_state: Option<Res<MatchmakerState>>,
    mut main_menu_ui_channels: Option<ResMut<MainMenuUiChannels>>,
    mut server_to_connect: ResMut<ServerToConnect>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();

    if let Some(matchmaker_state) = &matchmaker_state {
        if !matches!(matchmaker_state.status, TcpConnectionStatus::Connected) {
            main_menu_ui_state.matchmaker.servers.clear();
        }

        if !main_menu_ui_state.matchmaker.has_ready_server()
            && matches!(matchmaker_state.status, TcpConnectionStatus::Connected)
            && main_menu_ui_state
                .matchmaker
                .observed_no_ready_servers_at
                .is_none()
        {
            main_menu_ui_state.matchmaker.observed_no_ready_servers_at = Some(Instant::now());
        }
    }

    let screen_height = ui_context.windows.single().height();

    let window_width = 400.0;
    let window_height = 600.0;
    let offset_y = (200.0 - (200.0 + window_height - screen_height).max(0.0)).max(0.0);

    let ctx = ui_context.egui_contexts.ctx_mut();
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
                        screen: main_menu_ui_screen,
                        auth: auth_ui_state,
                        matchmaker: matchmaker_ui_state,
                    } = &mut *main_menu_ui_state;

                    if let Some(matchmaker_state) = &matchmaker_state {
                        match matchmaker_state.status {
                            TcpConnectionStatus::Connected => {
                                // Header.
                                if !matchmaker_ui_state.has_ready_server()
                                    && matchmaker_ui_state.pending_create_server_request.is_some()
                                {
                                    let progress = Instant::now()
                                        .duration_since(
                                            matchmaker_ui_state.observed_no_ready_servers_at.unwrap(),
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
                    } else {
                        status_bar(ui, "Matchmaker address isn't set", 0.0);
                    }

                    match (*main_menu_ui_screen, main_menu_ui_channels.as_deref_mut()) {
                        (MainMenuUiScreen::Auth, Some(main_menu_ui_channels)) => {
                            egui::containers::Frame::none()
                                .inner_margin(egui::style::Margin::symmetric(25.0, 15.0))
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
                        (MainMenuUiScreen::Matchmaker | MainMenuUiScreen::Auth, main_menu_ui_channels) => {
                            matchmaker_screen(
                                ui,
                                matchmaker_state.as_deref(),
                                matchmaker_ui_state,
                                &mut server_to_connect,
                                main_menu_ui_channels,
                            );
                        }
                    }
                });
        });
}

pub fn process_auth_messages_system(
    mut main_menu_ui_state: ResMut<MainMenuUiState>,
    mut matchmaker_state: ResMut<MatchmakerState>,
    mut main_menu_ui_channels: ResMut<MainMenuUiChannels>,
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

pub fn process_matchmaker_messages_system(
    mut main_menu_ui_state: ResMut<MainMenuUiState>,
    mut main_menu_ui_channels: ResMut<MainMenuUiChannels>,
    mut server_to_connect: ResMut<ServerToConnect>,
) {
    loop {
        if let Some(request) = main_menu_ui_state
            .matchmaker
            .pending_create_server_request
            .clone()
        {
            if main_menu_ui_state
                .matchmaker
                .create_server_request_sent_at
                .map_or(true, |sent_at| {
                    Instant::now().duration_since(sent_at) > Duration::from_secs(5)
                })
            {
                if main_menu_ui_state.matchmaker.has_ready_server() {
                    log::info!("Sending an allocation request: {}", request.request_id());
                    main_menu_ui_state.matchmaker.create_server_request_sent_at =
                        Some(Instant::now());
                    main_menu_ui_channels
                        .matchmaker_request_tx
                        .send(request)
                        .expect("Failed to write to a channel (matchmaker request)");
                }
            } else {
                let requested_server = main_menu_ui_state
                    .matchmaker
                    .servers
                    .values()
                    .find(|server| server.request_id == request.request_id())
                    .cloned();
                if let Some(requested_server) = requested_server {
                    main_menu_ui_state.matchmaker.pending_create_server_request = None;
                    **server_to_connect = Some(requested_server);
                }
            }
        }

        match main_menu_ui_channels.matchmaker_message_rx.try_recv() {
            Ok(MatchmakerMessage::Init { servers: init_list }) => {
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

pub fn process_persistence_messages_system(
    mut main_menu_ui_state: ResMut<MainMenuUiState>,
    mut main_menu_ui_channels: ResMut<MainMenuUiChannels>,
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
            if google_is_available {
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
        AuthUiScreen::GoogleOpenID => {
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

            if let Some(logged_in_as) = auth_ui_state.logged_in_as.as_ref() {
                ui.with_layout(
                    egui::Layout::top_down_justified(egui::Align::Center),
                    |ui| {
                        ui.label(format!("Logged in as {logged_in_as}"));
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
    matchmaker_state: Option<&MatchmakerState>,
    matchmaker_ui_state: &mut MatchmakerUiState,
    server_to_connect: &mut Option<Server>,
    main_menu_ui_channels: Option<&mut MainMenuUiChannels>,
) {
    match (matchmaker_ui_state.screen, matchmaker_state) {
        (MatchmakerUiScreen::CreateServer, Some(_matchmaker_state))
            if matchmaker_ui_state.pending_create_server_request.is_some() =>
        {
            connect_to_server_screen(ui, matchmaker_ui_state)
        }
        (MatchmakerUiScreen::CreateServer, Some(matchmaker_state)) => {
            matchmaker_create_server_screen(
                ui,
                matchmaker_state,
                matchmaker_ui_state,
                main_menu_ui_channels
                    .expect("Expected UI channels to exist when matchmaker state exists")
                    .persistence_request_tx
                    .clone(),
            )
        }
        (MatchmakerUiScreen::ServersList | MatchmakerUiScreen::CreateServer, _) => {
            matchmaker_servers_list_screen(
                ui,
                server_to_connect,
                matchmaker_ui_state,
                main_menu_ui_channels.map(|channels| channels.persistence_request_tx.clone()),
            )
        }
    }
}

fn matchmaker_servers_list_screen(
    ui: &mut egui::Ui,
    server_to_connect: &mut Option<Server>,
    matchmaker_ui_state: &mut MatchmakerUiState,
    persistence_requests_tx: Option<UnboundedSender<PersistenceRequest>>,
) {
    // Server list.
    let mut sorted_servers = matchmaker_ui_state
        .servers
        .values()
        .filter(|server| server.state == GameServerState::Allocated)
        .collect::<Vec<_>>();
    sorted_servers.sort_by(|a, b| a.name.cmp(&b.name));

    without_item_spacing(ui, |ui| {
        egui::containers::ScrollArea::vertical()
            .max_height(500.0)
            .show(ui, |ui| {
                if let Some(persistence_requests_tx) = persistence_requests_tx {
                    let response = MenuListItem::new("Create a server")
                        .secondary_widget(|ui| {
                            ui.label("Create a server to let other players join");
                        })
                        .image_widget(plus_image)
                        .show(ui);
                    if response.item.clicked() {
                        matchmaker_ui_state.connect_manually_is_active = false;
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
                }

                let connect_response = connect_manually_item(
                    &mut matchmaker_ui_state.connect_manually_ip_addr,
                    matchmaker_ui_state.connect_manually_is_active,
                    ui,
                );
                if connect_response.item.clicked() {
                    matchmaker_ui_state.connect_manually_is_active = true;
                    matchmaker_ui_state.selected_server = None;
                }
                if connect_response.secondary.clicked() {
                    match matchmaker_ui_state.connect_manually_ip_addr.parse() {
                        Ok(addr) => {
                            matchmaker_ui_state.connect_manually_is_active = false;
                            *server_to_connect = Some(Server {
                                name: "Unknown".to_string(),
                                state: GameServerState::Ready,
                                addr,
                                player_capacity: 0,
                                player_count: 0,
                                request_id: Default::default(),
                            });
                        }
                        Err(err) => {
                            log::error!("Invalid server addr: {err:?}");
                        }
                    }
                }

                server_list(
                    ui,
                    &sorted_servers,
                    &mut matchmaker_ui_state.selected_server,
                );
                if matchmaker_ui_state.selected_server.is_some() {
                    matchmaker_ui_state.connect_manually_is_active = false;
                }
            });
    });

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
        *server_to_connect = Some(
            matchmaker_ui_state.servers[matchmaker_ui_state.selected_server.as_ref().unwrap()]
                .clone(),
        );
    }
}

fn connect_manually_item(
    connect_manually_ip_addr: &mut String,
    is_active: bool,
    ui: &mut Ui,
) -> MenuListItemResponse<egui::Response, ()> {
    MenuListItem::new("Connect manually")
        .secondary_widget(|ui| {
            if !is_active {
                return ui.label("Select to connect to a server by IP");
            }

            ui.horizontal(|ui| {
                ui.style_mut().visuals.widgets.inactive.bg_stroke =
                    ui.style_mut().visuals.window_stroke();
                egui::widgets::TextEdit::singleline(connect_manually_ip_addr)
                    .desired_width(150.0)
                    .show(ui);
                ui.add_enabled(
                    connect_manually_ip_addr.parse::<SocketAddr>().is_ok(),
                    egui::widgets::Button::new("Connect"),
                )
                .on_disabled_hover_text("Enter a valid server address to connect")
            })
            .inner
        })
        .selected(is_active)
        .image_widget(circle_image)
        .show(ui)
}

fn matchmaker_create_server_screen(
    ui: &mut egui::Ui,
    matchmaker_state: &MatchmakerState,
    matchmaker_ui_state: &mut MatchmakerUiState,
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
        egui::Layout::left_to_right(egui::Align::Min),
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

    let matchmaker_is_connected = matches!(matchmaker_state.status, TcpConnectionStatus::Connected);
    let is_authenticated = matchmaker_state.id_token.is_some();
    let (create_enabled, create_disabled_reason) = match &matchmaker_ui_state.selected_level {
        _ if !matchmaker_is_connected => (false, "Not connected to the matchmaker"),
        SelectedLevel::NewLevel(_) if !is_authenticated => {
            (false, "You must be logged in to create new levels")
        }
        SelectedLevel::NewLevel(title) if title.is_empty() => {
            (false, "New level title cannot be empty")
        }
        SelectedLevel::NewLevel(_) => (true, ""),
        SelectedLevel::Existing(_) => (true, ""),
        SelectedLevel::None => (false, "Select a level to create a server"),
    };
    let (fork_enabled, fork_disabled_reason) = match &matchmaker_ui_state.selected_level {
        _ if !matchmaker_is_connected => (false, "Not connected to the matchmaker"),
        _ if !is_authenticated => (false, "You must be logged in to fork levels"),
        SelectedLevel::NewLevel(_) => (false, "You can fork only an existing level"),
        SelectedLevel::Existing(_) => (true, ""),
        SelectedLevel::None => (false, "Select a level to create a server"),
    };

    let [back_response, create_response, fork_response] = button_panel(
        ui,
        70.0,
        [
            PanelButton::new(egui::Button::new("Back")),
            PanelButton::new(egui::Button::new("Create"))
                .enabled(create_enabled)
                .on_disabled_hover_text(create_disabled_reason),
            PanelButton::new(egui::Button::new("Fork"))
                .enabled(fork_enabled)
                .on_hover_text("Clone and host the selected level")
                .on_disabled_hover_text(fork_disabled_reason),
        ],
    );

    if back_response.clicked() {
        matchmaker_ui_state.screen = MatchmakerUiScreen::ServersList;
        matchmaker_ui_state.selected_level = SelectedLevel::None;
    }

    if create_response.clicked() {
        let request_id = Uuid::new_v4();
        log::info!("Scheduling a create level request: {request_id}");
        let init_level = match &matchmaker_ui_state.selected_level {
            SelectedLevel::NewLevel(level_title) => InitLevel::Create {
                title: level_title.clone(),
                parent_id: None,
            },
            SelectedLevel::Existing(level_id) => InitLevel::Existing(*level_id),
            SelectedLevel::None => unreachable!(),
        };
        let request = MatchmakerRequest::CreateServer {
            init_level,
            request_id,
            id_token: matchmaker_state.id_token.clone(),
        };
        matchmaker_ui_state.pending_create_server_request = Some(request);
    }

    if fork_response.clicked() {
        let request_id = Uuid::new_v4();
        log::info!("Scheduling a fork level request: {request_id}");
        let init_level = match &matchmaker_ui_state.selected_level {
            SelectedLevel::Existing(level_id) => InitLevel::Create {
                title: matchmaker_ui_state.levels[level_id].title.clone(),
                parent_id: Some(*level_id),
            },
            _ => unreachable!(),
        };
        let request = MatchmakerRequest::CreateServer {
            init_level,
            request_id,
            id_token: matchmaker_state.id_token.clone(),
        };
        matchmaker_ui_state.pending_create_server_request = Some(request);
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

fn connect_to_server_screen(ui: &mut egui::Ui, matchmaker_ui_state: &mut MatchmakerUiState) {
    ui.scope(|ui| {
        ui.add_space(20.0);
        ui.with_layout(
            egui::Layout::top_down_justified(egui::Align::Center),
            |ui| {
                ui.style_mut().override_text_style = Some(egui::TextStyle::Heading);
                ui.label("Creating a game server...");
            },
        );
        ui.add_space(10.0);
    });
    let [response] = button_panel(ui, 70.0, [PanelButton::new(egui::Button::new("Back"))]);
    if response.clicked() {
        matchmaker_ui_state.pending_create_server_request = None;
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

#[allow(dead_code)]
fn cancel_image(ui: &mut egui::Ui) {
    ui.painter().circle_stroke(
        ui.max_rect().center(),
        14.0,
        egui::Stroke::new(3.0, ui.visuals().widgets.inactive.bg_fill),
    );
    let mut mesh = egui::Mesh::with_texture(egui::TextureId::Managed(0));
    mesh.add_colored_rect(
        egui::Rect::from_center_size(ui.max_rect().center(), egui::Vec2::new(28.0, 4.0)),
        ui.visuals().widgets.inactive.bg_fill,
    );
    mesh.rotate(
        egui::emath::Rot2::from_angle(-std::f32::consts::FRAC_PI_4),
        ui.max_rect().center(),
    );
    ui.painter().add(mesh);
}

fn circle_image(ui: &mut egui::Ui) {
    ui.painter().circle_stroke(
        ui.max_rect().center(),
        14.0,
        egui::Stroke::new(3.0, ui.visuals().widgets.inactive.bg_fill),
    );
}
