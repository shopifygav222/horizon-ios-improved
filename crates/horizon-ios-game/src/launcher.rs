//! Horizon Android's launcher (its native Rust pages: install, play, catalog, files, accounts,
//! settings), drawing with Starling's game's renderer, and starting Starling's game in
//! the same event loop (game_launch.rs).
use crate::catalog::{Catalog, Page};
use horizon_client::game;
mod chooser;
mod gesture;
mod login;
mod pages;

/// The menu atlas and the panorama the launcher draws with.
pub(super) struct Gui {
    pub atlas: HudAtlas,
    pub panorama: Arc<horizon_client::gfx::panorama::Faces>,
}
use horizon_client::{
    Result,
    gfx::Renderer,
    hud::{HudAtlas, HudBatch},
    android_control_theme::{Face, Ui},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, ElementState, MouseButton, TouchPhase, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow},
    window::{Window, WindowId},
};

pub(super) struct Launcher {
    home: PathBuf,
    controller_edges: horizon_client::gamepad::Edges,
    controller_repeat: Option<(horizon_client::gamepad::Button, Instant, bool)>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    gui: Option<Gui>,
    pub(super) audio: Option<horizon_client::audio::Audio>,
    game: Option<Box<dyn ApplicationHandler>>,
    cursor: (f64, f64),
    mouse_down: bool,
    feedback: Option<(pages::Action, Instant)>,
    install: Arc<Mutex<crate::official_install::InstallProgress>>,
    install_paused: Arc<std::sync::atomic::AtomicBool>,
    install_cancelled: Arc<std::sync::atomic::AtomicBool>,
    install_transfer: crate::catalog::TransferTracker,
    pub(super) anim: pages::AnimState,
    worker: Option<mpsc::Receiver<std::result::Result<(), String>>>,
    error: Option<String>,
    catalog: Catalog,
    targets: Vec<([f32; 4], pages::Action)>,
    list_targets: Vec<([f32; 4], pages::Action)>,
    list_view: Option<[f32; 3]>,
    pending_focus: Option<pages::Action>,
    drag: Option<gesture::Drag>,
    chooser: Option<chooser::Chooser>,
    exit_confirmation: bool,
    login: login::Login,
    pub(super) launching: bool,
    pub(super) launch_pending_tick: bool,
    pub(super) diagnostics_modal_open: bool,
    focus: Option<usize>,
    diagnostic_export: Option<mpsc::Receiver<std::result::Result<String, String>>>,
    /// The game asked to start again (a world's own mods): Play once the launcher is back.
    again: bool,
}

impl Launcher {
    fn edit_text(&mut self, output: horizon_client::android_control_keyboard::Output) {
        if self.login.edit(self.catalog.editing, &output) {
            if matches!(
                output,
                horizon_client::android_control_keyboard::Output::Enter | horizon_client::android_control_keyboard::Output::Close
            ) {
                self.catalog.editing = None;
                crate::platform::hide();
            }
        } else {
            self.catalog.edit(output, &self.home);
        }
    }

    pub(super) fn new(home: PathBuf) -> Self {
        let catalog = Catalog::new(&home);
        horizon_client::android_control_theme::set_ui_scale_percent(catalog.config.ui_scale_percent);
        horizon_client::android_control_theme::motion::set_launcher_animations_disabled(
            catalog.config.disable_launcher_animations,
        );
        horizon_client::android_control_theme::motion::set_game_animations_disabled(
            catalog.config.disable_game_animations,
        );
        Self {
            home,
            controller_edges: Default::default(),
            controller_repeat: None,
            window: None,
            renderer: None,
            gui: None,
            audio: None,
            game: None,
            cursor: (0.0, 0.0),
            mouse_down: false,
            feedback: None,
            install: Arc::default(),
            install_paused: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            install_cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            install_transfer: Default::default(),
            anim: Default::default(),
            worker: None,
            error: None,
            catalog,
            targets: vec![],
            list_targets: vec![],
            list_view: None,
            pending_focus: None,
            drag: None,
            chooser: None,
            exit_confirmation: false,
            login: login::Login::new(),
            launching: false,
            launch_pending_tick: false,
            diagnostics_modal_open: false,
            focus: None,
            diagnostic_export: None,
            again: false,
        }
    }

