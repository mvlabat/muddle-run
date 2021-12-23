use crate::net::{
    auth::{AuthMessage, AuthRequest},
    MainMenuUiChannels, MatchmakerState, ServerToConnect, TcpConnectionStatus,
};
use bevy::{
    ecs::system::{Local, Res, ResMut},
    log,
    utils::{HashMap, Instant},
};
use bevy_egui::{egui, egui::Widget, EguiContext};
use mr_messages_lib::{MatchmakerMessage, Server};
use mr_shared_lib::net::{ConnectionState, ConnectionStatus};
use std::ops::{Add, Mul, Sub};
use tokio::sync::mpsc::{error::TryRecvError, UnboundedSender};

const ERROR_COLOR: egui::Color32 = egui::Color32::RED;
const INVALID_EMAIL_ERROR: &str = "must be a valid email";
const SHORT_PASSWORD_ERROR: &str = "must be 8 characters or longer";

pub struct AuthUiState {
    screen: AuthUiScreen,
    email: InputField,
    password: InputField,
    #[allow(dead_code)]
    domain: String,
    error_message: String,
    redirect_is_ready: bool,
    pending_request: bool,
    #[allow(dead_code)]
    logged_in_as: Option<String>,
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
            domain: "".to_owned(),
            error_message: "".to_owned(),
            redirect_is_ready: false,
            pending_request: false,
            logged_in_as: None,
        }
    }
}

impl AuthUiState {
    pub fn validate(&mut self) {
        self.email.errors.clear();
        self.password.errors.clear();

        if !self.email.value.contains('@') {
            self.email.errors.push(INVALID_EMAIL_ERROR.to_owned());
        }

        if self.password.value.len() < 8 {
            self.password.errors.push(SHORT_PASSWORD_ERROR.to_owned());
        }
    }
}

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
            .desired_width(350.0)
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
                ui.style_mut().body_text_style = egui::TextStyle::Button;
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
    SignIn,
    SignUp,
    GoogleOpenID,
    #[allow(dead_code)]
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
    selected: Option<String>,
}

#[derive(Default)]
pub struct MainMenuUiState {
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

pub fn matchmaker_ui(
    mut main_menu_ui_state: Local<MainMenuUiState>,
    egui_context: ResMut<EguiContext>,
    matchmaker_state: Option<ResMut<MatchmakerState>>,
    main_menu_ui_channels: Option<ResMut<MainMenuUiChannels>>,
    mut server_to_connect: ResMut<Option<ServerToConnect>>,
    connection_state: Res<ConnectionState>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();

    let (matchmaker_state, mut main_menu_ui_channels) =
        match matchmaker_state.zip(main_menu_ui_channels) {
            Some(resources) => resources,
            None => return,
        };

    loop {
        match main_menu_ui_channels.auth_message_rx.try_recv() {
            Ok(AuthMessage::RedirectUrlServerIsReady) => {
                main_menu_ui_state.auth.redirect_is_ready = true;
            }
            Ok(AuthMessage::Success) => {
                log::debug!("Successful auth");
                main_menu_ui_state.screen = MainMenuUiScreen::Matchmaker;
                main_menu_ui_state.auth.pending_request = false;
                main_menu_ui_state.auth.password.value.clear();
            }
            #[cfg(feature = "unstoppable_resolution")]
            Ok(AuthMessage::InvalidDomainError) => {
                main_menu_ui_state.auth.pending_request = false;
                main_menu_ui_state.auth.error_message =
                    "The requested domain isn't registered".to_owned();
            }
            Ok(AuthMessage::UnavailableError) => {
                log::debug!("Authentication unavailable");
                main_menu_ui_state.auth.pending_request = false;
                main_menu_ui_state.auth.error_message =
                    "The service is unavailable. Please try again later".to_owned();
            }
            Ok(AuthMessage::WrongPasswordError) => {
                log::debug!("Wrong password");
                main_menu_ui_state.auth.pending_request = false;
                main_menu_ui_state.auth.error_message = "Incorrect username or password".to_owned();
            }
            Ok(AuthMessage::SignUpFailedError) => {
                log::debug!("Bad Sign Up");
                main_menu_ui_state.auth.pending_request = false;
                main_menu_ui_state.auth.error_message =
                    "Signing Up failed (email might be taken)".to_owned();
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                panic!("Failed to read from a channel (auth messages)")
            }
        }
    }

    loop {
        match main_menu_ui_channels.matchmaker_message_rx.try_recv() {
            Ok(MatchmakerMessage::Init(init_list)) => {
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
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                panic!("Failed to read from a channel (matchmaker messages)")
            }
        }
    }

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

    let window_width = 400.0;
    let window_height = 600.0;

    let ctx = egui_context.ctx();
    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(egui::Color32::from_black_alpha(200)))
        .show(ctx, |ui| {
            ui.spacing_mut().window_padding = egui::Vec2::splat(ui.visuals().window_stroke().width);
            egui::Window::new("Server browser")
                .title_bar(false)
                .collapsible(false)
                .resizable(false)
                .frame(egui::Frame::window(ui.style()))
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .fixed_size(egui::Vec2::new(window_width, window_height))
                .show(ui.ctx(), |ui| {
                    let MainMenuUiState {
                        ref mut screen,
                        auth: auth_ui_state,
                        matchmaker:
                            MatchmakerUiState {
                                ref observed_empty_servers_at,
                                ref servers,
                                ref mut selected,
                            },
                    } = &mut *main_menu_ui_state;

                    match matchmaker_state.status {
                        TcpConnectionStatus::Connected => {
                            // Header.
                            if servers.is_empty() {
                                let progress = Instant::now()
                                    .duration_since(observed_empty_servers_at.unwrap())
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

                    match screen {
                        MainMenuUiScreen::Auth => {
                            egui::containers::Frame::none()
                                .margin([25.0, 15.0])
                                .show(ui, |ui| {
                                    let confirm = authentication_screen(
                                        ui,
                                        &mut main_menu_ui_channels.auth_request_tx,
                                        auth_ui_state,
                                    );
                                    if confirm {
                                        *screen = MainMenuUiScreen::Matchmaker;
                                    }
                                });
                        }
                        MainMenuUiScreen::Matchmaker => {
                            matchmaker_screen(
                                ui,
                                &matchmaker_state,
                                servers,
                                selected,
                                &mut server_to_connect,
                            );
                        }
                    }
                });
        });
}

fn authentication_screen(
    ui: &mut egui::Ui,
    auth_request_tx: &mut UnboundedSender<AuthRequest>,
    auth_ui_state: &mut AuthUiState,
) -> bool {
    ui.style_mut().spacing.item_spacing = egui::Vec2::new(10.0, 5.0);
    let mut confirm_auth = false;
    let mut new_screen = None;

    auth_ui_state.validate();

    match auth_ui_state.screen {
        AuthUiScreen::SignIn | AuthUiScreen::SignUp => {
            auth_ui_state.email.ui(ui);
            auth_ui_state.password.ui(ui);

            ui.add_space(5.0);

            let is_sign_up = matches!(auth_ui_state.screen, AuthUiScreen::SignUp);
            let is_valid = auth_ui_state.email.is_valid() && auth_ui_state.password.is_valid();
            ui.horizontal(|ui| {
                if egui::widgets::Button::new(if is_sign_up { "Sign Up" } else { "Sign In" })
                    .enabled(!auth_ui_state.pending_request && is_valid)
                    .ui(ui)
                    .clicked()
                {
                    auth_ui_state.pending_request = true;
                    auth_request_tx
                        .send(AuthRequest::Password {
                            username: auth_ui_state.email.value.clone(),
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

            ui.separator();
            ui.label("Continue with an auth provider");

            ui.horizontal(|ui| {
                if egui::widgets::Button::new("Google")
                    .enabled(!auth_ui_state.pending_request && auth_ui_state.redirect_is_ready)
                    .ui(ui)
                    .clicked()
                {
                    auth_ui_state.email.value.clear();
                    auth_ui_state.password.value.clear();
                    auth_ui_state.pending_request = true;
                    new_screen = Some(AuthUiScreen::GoogleOpenID);
                    auth_request_tx
                        .send(AuthRequest::RequestGoogleAuth)
                        .expect("Failed to write to a channel (auth request)");
                }

                #[cfg(feature = "unstoppable_resolution")]
                if ui.button("Unstoppable Domains").clicked() {
                    auth_ui_state.email.value.clear();
                    auth_ui_state.password.value.clear();
                    new_screen = Some(AuthUiScreen::UnstoppableDomainsOpenID);
                }
            });

            ui.separator();

            match auth_ui_state.screen {
                AuthUiScreen::SignIn => {
                    ui.label("Don't have an account?");
                    if egui::widgets::Button::new("Sign Up")
                        .enabled(!auth_ui_state.pending_request)
                        .ui(ui)
                        .clicked()
                    {
                        new_screen = Some(AuthUiScreen::SignUp);
                    }
                }
                AuthUiScreen::SignUp => {
                    ui.label("Already have an account?");
                    if egui::widgets::Button::new("Sign In")
                        .enabled(!auth_ui_state.pending_request)
                        .ui(ui)
                        .clicked()
                    {
                        new_screen = Some(AuthUiScreen::SignIn);
                    }
                }
                _ => unreachable!(),
            }
        }
        AuthUiScreen::GoogleOpenID | AuthUiScreen::UnstoppableDomainsOpenID => {
            ui.horizontal(|ui| {
                if ui.button("Back").clicked() {
                    new_screen = Some(AuthUiScreen::SignIn);
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
                        if egui::widgets::Button::new("Continue")
                            .enabled(!auth_ui_state.domain.is_empty())
                            .ui(ui)
                            .clicked()
                        {
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
                        egui::widgets::Button::new("Continue").enabled(false).ui(ui);
                    },
                );
            }
        }
    }

    ui.add_space(5.0);
    if let Some(new_screen) = new_screen {
        auth_ui_state.screen = new_screen;
    }
    confirm_auth
}

fn matchmaker_screen(
    ui: &mut egui::Ui,
    matchmaker_state: &MatchmakerState,
    servers: &HashMap<String, Server>,
    selected: &mut Option<String>,
    server_to_connect: &mut Option<ServerToConnect>,
) {
    if let TcpConnectionStatus::Connected = matchmaker_state.status {
        // Server list.
        let mut sorted_servers = servers.values().collect::<Vec<_>>();
        sorted_servers.sort_by(|a, b| a.name.cmp(&b.name));
        egui::containers::ScrollArea::from_max_height(500.0).show(ui, |ui| {
            server_list(ui, &sorted_servers, selected);
        });
    }

    // Play button.
    let button_size = egui::Vec2::new(100.0, 30.0);
    let margin = 10.0;
    let is_selected = selected
        .as_ref()
        .map_or(false, |selected| servers.contains_key(selected));
    let (outer_rect, _) = ui.allocate_exact_size(
        egui::Vec2::new(
            ui.available_size_before_wrap().x,
            button_size.y + margin * 2.0,
        ),
        egui::Sense::hover(),
    );
    let button_response = ui.put(
        egui::Rect::from_min_size(outer_rect.center() - button_size / 2.0, button_size),
        egui::widgets::Button::new("Play").enabled(is_selected),
    );
    if button_response.clicked() {
        *server_to_connect = Some(ServerToConnect(servers[selected.as_ref().unwrap()].clone()));
    }
}

fn server_list(ui: &mut egui::Ui, servers: &[&Server], selected: &mut Option<String>) {
    for server in servers {
        let is_selected = selected
            .as_ref()
            .map_or(false, |selected| &server.name == selected);

        let padding = 10.0;
        let spacing = 5.0;
        let (outer_rect, response) = ui.allocate_exact_size(
            egui::Vec2::new(ui.available_size_before_wrap_finite().x, 60.0),
            egui::Sense::click(),
        );

        let inner_rect = outer_rect.shrink(padding);

        let fill = if is_selected {
            Some(ui.style().visuals.extreme_bg_color)
        } else if response.hovered() {
            Some(ui.style().visuals.faint_bg_color)
        } else {
            None
        };

        let server_name_galley = ui
            .fonts()
            .layout_no_wrap(egui::TextStyle::Heading, server.name.clone());
        let server_name_cursor = inner_rect.min;

        let players_galley = ui.fonts().layout_no_wrap(
            egui::TextStyle::Body,
            format!(
                "Players: {}/{}",
                server.player_count, server.player_capacity
            ),
        );
        let players_cursor =
            inner_rect.min + egui::Vec2::new(0.0, server_name_galley.size.y + spacing);

        if let Some(fill) = fill {
            ui.painter().rect_filled(outer_rect, 0.0, fill);
        }
        ui.painter().line_segment(
            [
                egui::Pos2::new(outer_rect.min.x, outer_rect.max.y),
                egui::Pos2::new(outer_rect.max.x, outer_rect.max.y),
            ],
            ui.style().visuals.window_stroke(),
        );
        ui.painter().galley(
            server_name_cursor,
            server_name_galley,
            ui.visuals().text_color(),
        );
        ui.painter()
            .galley(players_cursor, players_galley, ui.visuals().text_color());

        if response.clicked() {
            *selected = Some(server.name.clone());
        }
        response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
}

fn status_bar(ui: &mut egui::Ui, label: impl ToString, progress: f32) {
    let desired_width = ui.available_size_before_wrap_finite().x;
    let height = ui.spacing().interact_size.y;
    let (outer_rect, _response) =
        ui.allocate_exact_size(egui::Vec2::new(desired_width, height), egui::Sense::hover());

    if progress > 0.0 {
        let corner_radius = ui.style().visuals.window_corner_radius;
        let size = outer_rect
            .size()
            .mul(egui::Vec2::new(progress, 1.0))
            .max(egui::Vec2::new(corner_radius * 2.0, outer_rect.height()));
        let fill = egui::Color32::from_rgb(23, 98, 3);
        let stroke = egui::Stroke::new(0.0, egui::Color32::TRANSPARENT);

        ui.painter().add(egui::Shape::Rect {
            rect: egui::Rect::from_min_size(outer_rect.min, size),
            corner_radius,
            fill,
            stroke,
        });
        ui.painter().add(egui::Shape::Rect {
            rect: egui::Rect::from_min_size(
                outer_rect.min.add(egui::Vec2::new(0.0, size.y / 2.0)),
                size.mul(egui::Vec2::new(1.0, 0.5)),
            ),
            corner_radius: 0.0,
            fill,
            stroke,
        });
        if size.x < outer_rect.size().sub(egui::Vec2::new(corner_radius, 0.0)).x {
            ui.painter().add(egui::Shape::Rect {
                rect: egui::Rect::from_min_size(
                    outer_rect.min.add(egui::Vec2::new(corner_radius, 0.0)),
                    size.sub(egui::Vec2::new(corner_radius, 0.0)),
                ),
                corner_radius: 0.0,
                fill,
                stroke,
            });
        }
    }

    ui.painter().text(
        outer_rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        ui.style().body_text_style,
        ui.visuals().text_color(),
    );
}