    /// Whether Starling's game is running (the launcher only forwards events to it).
    pub(super) fn playing(&self) -> bool {
        self.game.is_some()
    }

    /// The game ended to start again: the launcher plays once it is back (`restore_launcher`).
    pub(super) fn play_again(&mut self) {
        self.again = true;
    }

    fn restore_launcher(&mut self, events: &ActiveEventLoop) {
        if !crate::platform::take_launcher_return() {
            return;
        }
        if let Some(mut game) = self.game.take() {
            game.exiting(events);
            drop(game);
        }
        crate::document_store::set_game_active(false);
        horizon_client::performance::stop();
        let _ = crate::platform::capture_mouse(false);
        crate::platform::hide();
        self.launching = false;
        self.launch_pending_tick = false;
        horizon_client::discord_presence::stop();
        horizon_client::android_control_theme::reset_atlas();
        self.cursor = (0.0, 0.0);
        self.drag = None;
        self.chooser = None;
        self.exit_confirmation = false;
        self.focus = None;
        self.pending_focus = None;
        if let Err(error) = std::env::set_current_dir(&self.home) {
            self.fail(error);
            return;
        }
        self.controller_edges = Default::default();
        self.controller_repeat = None;
        self.catalog.refresh(&self.home);
        self.login.refresh_status();
        horizon_client::diagnostics::heartbeat("returned_to_launcher", || serde_json::json!({}));
        self.catalog.page = Page::Play;
        if let Ok(message) = std::fs::read_to_string(self.home.join("ios-error.txt")) {
            self.error = Some(message);
        }
        if let Err(error) = self.open(events) {
            self.fail(error);
            events.exit();
        } else if std::mem::take(&mut self.again) {
            self.launch_pending_tick = true;
        }
    }
    pub(super) fn configure_screen(&self) {
        crate::native_ime::configure_screen(
            self.catalog.config.screen.immersive,
            self.catalog.config.screen.avoid_cutout,
            self.catalog.config.screen.aspect,
        );
        // Orientation is UIKit's (display.rs asks for the chosen ones).
    }
    fn fail(&mut self, error: impl std::fmt::Display) {
        log::error!("{error}");
        self.launching = false;
        self.launch_pending_tick = false;
        horizon_client::diagnostics::record_error(&error.to_string());
        self.error = Some(error.to_string());
        let _ = std::fs::write(self.home.join("ios-error.txt"), error.to_string());
    }

    fn open(&mut self, events: &ActiveEventLoop) -> Result<()> {
        crate::platform::set_orientation(self.catalog.config.orientation);
        self.configure_screen();
        horizon_client::android_control_theme::set_android_dpi(crate::platform::display_density());
        horizon_client::android_control_theme::set_ui_scale_percent(self.catalog.config.ui_scale_percent);
        crate::display::set_scale(self.catalog.config.render_scale);
        if let (Some(renderer), Some(window)) = (&mut self.renderer, &self.window) {
            if let Some(size) = crate::display::prepare(window)? {
                renderer.resize(size[0], size[1]);
            }
            renderer.recreate_surface()?;
            let size = window.inner_size();
            renderer.resize(size.width, size.height);
            return Ok(());
        }
        let window =
            Arc::new(events.create_window(Window::default_attributes().with_title("Starling"))?);
        crate::platform::attach_window(&window);
        crate::display::prepare(&window)?;
        // The game's own menu art once it is installed; before that, the low-budget skin the app
        // carries (Starling's setup draws with it too).
        let resources = crate::official_install::resources();
        let atlas_root = if resources.join("assets/minecraft/textures/gui").is_dir() {
            resources
        } else {
            horizon_client::preparer::skin()
                .ok_or("The app's menu art is missing: install it again.")?
        };
        let gui = Gui {
            atlas: HudAtlas::build(&atlas_root)?,
            panorama: Arc::new(horizon_client::gfx::panorama::Faces::colour([0; 4])),
        };
        let renderer = game::loading_renderer(
            window.clone(),
            false,
            Some(&gui.atlas),
            Some(&gui.panorama),
            None,
            horizon_client::graphics::GraphicsApi::Auto,
        )?;
        self.renderer = Some(renderer);
        self.window = Some(window);
        self.gui = Some(gui);
        log::info!("Horizon: the launcher is shown");
        Ok(())
    }

    fn request_back(&mut self) {
        crate::platform::hide();
        if !self.anim.intro_done {
            self.anim.intro_done = true;
            return;
        }
        if self.diagnostics_modal_open {
            self.diagnostics_modal_open = false;
        } else if self.exit_confirmation {
            self.exit_confirmation = false;
        } else if self.catalog.fm_modal.take().is_some() {
            self.catalog.editing = None;
        } else if self.catalog.project_modal.take().is_some() {
            self.catalog.scroll = 0.0;
        } else if self.login.visible {
            self.login.hide();
        } else if self.chooser.take().is_some() {
            // Back dismisses the current dialog, including the exit question.
        } else if self.catalog.page == Page::Installed
            && self.catalog.installed_sub_tab == crate::catalog::InstalledSubTab::FileManager
            && self.catalog.fm_back(&self.home)
        {
            // Back in File Manager pops the directory history stack.
        } else if self.catalog.page == Page::Settings
            && self.catalog.settings_category != crate::catalog::SettingsCategory::Overview
        {
            self.catalog.settings_category =
                if self.catalog.settings_category == crate::catalog::SettingsCategory::Theme {
                    crate::catalog::SettingsCategory::Launcher
                } else {
                    crate::catalog::SettingsCategory::Overview
                };
            self.catalog.scroll = 0.0;
            self.anim.trigger_page_transition();
        } else if self.catalog.editing.take().is_none() {
            self.exit_confirmation = true;
        }
        self.focus = None;
        self.pending_focus = self.exit_confirmation.then_some(pages::Action::Dismiss);
        self.targets.clear();
        self.list_targets.clear();
        self.drag = None;
        if let Err(error) = self.catalog.config.save(&self.home) {
            self.fail(error);
        }
    }

    fn action(&mut self, events: &ActiveEventLoop) -> Result<()> {
        if self.exit_confirmation
            || self.chooser.is_some()
            || self.worker.is_some()
            || self.launch_pending_tick
        {
            return Ok(());
        }
        if self.login.pending() {
            return Ok(());
        }
        horizon_client::diagnostics::heartbeat(
            "play_requested",
            || serde_json::json!({"resolution_percent": self.catalog.config.render_scale.percent()}),
        );
        if crate::official_install::ready(&self.home) {
            if !self.launching {
                self.launching = true;
                self.error = None;
                self.catalog.status = "Launching Minecraft 26.3...".into();
                self.launch_pending_tick = true;
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                return Ok(());
            }
            crate::platform::hide();
            self.catalog.config.save(&self.home)?;
            self.catalog.config.apply_experimental_to_game(&self.home);
            let game_home = self.catalog.config.game_home(&self.home);
            std::fs::create_dir_all(&game_home)?;
            let extra_arguments = if crate::PUBLIC_BUILD {
                Vec::new()
            } else {
                match std::fs::read(self.home.join("ios-launch.json")) {
                    Ok(bytes) => serde_json::from_slice::<Vec<String>>(&bytes)?,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                    Err(e) => return Err(e.into()),
                }
            };

            self.audio = None;
            self.error = None;
            let _ = std::fs::remove_file(self.home.join("ios-error.txt"));
            // The A press that launches the game must not accept its first menu too.
            self.controller_edges = Default::default();
            self.controller_repeat = None;
            let request = crate::game_launch::LaunchRequest {
                resources: crate::official_install::resources(),
                game_home,
                server: self.catalog.config.server.clone(),
                control_socket: (!crate::PUBLIC_BUILD).then(|| self.home.join("control.json")),
                extra_arguments,
            };
            crate::document_store::set_game_active(true);
            let game = match crate::game_launch::start(request) {
                Ok(game) => game,
                Err(error) => {
                    crate::document_store::set_game_active(false);
                    horizon_client::performance::stop();
                    return Err(error);
                }
            };
            // Android has one native window; release its surface before handing it over.
            self.renderer = None;
            self.window = None;
            self.gui = None;
            horizon_client::android_control_theme::reset_atlas();
            self.game = Some(game);
            if let Some(game) = &mut self.game {
                game.new_events(events, winit::event::StartCause::Init);
                game.resumed(events);
            }
        } else {
            if !std::env::current_exe()?
                .parent()
                .is_some_and(|app| app.join("assets/engine-data.zip").is_file())
            {
                return Err(
                    "This app is missing its engine data. Install a complete build.".into(),
                );
            }
            self.error = None;
            *self
                .install
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Default::default();
            self.install_paused
                .store(false, std::sync::atomic::Ordering::Relaxed);
            self.install_cancelled
                .store(false, std::sync::atomic::Ordering::Relaxed);
            self.install_transfer.reset();
            let home = self.home.clone();
            let install = self.install.clone();
            let paused = self.install_paused.clone();
            let cancelled = self.install_cancelled.clone();
            let (sender, receiver) = mpsc::channel();
            self.worker = Some(receiver);
            std::thread::spawn(move || {
                let result = crate::official_install::install_with_progress(&home, |update| {
                    while paused.load(std::sync::atomic::Ordering::Relaxed)
                        && !cancelled.load(std::sync::atomic::Ordering::Relaxed)
                    {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    let mut current = install
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let done = if current.stage == update.stage {
                        current.done.max(update.done)
                    } else {
                        update.done
                    };
                    *current = crate::official_install::InstallProgress { done, ..update };
                });
                let _ = sender.send(result.map_err(|e| e.to_string()));
            });
        }
        Ok(())
    }
}

impl ApplicationHandler for Launcher {
    fn new_events(&mut self, events: &ActiveEventLoop, cause: winit::event::StartCause) {
        if let Some(game) = &mut self.game {
            game.new_events(events, cause);
        }
    }
    fn user_event(&mut self, events: &ActiveEventLoop, event: ()) {
        if let Some(game) = &mut self.game {
            game.user_event(events, event);
        }
    }
    fn memory_warning(&mut self, events: &ActiveEventLoop) {
        if let Some(game) = &mut self.game {
            game.memory_warning(events);
        }
    }
    fn resumed(&mut self, events: &ActiveEventLoop) {
        self.configure_screen();
        if let Some(game) = &mut self.game {
            return game.resumed(events);
        }
        if let Err(error) = self.open(events) {
            self.fail(error);
            events.exit();
        }
    }
    fn suspended(&mut self, events: &ActiveEventLoop) {
        horizon_client::diagnostics::heartbeat("suspended", || serde_json::json!({}));
        if let Some(game) = &mut self.game {
            return game.suspended(events);
        }
        self.drag = None;
        if let Some(renderer) = &mut self.renderer {
            renderer.suspend();
        }
    }
    fn window_event(&mut self, events: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if crate::documents::handle_event(&event) {
            return;
        }
        if matches!(event, WindowEvent::Resized(_)) {
            let _ = crate::native_ime::viewport_bottom();
        }
        if matches!(event, WindowEvent::Focused(true)) {
            self.configure_screen();
        }
        if let Some(game) = &mut self.game {
            game.window_event(events, id, event);
            self.restore_launcher(events);
            return;
        }
        let result = match event {
            WindowEvent::CloseRequested => {
                self.request_back();
                Ok(())
            }
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width, size.height);
                }
                Ok(())
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.focus = None;
                self.pending_focus = None;
                self.cursor = (position.x, position.y);
                Ok(())
            }
            WindowEvent::Touch(touch) => self.touch_event(events, touch),
            WindowEvent::Focused(false) => {
                horizon_client::diagnostics::heartbeat("launcher_background", || serde_json::json!({}));
                self.controller_repeat = None;
                self.mouse_down = false;
                self.drag = None;
                Ok(())
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                if state == ElementState::Pressed {
                    self.mouse_down = true;
                    Ok(())
                } else if std::mem::take(&mut self.mouse_down) {
                    self.activate(events)
                } else {
                    Ok(())
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if self.exit_confirmation {
                    return;
                }
                let y = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y * 42.0,
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / self.ui_density(),
                };
                if let Some(chooser) = &mut self.chooser {
                    chooser.scroll = (chooser.scroll - y).max(0.0);
                } else {
                    self.catalog.scroll = (self.catalog.scroll - y).max(0.0);
                }
                Ok(())
            }
            WindowEvent::RedrawRequested => self.draw_pages(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if event.physical_key
                    == winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::Escape)
                    || event.logical_key
                        == winit::keyboard::Key::Named(winit::keyboard::NamedKey::BrowserBack)
                {
                    self.type_key(event);
                    Ok(())
                } else if self.navigate(&event) {
                    Ok(())
                } else if self.catalog.editing.is_none()
                    && !event.repeat
                    && matches!(
                        event.logical_key,
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter)
                    )
                {
                    self.activate(events)
                } else {
                    self.type_key(event);
                    Ok(())
                }
            }
            WindowEvent::Ime(winit::event::Ime::Commit(text)) => {
                self.edit_text(horizon_client::android_control_keyboard::Output::Text(text));
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(error) = result {
            self.fail(error);
        }
    }
    fn device_event(&mut self, events: &ActiveEventLoop, id: DeviceId, event: DeviceEvent) {
        if matches!(event, DeviceEvent::Removed) {
            self.mouse_down = false;
            self.drag = None;
        }
        if let Some(game) = &mut self.game {
            game.device_event(events, id, event);
        }
    }
    fn about_to_wait(&mut self, events: &ActiveEventLoop) {
        if let Some(game) = &mut self.game {
            game.about_to_wait(events);
            self.restore_launcher(events);
            return;
        }
        horizon_client::diagnostics::heartbeat("launcher", || serde_json::json!({}));
        if let Some(result) = self
            .diagnostic_export
            .as_ref()
            .and_then(|r| r.try_recv().ok())
        {
            self.diagnostic_export = None;
            self.catalog.status = match result {
                // Files' sheet saves a copy where the player chooses, as the game's own export.
                Ok(name) if horizon_client::ios::share_diagnostic(&name) => format!(
                    "{name} is ready: choose where to keep it. Review it before sharing; nothing was uploaded."
                ),
                Ok(name) => format!("{name} was made, but the share sheet could not open."),
                Err(error) => format!("Diagnostic export failed: {error}"),
            };
        }
        if let Err(error) = self.controller_frame(events) {
            self.fail(error);
        }
        if let (Some(window), Some(renderer)) = (&self.window, &mut self.renderer)
            && let Err(error) = crate::display::apply(window, renderer)
        {
            crate::display::set_scale(crate::display::RenderScale::default());
            self.fail(error);
        }
        if let Some(result) = self.worker.as_ref().and_then(|w| w.try_recv().ok()) {
            self.worker = None;
            if let Err(error) = result {
                let message = error.to_string();
                if message.contains("Dns Failed") || message.contains("lookup address") {
                    self.fail("Minecraft download interrupted: DNS could not resolve the download host. Check your connection, VPN or Private DNS, then retry Download. Completed files are kept.");
                } else {
                    self.fail(error);
                }
            }
        }
        for out in crate::platform::poll() {
            if matches!(out, horizon_client::android_control_keyboard::Output::Enter) {
                crate::platform::hide();
            }
            self.edit_text(out);
        }
        if let Some(audio) = &mut self.audio {
            audio.tick(false, &Default::default());
        }
        self.catalog.poll(&self.home);
        if let Some(notice) = crate::documents::poll() {
            self.catalog.status = notice.text;
            if notice.changed {
                self.catalog.refresh(&self.home);
                self.catalog.show_installed(notice.kind);
            }
        }
        for result in crate::gallery::poll() {
            self.catalog.status = result.unwrap_or_else(|e| format!("File export failed: {e}"));
        }
        // Starling's account state (a sign-in under way, the profile that plays).
        self.login.poll();

        if self.launch_pending_tick {
            self.launch_pending_tick = false;
            if let Err(error) = self.action(events) {
                self.fail(error);
            }
            if self.game.is_some() {
                return;
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        let frame_ms = if self.worker.is_some()
            || self.launching
            || self.catalog.busy()
            || self.drag.is_some()
            || self.login.pending()
            || self
                .catalog
                .project_modal
                .as_ref()
                .is_some_and(|m| m.loading)
            || self.anim.is_active()
            || self
                .feedback
                .is_some_and(|(_, t)| t.elapsed() < Duration::from_millis(180))
        {
            16
        } else {
            33
        };
        events.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(frame_ms),
        ));
    }
    fn exiting(&mut self, events: &ActiveEventLoop) {
        if let Some(game) = &mut self.game {
            game.exiting(events);
        }
    }
}

impl Launcher {
    fn controller_frame(&mut self, events: &ActiveEventLoop) -> Result<()> {
        use horizon_client::gamepad::Button;
        use winit::keyboard::{Key, NamedKey};
        let mut state = crate::platform::gamepad_state();
        for (b, held) in [
            (Button::DPadLeft, state.left[0] < -0.55),
            (Button::DPadRight, state.left[0] > 0.55),
            (Button::DPadUp, state.left[1] > 0.55),
            (Button::DPadDown, state.left[1] < -0.55),
        ] {
            state.set(b, held || state.held(b));
        }
        let now = Instant::now();
        let mut pressed: Vec<_> = self
            .controller_edges
            .update(&state, 0.5)
            .into_iter()
            .filter_map(|(b, down)| down.then_some(b))
            .collect();
        if let Some((button, last, repeating)) = self.controller_repeat {
            if !state.held(button) {
                self.controller_repeat = None;
            } else if now.duration_since(last).as_secs_f32() >= if repeating { 0.09 } else { 0.35 }
            {
                pressed.push(button);
                self.controller_repeat = Some((button, now, true));
            }
        }
        for b in pressed {
            let direction = match b {
                Button::DPadUp => Some(NamedKey::ArrowUp),
                Button::DPadDown => Some(NamedKey::ArrowDown),
                Button::DPadLeft => Some(NamedKey::ArrowLeft),
                Button::DPadRight => Some(NamedKey::ArrowRight),
                _ => None,
            };
            if let Some(key) = direction {
                self.navigate_key(&Key::Named(key));
                if self.controller_repeat.is_none_or(|(old, _, _)| old != b) {
                    self.controller_repeat = Some((b, now, false));
                }
            } else if b == Button::South {
                self.activate(events)?;
            } else if b == Button::East {
                crate::platform::hide();
                self.catalog.editing = None;
                if self.chooser.take().is_none() {
                    if self.login.visible {
                        self.login.visible = false;
                    } else {
                        self.catalog.page = Page::Play;
                    }
                }
                self.focus = None;
            }
        }
        Ok(())
    }
}
