//! Horizon Android's touch controls on iOS, the "Horizon controls" scheme (IOS-SYNC.md): its
//! GoyControl and MCPE styles with their presets, the virtual mouse, the dynamic joystick, layout
//! profiles with snap to grid, the control hub, the shield button and the keypad profile, as its
//! game drove them (its android_input.rs and the touch parts of its game.rs and pad.rs), over
//! Starling's game. They play through the touch pad Starling's own controls use (pad.rs), and
//! type through Starling's keyboard (touch.rs); Starling's Bedrock-style controls are the other
//! scheme (`starling_controls` in horizon-touch.json, Options > Controls).
use super::input_trace::InputOrigin;
use super::*;
use horizon_client::{
    debug_keys::KeyAction,
    gamepad::{Button, PadState},
    input_mode::InputMode,
    touch_ui::{Action, Gesture},
};
use std::collections::{HashMap, HashSet};
use winit::{
    event::{
        DeviceId, ElementState, MouseButton, MouseScrollDelta, Touch, TouchPhase, WindowEvent,
    },
    keyboard::{KeyCode, PhysicalKey},
    window::WindowId,
};

/// What the pad keeps for the Horizon controls (Horizon Android's `Pads` fields).
#[derive(Default)]
pub(super) struct HorizonPad {
    pub(super) touch: horizon_client::horizon_touch::TouchControls,
    pub(super) ui: horizon_client::touch_ui::TouchUi,
    /// The keys custom buttons hold down.
    pub(super) custom_keys: HashSet<KeyCode>,
    /// The tick each custom token went down in, and releases held back until a tick has passed.
    pub(super) custom_pressed_at: HashMap<String, u64>,
    pub(super) late_custom_releases: Vec<String>,
    /// A hardware keyboard's keys held, released when another input takes over.
    pub(super) hardware_keys: HashSet<KeyCode>,
    pub(super) last_input_mode: Option<InputMode>,
    /// The shield (use) held by the shield button or the sneak.
    pub(super) shield: bool,
    /// A released tap's aim, kept until the tick used its queued click.
    pub(super) aim: Option<[f32; 2]>,
}

impl HorizonPad {
    /// A custom button holds sneak or sprint, or the key it is bound to (Horizon Android's
    /// `Pads::holds_action`, its touch part).
    pub(super) fn holds_action(&self, action: &str) -> bool {
        let (keys, tokens) = match action {
            "key.sneak" => (
                [KeyCode::ShiftLeft, KeyCode::ShiftRight],
                [
                    "key.keyboard.left.shift",
                    "key.keyboard.right.shift",
                    "action:key.sneak",
                ],
            ),
            "key.sprint" => (
                [KeyCode::ControlLeft, KeyCode::ControlRight],
                [
                    "key.keyboard.left.control",
                    "key.keyboard.right.control",
                    "action:key.sprint",
                ],
            ),
            _ => return false,
        };
        keys.iter().any(|k| self.custom_keys.contains(k))
            || self
                .touch
                .held_custom_tokens()
                .chain(self.late_custom_releases.iter().map(String::as_str))
                .any(|t| tokens.contains(&t))
    }
}

impl Game<'_> {
    /// The Horizon controls play, not Starling's own.
    pub(super) fn horizon_scheme(&self) -> bool {
        !self.settings.touch.starling_controls
    }

    /// The touch screen (true) or the keyboard and mouse (false) used: Horizon Android's
    /// `touch_input`, Starling's input mode.
    pub(super) fn horizon_input(&mut self, touch: bool) {
        self.input_used(if touch {
            InputMode::Touch
        } else {
            InputMode::KeyboardMouse
        });
    }

    /// Starling's on-screen keyboard, for the field being typed in (touch.rs shows it).
    pub(super) fn horizon_open_keyboard(&mut self) {
        // The in-world keyboard button types in the chat.
        if !self.pad_in_menu() {
            self.open_chat("");
        }
        self.touch.wanted = true;
    }
    pub(super) fn horizon_hide_keyboard(&mut self) {
        self.touch.wanted = false;
    }

    /// The phone's buzz (Starling's haptics), as Horizon Android's pulses.
    pub(super) fn horizon_vibrate(&self, pulse: horizon_client::gamepad::Pulse) {
        crate::android::vibrate(pulse.millis);
    }

    /// The control hub's Logs: the game folder's performance reports, in the Files app.
    pub(super) fn horizon_open_reports(&self) -> bool {
        let dir = std::env::current_dir()
            .map(|dir| dir.join("performance"))
            .unwrap_or_else(|_| "performance".into());
        let _ = std::fs::create_dir_all(&dir);
        horizon_client::platform::open_folder(&dir).unwrap_or(false)
    }

    /// Where the Horizon controls aim (tap to interact): window pixels.
    pub(super) fn horizon_aim_point(&self) -> Option<[f32; 2]> {
        let point = self.pad.horizon.touch.aim.or(self.pad.horizon.aim)?;
        let [w, h] = self.touch_size().map(|v| v as f32);
        Some([point[0] * w, point[1] * h])
    }

    /// The crosshair targets: a keyboard and mouse, the keypad, a crosshair mode, or a projectile
    /// held (Horizon Android's `interaction_crosshair`).
    pub(super) fn horizon_crosshair(&self) -> bool {
        self.input_mode == InputMode::KeyboardMouse
            || self.settings.keypad.enabled
            || self.pad.horizon.touch.settings.crosshair()
            || self.touch_projectile_crosshair()
    }

    /// Whether the fingers aim (tap to interact) while the Horizon controls play.
    pub(super) fn horizon_taps(&self) -> bool {
        self.touch_shown()
            && self.pad.horizon.touch.editor.is_none()
            && self.camera_type.first_person()
            && !self.horizon_crosshair()
    }

    /// Horizon Android's `apply_settings` part: the touch settings follow the game's own.
    pub(super) fn horizon_apply_settings(&mut self) {
        if self.settings.touch.auto_jump != self.settings.auto_jump {
            self.settings.touch.auto_jump = self.settings.auto_jump;
            let _ = self.settings.touch.save();
        }
        if self.settings.touch.sprint_toggle != self.settings.toggle_sprint {
            let s = self.settings.toggle_sprint || self.settings.touch.sprint_toggle;
            self.settings.touch.sprint_toggle = s;
            self.settings.touch.toggle_sprint = s;
            self.settings.toggle_sprint = s;
        }
        if self.pad.horizon.touch.editor.is_none() {
            self.pad.horizon.touch.settings = self.settings.touch.clone();
        }
        self.pad.horizon.touch.force_crosshair = !self.touch_shown() || self.settings.keypad.enabled;
        let changed =
            horizon_client::android_runtime::keypad_enabled() != self.settings.keypad.enabled;
        horizon_client::android_runtime::configure_keypad_settings(&self.settings.keypad);
        if changed {
            self.pad.horizon.touch.clear();
            self.horizon_input(!self.settings.keypad.enabled);
        }
    }

    /// Once per loop iteration: the custom buttons' key events, the virtual mouse's pointer and
    /// the editor's keyboard request (Horizon Android's `android_poll_keyboard`; typing reaches
    /// the fields as key events here).
    pub(super) fn horizon_frame(&mut self) {
        self.android_custom_events();
        self.android_sync_pointer();
        if let Some(ed) = self.pad.horizon.touch.editor.as_mut()
            && std::mem::take(&mut ed.keyboard_request)
        {
            self.touch.wanted = true;
        }
        self.horizon_gyro();
    }

    /// The gyroscope turns the view while the touch screen plays and the controls' Gyro is on
    /// (Horizon Android's `pad_look`), through Starling's motion readings.
    fn horizon_gyro(&mut self) {
        let wanted = self.settings.touch.gyroscope
            && !self.pad_in_menu()
            && self.player.is_some()
            && self.focused
            && self.touch_shown();
        if wanted != self.touch.gyro_on {
            if wanted && !crate::android::gyroscope(true) {
                log::info!("Horizon: no gyroscope to aim with");
            } else if !wanted {
                crate::android::gyroscope(false);
            }
            self.touch.gyro_on = wanted;
        }
        if !wanted {
            return;
        }
        let Some([x, y, rotation]) = crate::android::take_gyro() else {
            return;
        };
        let [yaw, pitch] = horizon_client::touch::gyro_turn([x, y], rotation as i32);
        let sensitivity = self.settings.touch.gyro_sensitivity;
        if yaw != 0.0 || pitch != 0.0 {
            self.tutorial
                .mouse(f64::from(yaw * sensitivity), f64::from(pitch * sensitivity));
            self.turn_player(
                f64::from(yaw * sensitivity / 0.15),
                f64::from(pitch * sensitivity / 0.15),
            );
        }
    }

    /// The pad's poll, before it reads the pads (Horizon Android's `poll_pads`, its touch part).
    pub(super) fn horizon_pad_prelude(&mut self, event_loop: &dyn super::event_loop::EventLoopCtl) {
        if !matches!(self.screen, Screen::Playing | Screen::Options)
            && self.pad.horizon.touch.editor.is_some()
        {
            self.pad.horizon.touch.cancel_editor();
        }
        self.pad.horizon.touch.toggle_crouch = self.settings.toggle_crouch;
        self.pad.horizon.touch.in_inventory =
            matches!(self.screen, Screen::Inventory | Screen::Creative);
        self.pad.horizon.touch.inventory_mode = self.android_inventory_controls();
        if !self.pad.horizon.touch.in_inventory {
            self.pad.horizon.touch.inventory_shift = false;
        }
        self.android_virtual_mouse_clicks(event_loop);
        let active = self.input_mode;
        if self
            .pad
            .horizon
            .last_input_mode
            .replace(active)
            .is_some_and(|old| old == InputMode::KeyboardMouse && old != active)
        {
            for key in std::mem::take(&mut self.pad.horizon.hardware_keys) {
                self.android_key_up(key);
            }
            horizon_client::android_runtime::clear_keypad();
        }
        if !self.focused {
            self.pad.horizon.touch.clear();
            horizon_client::android_runtime::clear_keypad();
        } else if self.pad_in_menu() {
            self.pad.horizon.touch.clear_world_gesture();
            if !self.android_inventory_controls() {
                horizon_client::android_runtime::clear_keypad();
            }
        }
        self.android_custom_events();
        self.pad.horizon.touch.force_crosshair = !self.touch_shown() || self.settings.keypad.enabled;
    }

    /// The Horizon controls' pad this poll, the keypad's merged in (Horizon Android's
    /// `poll_pads`): the shield, the held use and the taps.
    pub(super) fn horizon_pad_state(&mut self) -> PadState {
        self.pad.horizon.touch.shield_available = self.touch_shield_available();
        // A crouch toggled on land would keep sinking, descending or dismounting once the
        // player swims, flies or rides; there the button holds, so the toggle ends.
        if self.touch_shown()
            && (!self.settings.toggle_crouch || self.android_touch_descending())
            && self.toggles.drop_pad_toggle(SNEAK)
        {
            self.sync_toggle(SNEAK);
        }
        let mut touch = self.pad.horizon.touch.poll_state();
        self.pad.horizon.shield = self.touch_shown()
            && !self.pad_in_menu()
            && self.touch_shield_available()
            && if self.settings.touch.shield_button {
                self.pad.horizon.touch.shield_held()
            } else {
                self.pad.horizon.touch.sneaking || touch.held(Button::RightStick)
            };
        if self.pad.horizon.shield {
            touch.triggers[0] = 1.0;
        }
        if let Some(point) = self.pad.horizon.touch.aim {
            self.pad.horizon.aim = Some(point);
        }
        if self.pad.horizon.touch.aim.is_some()
            && self.pad.horizon.touch.holding
            && (self.pad.horizon.touch.holding_use
                || self.touch_item_held_use()
                || self.touch_entity_accepts_use())
        {
            touch.triggers = [1.0, 0.0];
            self.pad.horizon.touch.latch_use();
        }
        if let Some(held_use) = self.pad.horizon.touch.take_tap() {
            if self.pick_entity_hit().is_some()
                && self.using.is_none()
                && self.screen == Screen::Playing
            {
                // The entity's attack now, never a held button: a mob gone before the next tick
                // cannot leave a block exposed.
                if self.remote.is_some() {
                    self.remote_start_attack();
                } else {
                    self.attack();
                }
            } else if !held_use {
                // Block placement and empty-hand use; items with a use action wait for the
                // long press instead.
                touch.triggers[0] = 1.0;
            }
        }
        touch.merge(&horizon_client::android_runtime::keypad_state())
    }

    /// After the pad's edges: custom releases held back a tick, and the tap's aim once nothing
    /// clicks any more.
    pub(super) fn horizon_pad_after(&mut self, state: &PadState, late_releases: bool) {
        let ticking = self.title.is_none() && self.screen == Screen::Playing && !self.pause;
        for token in std::mem::take(&mut self.pad.horizon.late_custom_releases) {
            if !ticking || self.pad.horizon.custom_pressed_at.get(&token) != Some(&self.ticks) {
                self.android_dispatch_custom_token(&token, false);
            } else {
                self.pad.horizon.late_custom_releases.push(token);
            }
        }
        if state.triggers == [0.0; 2] && !late_releases {
            self.pad.horizon.aim = None;
        }
    }

    /// A custom button holds this Shift or Control (Horizon Android's custom keys and latches).
    pub(super) fn horizon_holds_modifier(&self, shift: bool) -> bool {
        let (left, right) = if shift {
            (KeyCode::ShiftLeft, KeyCode::ShiftRight)
        } else {
            (KeyCode::ControlLeft, KeyCode::ControlRight)
        };
        (shift
            && matches!(self.screen, Screen::Inventory | Screen::Creative)
            && self.pad.horizon.touch.inventory_shift)
            || self.pad.horizon.ui.modifier_latched(left)
            || self.pad.horizon.custom_keys.contains(&left)
            || self.pad.horizon.custom_keys.contains(&right)
    }

    /// A Horizon custom button holds Shift (or Control) now.
    pub(super) fn horizon_custom_holds(&self, shift: bool) -> bool {
        if !self.horizon_scheme() {
            return false;
        }
        let (left, right) = if shift {
            (KeyCode::ShiftLeft, KeyCode::ShiftRight)
        } else {
            (KeyCode::ControlLeft, KeyCode::ControlRight)
        };
        self.android_custom_key_held(left) || self.android_custom_key_held(right)
    }

// ---- methods ----
    pub(super) fn android_keypad_menu_key(
        &mut self,
        events: &dyn super::event_loop::EventLoopCtl,
        event: &winit::event::KeyEvent,
    ) -> Result<bool> {
        if !self.pad_in_menu()
            || self.touch.keyboard
            || self.ui.options.selected_key.is_some()
        {
            return Ok(false);
        }
        let Some(key) = horizon_client::keypad::resolve_key(event.physical_key, &event.logical_key)
        else {
            return Ok(false);
        };
        // Menu arrows work independently of the optional gameplay keypad profile.
        let arrow = matches!(
            key,
            KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::ArrowLeft | KeyCode::ArrowRight
        );
        if !self.settings.keypad.enabled && !arrow && !self.pad.menu.keyboard_focus {
            return Ok(false);
        }
        if !self.settings.keypad.enabled
            && !matches!(
                key,
                KeyCode::ArrowUp
                    | KeyCode::ArrowDown
                    | KeyCode::ArrowLeft
                    | KeyCode::ArrowRight
                    | KeyCode::Enter
                    | KeyCode::NumpadEnter
                    | KeyCode::Space
                    | KeyCode::Escape
            )
        {
            return Ok(false);
        }
        let Some(button) = self.settings.keypad.menu_button(key) else {
            return Ok(false);
        };
        if !event.repeat {
            self.pad.menu.keyboard_focus = true;
            self.pad.in_use = true;
            self.pad_menu_button(events, button, event.state == ElementState::Pressed)?;
        }
        Ok(true)
    }

    /// A freely moving physical pointer can use the same utility buttons as touch.
    pub(super) fn android_aux_mouse(
        &mut self,
        events: &dyn super::event_loop::EventLoopCtl,
        id: WindowId,
        event: &WindowEvent,
    ) -> bool {
        const POINTER: u64 = u64::MAX - 2;
        if !self.settings.keypad.enabled || (self.captured && (self.input_mode == InputMode::KeyboardMouse)) {
            return false;
        }
        let WindowEvent::MouseInput {
            device_id,
            state,
            button: MouseButton::Left,
            ..
        } = event
        else {
            return false;
        };
        if *device_id == DeviceId::dummy() {
            return false;
        }
        let Some(batch) = self.gui_scale() else {
            return false;
        };
        let point = [self.cursor.0, self.cursor.1];
        let normalized = [point[0] / batch.width, point[1] / batch.height];
        // What the pad has under the pointer, and what it holds: touches are
        // position-dependent and change with the layout, ownership survives
        // screen transitions (a held press must complete before the pad
        // frees it, see `aux_mouse_phase`).
        let touched = self
            .pad
            .horizon
            .ui
            .action(batch.width, batch.height, point, self.pad_in_menu())
            .is_some()
            || (!self.pad_in_menu() && self.pad.horizon.touch.auxiliary_at(normalized));
        let owned = self.pad.horizon.ui.owns_key(POINTER)
            || self.pad.horizon.ui.owns_mouse(POINTER)
            || self.pad.horizon.touch.owns_pointer(POINTER);
        let Some(phase) = horizon_client::touch_ui::aux_mouse_phase(
            *state == ElementState::Pressed,
            owned,
            touched,
        ) else {
            return false;
        };
        let touch = Touch {
            device_id: *device_id,
            phase,
            location: winit::dpi::PhysicalPosition::new(
                f64::from(point[0] * batch.scale),
                f64::from(point[1] * batch.scale),
            ),
            force: None,
            id: POINTER,
        };
        self.handle_window_event(events, id, WindowEvent::Touch(touch), InputOrigin::User);
        true
    }

    pub(super) fn android_custom_key_held(&self, key: KeyCode) -> bool {
        self.pad
            .horizon
            .touch
            .held_custom_tokens()
            .chain(self.pad.horizon.late_custom_releases.iter().map(String::as_str))
            .any(|token| {
                if let Some(mapping) = token.strip_prefix("action:") {
                    horizon_client::keybinds::key_code(
                        horizon_client::keybinds::KeyBinds::default_of(mapping),
                    ) == Some(key)
                } else {
                    horizon_client::keybinds::key_code(token).is_some_and(|physical| {
                        physical == key
                            || self
                                .settings
                                .keys
                                .translate_all(physical)
                                .contains(&Ok(key))
                    })
                }
            })
    }

    pub(super) fn android_custom_mouse_held(&self, button: MouseButton) -> bool {
        self.pad
            .horizon
            .touch
            .held_custom_tokens()
            .chain(self.pad.horizon.late_custom_releases.iter().map(String::as_str))
            .any(|token| {
                if let Some(mapping) = token.strip_prefix("action:") {
                    horizon_client::keybinds::mouse_button(
                        horizon_client::keybinds::KeyBinds::default_of(mapping),
                    ) == Some(button)
                } else if horizon_client::keybinds::mouse_button(token) == Some(button) {
                    true
                } else {
                    horizon_client::keybinds::key_code(token).is_some_and(|physical| {
                        self.settings
                            .keys
                            .translate_all(physical)
                            .contains(&Err(button))
                    })
                }
            })
    }

    pub(super) fn android_custom_events(&mut self) {
        for (name, down) in self.pad.horizon.touch.key_events() {
            self.android_custom_token_edge(&name, down);
        }
    }

    pub(super) fn android_custom_token_edge(&mut self, token: &str, down: bool) {
        let ticking = self.title.is_none()
            && self.screen == Screen::Playing
            && !self.pause
            && !self.pad_in_menu();
        if down {
            if let Some(i) = self
                .pad
                .horizon
                .late_custom_releases
                .iter()
                .position(|t| t == token)
            {
                let prev = self.pad.horizon.late_custom_releases.remove(i);
                self.android_dispatch_custom_token(&prev, false);
            }
            self.pad
                .horizon
                .custom_pressed_at
                .insert(token.to_owned(), self.ticks);
        } else if ticking && self.pad.horizon.custom_pressed_at.get(token) == Some(&self.ticks) {
            if !self.pad.horizon.late_custom_releases.iter().any(|t| t == token) {
                self.pad.horizon.late_custom_releases.push(token.to_owned());
            }
            return;
        }
        self.android_dispatch_custom_token(token, down);
    }

    pub(super) fn android_custom_token_tap(&mut self, token: &str) {
        self.android_custom_token_edge(token, true);
        if token != "mouse.virtual.toggle"
            && token != "mouse.scroll.up"
            && token != "mouse.scroll.down"
        {
            self.android_custom_token_edge(token, false);
        }
    }

    pub(super) fn android_dispatch_custom_token(&mut self, name: &str, down: bool) {
        self.last_input = Instant::now();
        self.pad.in_use = true;
        if down && !self.pad_in_menu() && !self.captured && self.focused {
            self.capture(true);
        }
        if self.screen == Screen::Options && self.options_waiting_for_key() {
            if !down {
                return;
            }
            if let Some(mapping) = name.strip_prefix("action:") {
                let default = horizon_client::keybinds::KeyBinds::default_of(mapping);
                if let Some(key) = horizon_client::keybinds::key_code(default) {
                    let _ = self.options_key(key, None);
                } else if let Some(button) = horizon_client::keybinds::mouse_button(default) {
                    let _ = self.options_bind_mouse(button);
                }
                return;
            }
            if let Some(button) = horizon_client::keybinds::mouse_button(name) {
                let _ = self.options_bind_mouse(button);
                return;
            }
            if let Some(key) = horizon_client::keybinds::key_code(name) {
                let _ = self.options_key(key, None);
                return;
            }
        }
        if let Some(mapping) = name.strip_prefix("action:") {
            if mapping == "key.sneak" || mapping == "key.sprint" {
                let slot = usize::from(mapping == "key.sprint");
                let key = [KeyCode::ShiftLeft, KeyCode::ControlLeft][slot];
                if down {
                    self.pad.horizon.custom_keys.insert(key);
                    self.physical_keys.insert(key);
                    self.keys.insert(key);
                } else {
                    self.pad.horizon.custom_keys.remove(&key);
                    self.physical_keys.remove(&key);
                    self.keys.remove(&key);
                }
                self.toggles.0[slot].set_down(down, false);
                self.sync_toggle(slot);
                if slot == 1 && !down {
                    if let Some(player) = &mut self.player {
                        player.sprinting = false;
                    }
                }
                return;
            }
            if self.pad_in_menu()
                && down
                && mapping == "key.inventory"
                && matches!(self.screen, Screen::Inventory | Screen::Creative)
            {
                let _ = self.pad_back();
                return;
            }
            let default = horizon_client::keybinds::KeyBinds::default_of(mapping);
            let result = if let Some(key) = horizon_client::keybinds::key_code(default) {
                self.mapped_key(key, down, false)
            } else if let Some(button) = horizon_client::keybinds::mouse_button(default) {
                self.android_custom_mouse(button, down)
            } else {
                Ok(())
            };
            if let Err(error) = result {
                log::warn!("Custom touch action: {error}");
            }
            if down && self.screen == Screen::Chat {
                self.horizon_open_keyboard();
            }
            return;
        }
        if let Some(button) = horizon_client::keybinds::mouse_button(name) {
            if let Err(error) = self.android_custom_mouse(button, down) {
                log::warn!("Custom mouse button: {error}");
            }
            return;
        }
        if name == "mouse.scroll.up" || name == "mouse.scroll.down" {
            if down {
                let delta = if name == "mouse.scroll.up" { 8.0 } else { 1.0 };
                let next = horizon_client::mouse::ScrollWheelHandler::next_selection(
                    delta,
                    self.selected as i32,
                    9,
                );
                self.selected = next as usize;
            }
            return;
        }
        if name == "mouse.look" {
            return;
        }
        if name == "mouse.virtual.toggle" {
            if down {
                if self.pad_in_menu() || !self.pad.horizon.touch.settings.virtual_mouse_enabled {
                    self.pad.horizon.touch.settings.virtual_mouse_enabled =
                        !self.pad.horizon.touch.settings.virtual_mouse_enabled;
                    self.pad.horizon.ui.pointer_off = !self.pad.horizon.touch.settings.virtual_mouse_enabled;
                    if self.pad.horizon.touch.settings.virtual_mouse_enabled && !self.pad_in_menu() {
                        self.pad.horizon.ui.goy_in_game_mouse = true;
                    } else {
                        self.pad.horizon.ui.goy_in_game_mouse = false;
                    }
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                } else {
                    self.pad.horizon.ui.goy_in_game_mouse = !self.pad.horizon.ui.goy_in_game_mouse;
                }
                self.android_sync_pointer();
                self.android_touch_haptic(14);
            }
            return;
        }
        let Some(key) = horizon_client::keybinds::key_code(name) else {
            return;
        };
        if down {
            self.pad.horizon.custom_keys.insert(key);
            self.physical_keys.insert(key);
        } else {
            self.pad.horizon.custom_keys.remove(&key);
            self.physical_keys.remove(&key);
        }
        if down
            && horizon_client::keybinds::key_name(key)
                == Some(self.settings.keys.get("key.fullscreen"))
        {
            self.toggle_fullscreen();
            return;
        }
        if down && self.screenshot_shortcut(key, false) {
            return;
        }
        let action = if down {
            KeyAction::Press
        } else {
            KeyAction::Release
        };
        if self.debug_key(key, action) {
            return;
        }
        self.native_key_event(key, down, false);
        if self.screen == Screen::Options {
            if down {
                let _ = self.options_key(key, None);
            }
            return;
        }
        // Raw Shift and Control are the touch Sneak and Sprint buttons:
        // set their toggle state directly so both Hold and Toggle modes work reliably,
        // and also update `self.keys` so inventory Shift-click works.
        if matches!(
            key,
            KeyCode::ShiftLeft | KeyCode::ShiftRight | KeyCode::ControlLeft | KeyCode::ControlRight
        ) {
            if down {
                self.keys.insert(key);
            } else {
                self.keys.remove(&key);
            }
            let slot = usize::from(matches!(key, KeyCode::ControlLeft | KeyCode::ControlRight));
            self.toggles.0[slot].set_down(down, false);
            self.sync_toggle(slot);
            if slot == 1 && !down {
                if let Some(player) = &mut self.player {
                    player.sprinting = false;
                }
            }
            for mapped in self.settings.keys.translate_all(key) {
                match mapped {
                    Ok(code)
                        if !matches!(
                            code,
                            KeyCode::ShiftLeft
                                | KeyCode::ShiftRight
                                | KeyCode::ControlLeft
                                | KeyCode::ControlRight
                        ) =>
                    {
                        let _ = self.mapped_key(code, down, false);
                    }
                    Err(button) => {
                        let _ = self.android_custom_mouse(button, down);
                    }
                    _ => {}
                }
            }
            return;
        }
        if self.pad_in_menu() && down {
            if key == KeyCode::Escape {
                let _ = self.pad_back();
                return;
            }
            if key == KeyCode::KeyE && matches!(self.screen, Screen::Inventory | Screen::Creative) {
                let _ = self.pad_back();
                return;
            }
            if self.screen == Screen::Inventory
                && (self.merchant_key(key) || self.recipe_book_key(key))
            {
                return;
            }
            if self.screen == Screen::Chat && self.title.is_none() {
                self.keys.insert(key);
                self.chat_key_code(key, None);
                return;
            }
            if self.title.is_some() {
                let (w, h) = self.gui_size();
                let shift = self.has_shift_down();
                if let Some(t) = self.title.as_mut() {
                    match key {
                        KeyCode::Escape => {
                            t.escape();
                            self.save_title_options();
                        }
                        KeyCode::F5 => t.refresh_servers(),
                        KeyCode::Tab => t.tab_dir(shift, (w, h)),
                        KeyCode::ArrowLeft
                        | KeyCode::ArrowRight
                        | KeyCode::ArrowUp
                        | KeyCode::ArrowDown => t.arrow_key(key, (w, h)),
                        KeyCode::Backspace => self.title_edit(None, true),
                        _ => {}
                    }
                }
                return;
            }
        }
        if !down && self.screen == Screen::Chat && self.title.is_none() {
            self.keys.remove(&key);
        }
        if self.screen == Screen::Playing
            && down
            && horizon_client::keybinds::key_name(key)
                .is_some_and(|n| self.settings.keys.get("key.smoothCamera") == n)
        {
            self.smooth_camera = !self.smooth_camera;
        }
        let translated = self.settings.keys.translate_all(key);
        if translated.is_empty() {
            if down && matches!(self.screen, Screen::Inventory | Screen::Creative) {
                let _ = self.key_pressed(key);
            }
        } else {
            for mapped in translated {
                let result = match mapped {
                    Ok(code) => self.mapped_key(code, down, false),
                    Err(button) => self.android_custom_mouse(button, down),
                };
                if let Err(error) = result {
                    log::warn!("Custom touch key: {error}");
                }
            }
        }
        if down && matches!(key, KeyCode::KeyT | KeyCode::Slash) && self.screen == Screen::Chat {
            self.horizon_open_keyboard();
        }
    }

    pub(super) fn android_touch_haptic(&self, millis: u32) {
        if self.pad.horizon.touch.settings.ui_vibrate {
            self.horizon_vibrate(horizon_client::gamepad::Pulse {
                strong: 0.30,
                weak: 0.55,
                millis,
            });
        }
    }

    pub(super) fn android_set_virtual_mouse(&mut self, enabled: bool) {
        if !enabled {
            self.pad.horizon.ui.cancel_relative();
            for right in self.pad.horizon.ui.release_all_mouse() {
                let button = if right {
                    MouseButton::Right
                } else {
                    MouseButton::Left
                };
                let _ = self.android_custom_mouse(button, false);
            }
            self.pad.horizon.ui.virtual_mouse = false;
            return;
        }
        self.pad.horizon.ui.virtual_mouse = true;
        if let Some(b) = self.gui_scale() {
            self.pad
                .horizon
                .ui
                .sync_viewport_cursor(&mut self.cursor, [b.width, b.height]);
        }
    }

    /// State-driven Virtual Mouse visibility synchronization across MCPE and GoyControl.
    pub(super) fn android_sync_pointer(&mut self) {
        self.pad.horizon.ui.pointer_off = !self.pad.horizon.touch.settings.virtual_mouse_enabled;
        let in_menu = self.pad_in_menu();
        if let Some(b) = self.gui_scale() {
            self.pad
                .horizon
                .ui
                .sync_viewport_cursor(&mut self.cursor, [b.width, b.height]);
        }
        let want = self.pad.horizon.ui.compute_virtual_mouse_visibility(
            &self.pad.horizon.touch,
            in_menu,
            self.ui.hide_gui,
            self.touch_shown(),
            self.captured,
        );
        if self.pad.horizon.ui.virtual_mouse != want {
            self.android_set_virtual_mouse(want);
        }
    }

    fn android_custom_mouse(&mut self, button: MouseButton, down: bool) -> Result<()> {
        if self.screen == Screen::Options && self.options_waiting_for_key() {
            if down {
                return self.options_bind_mouse(button);
            }
            return Ok(());
        }
        if self.pad_in_menu() && !matches!(self.screen, Screen::Inventory | Screen::Creative) {
            let right = button == MouseButton::Right;
            if matches!(button, MouseButton::Left | MouseButton::Right) {
                let id = self.window.as_ref().map_or(WindowId::dummy(), |w| w.id());
                return self.android_menu_mouse(&super::event_loop::Noop, id, right, down);
            }
            return Ok(());
        }
        if !matches!(self.screen, Screen::Inventory | Screen::Creative) {
            if down && !self.captured && self.focused {
                self.capture(true);
            }
            self.extras_mouse(button, down);
            if down
                && self.settings.keys.get("key.smoothCamera")
                    == horizon_client::keybinds::mouse_name(button)
            {
                self.smooth_camera = !self.smooth_camera;
            }
            let mapped = self.settings.keys.translate_mouse_all(button);
            if mapped.is_empty() {
                return self.play_mouse(button, down);
            }
            for item in mapped {
                match item {
                    Err(b) => self.play_mouse(b, down)?,
                    Ok(k) => self.mapped_key(k, down, false)?,
                }
            }
            return Ok(());
        }
        if let Some(key) = self
            .settings
            .keys
            .container_mouse_key(button, self.creative)
        {
            if down && self.carried.is_none() {
                self.inventory_hotkey(key);
            }
            return Ok(());
        }
        let right = button == MouseButton::Right;
        if !matches!(button, MouseButton::Left | MouseButton::Right) {
            if self.settings.keys.get("key.pickItem")
                == horizon_client::keybinds::mouse_name(button)
            {
                if down {
                    self.inventory_pick_press();
                } else {
                    self.inventory_pick_release();
                }
                if let Some(container) = self.container {
                    self.container_changed(container);
                }
            }
            return Ok(());
        }
        if down {
            if self.screen == Screen::Creative {
                self.click_creative(right);
            } else {
                self.inventory_press(right);
            }
            if let Some(container) = self.container {
                self.container_changed(container);
            }
        } else {
            self.inventory_mouse_release(right);
            if self.screen == Screen::Creative && !right {
                self.creative_screen.release();
            }
            self.station_release();
            self.recipe_book_release();
            self.loom_release();
        }
        Ok(())
    }

    fn android_menu_mouse(
        &mut self,
        events: &dyn super::event_loop::EventLoopCtl,
        id: WindowId,
        right: bool,
        down: bool,
    ) -> Result<()> {
        let button = if right {
            MouseButton::Right
        } else {
            MouseButton::Left
        };
        if matches!(self.screen, Screen::Inventory | Screen::Creative) {
            self.android_custom_mouse(button, down)
        } else {
            let id = if id == WindowId::dummy() {
                self.window.as_ref().map_or(WindowId::dummy(), |w| w.id())
            } else {
                id
            };
            self.pad_synthetic_event(
                events,
                id,
                WindowEvent::MouseInput {
                    device_id: DeviceId::dummy(),
                    button,
                    state: if down {
                        ElementState::Pressed
                    } else {
                        ElementState::Released
                    },
                },
            );
            Ok(())
        }
    }

    pub(super) fn android_aux_touch(
        &mut self,
        t: &Touch,
        events: &dyn super::event_loop::EventLoopCtl,
        id: WindowId,
    ) -> Result<bool> {
        self.pad.horizon.ui.goy_mode = self.pad.horizon.touch.settings.control_style
            == horizon_client::horizon_touch::ControlStyle::GoyControl;
        if t.phase == TouchPhase::Ended
            && let Some((right, last)) = self.pad.horizon.ui.release_mouse(t.id)
        {
            if last {
                self.android_menu_mouse(events, id, right, false)?;
            }
            return Ok(true);
        }
        if t.phase == TouchPhase::Cancelled {
            self.pad.horizon.ui.cancel();
            self.android_release_touch_keys(true);
            return Ok(true);
        }
        if t.phase == TouchPhase::Ended && self.pad.horizon.ui.owns_key(t.id) {
            if let Some(key) = self.pad.horizon.ui.key_up(t.id) {
                self.android_key_up(key);
            }
            return Ok(true);
        }
        let size = if let Some(r) = self.renderer.as_ref() {
            winit::dpi::PhysicalSize::new(r.config.width, r.config.height)
        } else if let Some(window) = self.window.as_ref() {
            window.inner_size()
        } else {
            return Ok(false);
        };
        let mut b = HudBatch::new([size.width, size.height]);
        let p = [t.location.x as f32 / b.scale, t.location.y as f32 / b.scale];
        if !self.pad.horizon.ui.owns_relative(t.id)
            && !self.pad.horizon.ui.owns_gesture(t.id)
            && let Some(menu_act) = self.pad.horizon.ui.floating_menu_touch(
                &mut b,
                &self.gui.atlas,
                t.id,
                t.phase,
                p,
                &mut self.pad.horizon.touch.settings,
            )
        {
            use horizon_client::touch_ui::InGameMenuAction;
            self.pad.horizon.touch.clear();
            self.pad.horizon.touch.ignore(t.id);
            if t.phase == TouchPhase::Started {
                self.audio.play_button_click();
                self.android_touch_haptic(12);
            }
            match menu_act {
                InGameMenuAction::Close => {}
                InGameMenuAction::OpenControlEditor => {
                    self.pad.horizon.touch.open_editor();
                    self.android_sync_pointer();
                }
                InGameMenuAction::SetControlStyle(style) => {
                    self.android_release_touch_keys(true);
                    self.pad.horizon.ui.goy_in_game_mouse = false;
                    self.pad.horizon.touch.clear();
                    self.pad
                        .horizon
                        .touch
                        .settings
                        .switch_control_system(std::path::Path::new("."), style);
                    self.pad.horizon.ui.goy_mode =
                        style == horizon_client::horizon_touch::ControlStyle::GoyControl;
                    self.pad.horizon.ui.refresh_presets_cache(style);
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                    self.android_sync_pointer();
                }
                InGameMenuAction::SelectPreset(preset_id) => {
                    let cur_style = self.pad.horizon.touch.settings.control_style;
                    self.pad.horizon.touch.settings.select_preset(
                        std::path::Path::new("."),
                        cur_style,
                        &preset_id,
                    );
                    self.pad.horizon.ui.refresh_presets_cache(cur_style);
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                }
                InGameMenuAction::CopyPreset {
                    source_id,
                    name,
                    description,
                } => {
                    let cur_style = self.pad.horizon.touch.settings.control_style;
                    if let Ok(meta) = horizon_client::horizon_touch::Settings::copy_preset(
                        std::path::Path::new("."),
                        cur_style,
                        &source_id,
                        &name,
                        &description,
                    ) {
                        self.pad.horizon.touch.settings.select_preset(
                            std::path::Path::new("."),
                            cur_style,
                            &meta.id,
                        );
                        self.pad.horizon.ui.refresh_presets_cache(cur_style);
                        self.settings.touch = self.pad.horizon.touch.settings.clone();
                    }
                }
                InGameMenuAction::UpdatePresetMeta {
                    id: preset_id,
                    name,
                    description,
                } => {
                    let cur_style = self.pad.horizon.touch.settings.control_style;
                    let _ = self.pad.horizon.touch.settings.update_preset_meta(
                        std::path::Path::new("."),
                        cur_style,
                        &preset_id,
                        &name,
                        &description,
                    );
                    self.pad.horizon.ui.refresh_presets_cache(cur_style);
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                }
                InGameMenuAction::ApplyPreset(name) => {
                    self.pad.horizon.touch.settings.apply_preset(name);
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                }
                InGameMenuAction::LoadSavedProfile(idx) => {
                    let saved =
                        horizon_client::horizon_touch::Settings::list_profiles(std::path::Path::new("."));
                    if let Some(name) = saved.get(idx) {
                        self.pad.horizon.touch.settings = horizon_client::horizon_touch::Settings::load_profile(
                            std::path::Path::new("."),
                            name,
                        );
                        self.settings.touch = self.pad.horizon.touch.settings.clone();
                    }
                }
                InGameMenuAction::PresetVisibilityAll => {
                    self.pad.horizon.touch.settings.apply_visibility_preset_all();
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                }
                InGameMenuAction::CycleMovementMode => {
                    use horizon_client::horizon_touch::Mode;
                    self.pad.horizon.touch.settings.mode = match self.pad.horizon.touch.settings.mode {
                        Mode::Dpad => Mode::Crosshair,
                        Mode::Crosshair => Mode::Tap,
                        Mode::Tap => Mode::Dpad,
                    };
                    if self.pad.horizon.touch.settings.mode != Mode::Crosshair {
                        self.pad.horizon.touch.settings.split_controls = false;
                    }
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                }
                InGameMenuAction::CycleJoystickArea => {
                    self.pad.horizon.touch.settings.joystick_style.move_area =
                        self.pad.horizon.touch.settings.joystick_style.move_area.next();
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                }
                InGameMenuAction::ToggleSplitTouch => {
                    self.pad.horizon.touch.settings.toggle_split_touch();
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                }
                InGameMenuAction::ToggleInvertY => {
                    let next = !self.pad.horizon.touch.settings.invert_y;
                    self.pad.horizon.touch.settings.invert_y = next;
                    self.settings.invert_y = next;
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                    self.settings.save(&Self::options_path());
                }
                InGameMenuAction::ToggleLeftHanded => {
                    self.pad.horizon.touch.settings.left_handed = !self.pad.horizon.touch.settings.left_handed;
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                }
                InGameMenuAction::ToggleSprintMode => {
                    let next = !self.pad.horizon.touch.settings.sprint_toggle;
                    self.pad.horizon.touch.settings.sprint_toggle = next;
                    self.pad.horizon.touch.settings.toggle_sprint = next;
                    self.settings.toggle_sprint = next;
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                    self.settings.save(&Self::options_path());
                }
                InGameMenuAction::ToggleInventoryControls => {
                    self.pad.horizon.touch.settings.inventory_controls =
                        !self.pad.horizon.touch.settings.inventory_controls;
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                    self.android_sync_pointer();
                }
                InGameMenuAction::ToggleVibrate | InGameMenuAction::ToggleUiVibrate => {
                    let ui = matches!(menu_act, InGameMenuAction::ToggleUiVibrate);
                    let value = if ui {
                        &mut self.pad.horizon.touch.settings.ui_vibrate
                    } else {
                        &mut self.pad.horizon.touch.settings.vibrate
                    };
                    *value = !*value;
                    let next = *value;
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                    if next && ui {
                        self.android_touch_haptic(22);
                    } else {
                    }
                }
                InGameMenuAction::ToggleAutoJump => {
                    let next = !self.pad.horizon.touch.settings.auto_jump;
                    self.pad.horizon.touch.settings.auto_jump = next;
                    self.settings.auto_jump = next;
                    if let Some(player) = &mut self.player {
                        player.auto_jump_enabled = next;
                    }
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                    self.settings.save(&Self::options_path());
                }
                InGameMenuAction::ToggleGyro => {
                    self.pad.horizon.touch.settings.gyroscope = !self.pad.horizon.touch.settings.gyroscope;
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                }
                InGameMenuAction::ToggleVirtualMouse => {
                    let next = !self.pad.horizon.touch.settings.virtual_mouse_enabled;
                    self.pad.horizon.touch.settings.virtual_mouse_enabled = next;
                    self.pad.horizon.ui.pointer_off = !next;
                    if !next {
                        self.pad.horizon.ui.goy_in_game_mouse = false;
                    }
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                    self.android_sync_pointer();
                }
                InGameMenuAction::SetVirtualMouseSpeed(spd) => {
                    self.pad.horizon.touch.settings.virtual_mouse_sensitivity = spd.clamp(0.25, 3.00);
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                }
                InGameMenuAction::StepVirtualMouseSpeed(dir) => {
                    self.pad.horizon.touch.settings.virtual_mouse_sensitivity =
                        (self.pad.horizon.touch.settings.virtual_mouse_sensitivity + dir as f32 * 0.25)
                            .clamp(0.25, 3.00);
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                }
                InGameMenuAction::SetVirtualMousePointerSize(scale) => {
                    self.pad.horizon.touch.settings.virtual_mouse_pointer_scale = scale.clamp(0.5, 2.0);
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                }
                InGameMenuAction::SetGyroSensitivity(val) => {
                    self.pad.horizon.touch.settings.gyro_sensitivity = val.clamp(0.10, 3.00);
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                }
                InGameMenuAction::StepGyroSensitivity(dir) => {
                    self.pad.horizon.touch.settings.gyro_sensitivity =
                        (self.pad.horizon.touch.settings.gyro_sensitivity + dir as f32 * 0.20)
                            .clamp(0.20, 3.00);
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                }
                InGameMenuAction::SetTouchSensitivity(sens) => {
                    self.pad.horizon.touch.settings.sensitivity = sens.clamp(0.10, 3.00);
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                }
                InGameMenuAction::StepTouchSensitivity(dir) => {
                    self.pad.horizon.touch.settings.sensitivity =
                        (self.pad.horizon.touch.settings.sensitivity + dir as f32 * 0.15).clamp(0.10, 3.0);
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                }
                InGameMenuAction::SetButtonScale(val) => {
                    self.pad.horizon.touch.settings.button_scale = val.clamp(0.50, 2.00);
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                }
                InGameMenuAction::StepButtonScale(dir) => {
                    self.pad.horizon.touch.settings.button_scale =
                        (self.pad.horizon.touch.settings.button_scale + dir as f32 * 0.10).clamp(0.5, 2.0);
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                }
                InGameMenuAction::TriggerKey(key) => {
                    if !self.debug_key(key, KeyAction::Press) {
                        self.key_pressed(key)?;
                    }
                    self.debug_key(key, KeyAction::Release);
                    if matches!(key, KeyCode::KeyT | KeyCode::Slash) && self.screen == Screen::Chat
                    {
                        self.horizon_open_keyboard();
                    }
                }
                InGameMenuAction::OpenKeyboard => {
                    self.horizon_open_keyboard();
                }
                InGameMenuAction::ForceKill => {
                    let _ = self.pad.horizon.touch.settings.save();
                    std::process::exit(0);
                }
                InGameMenuAction::OpenLogs => {
                    // Logs are private to the app; the SAF provider shares the Reports
                    // folder the diagnostics exporter writes into.
                    if !self.horizon_open_reports() {
                        self.system_message(horizon_client::text::Text::literal(
                            "Could not open Reports. Logs are in the game folder's logs/ directory.",
                        ));
                    }
                }
                InGameMenuAction::EditPresetField(description) => {
                    let editing = if let Some(dlg) = self.pad.horizon.ui.preset_dialog.as_mut() {
                        dlg.editing_desc = description;
                        true
                    } else {
                        false
                    };
                    if editing {
                        self.horizon_open_keyboard();
                    }
                }
                InGameMenuAction::SetResolutionScale(pct) => {
                    let clamped = pct.clamp(25, 100);
                    let changed = self.pad.horizon.touch.settings.resolution_scale_percent != clamped
                        || self.settings.horizon.resolution_scale != clamped;
                    self.pad.horizon.touch.settings.resolution_scale_percent = clamped;
                    self.settings.horizon.resolution_scale = clamped;
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                    if let Some(r) = self.renderer.as_mut() {
                        r.set_world_post(
                            clamped as f32 / 100.0,
                            self.settings.horizon.fxaa,
                            self.settings.horizon.taa,
                            self.settings.horizon.upscaling.ratio(),
                        );
                    }
                    if changed {
                        let _ = self.pad.horizon.touch.settings.save();
                        self.settings.save(&Self::options_path());
                    }
                }
                InGameMenuAction::ToggleGameAnimations => {
                    let next = !self.pad.horizon.touch.settings.disable_game_animations;
                    self.pad.horizon.touch.settings.disable_game_animations = next;
                    horizon_client::ui_theme::motion::set_game_animations_disabled(next);
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                    if let Some(r) = self.renderer.as_mut() {
                        r.animated_textures = self.settings.horizon.animated_textures && !next;
                    }
                }
                InGameMenuAction::SendCustomInput(token) => {
                    self.android_sync_pointer();
                    self.android_custom_token_tap(&token);
                }
            }
            return Ok(true);
        }
        if t.phase == TouchPhase::Moved {
            return Ok(false);
        }
        if t.phase == TouchPhase::Started
            && !self.pad.horizon.ui.owns_relative(t.id)
            && !self.pad.horizon.ui.owns_gesture(t.id)
            && let Some(action) = self
                .pad
                .horizon
                .ui
                .action(b.width, b.height, p, self.pad_in_menu())
        {
            {
                self.pad.horizon.touch.clear();
                self.pad.horizon.touch.ignore(t.id);
                match action {
                    Action::LatchF3 => {
                        let was = self.pad.horizon.ui.key_held(KeyCode::F3);
                        self.pad.horizon.ui.f3_latched = !self.pad.horizon.ui.f3_latched;
                        let now = self.pad.horizon.ui.key_held(KeyCode::F3);
                        if !was && now {
                            self.physical_keys.insert(KeyCode::F3);
                            self.debug_key(KeyCode::F3, KeyAction::Press);
                        }
                        if was && !now {
                            self.android_key_up(KeyCode::F3);
                        }
                    }
                    Action::Pick => {
                        self.play_mouse(MouseButton::Middle, true)?;
                        self.play_mouse(MouseButton::Middle, false)?;
                        self.pad.horizon.ui.expanded = false;
                    }
                    Action::DebugCombo(key) => {
                        let held = self.pad.horizon.ui.key_held(KeyCode::F3);
                        if !held {
                            self.debug_key(KeyCode::F3, KeyAction::Press);
                        }
                        self.debug_key(key, KeyAction::Press);
                        self.debug_key(key, KeyAction::Release);
                        if !held {
                            self.debug_key(KeyCode::F3, KeyAction::Release);
                        }
                        self.pad.horizon.ui.expanded = false;
                    }
                    Action::MouseButton(right) => {
                        if self.pad.horizon.ui.press_mouse(t.id, right) {
                            self.android_menu_mouse(events, id, right, true)?;
                        }
                    }
                    Action::VirtualMouse => {
                        if matches!(self.screen, Screen::Inventory | Screen::Creative) {
                            // Cancel a prior drag without clicking/dropping at
                            // the toggle button, which is outside the inventory.
                            self.inv_clicks = Default::default();
                            self.inv_drag = None;
                            self.creative_screen.release();
                            self.loom_release();
                        }
                        for right in self.pad.horizon.ui.release_all_mouse() {
                            self.android_menu_mouse(events, id, right, false)?;
                        }
                        self.pad.horizon.ui.cancel();
                        let next = !self.pad.horizon.touch.settings.virtual_mouse_enabled;
                        self.pad.horizon.touch.settings.virtual_mouse_enabled = next;
                        self.pad.horizon.ui.pointer_off = !next;
                        let _ = self.pad.horizon.touch.settings.save();
                        self.settings.touch = self.pad.horizon.touch.settings.clone();
                        self.android_sync_pointer();
                    }
                    Action::More => self.pad.horizon.ui.expanded = !self.pad.horizon.ui.expanded,
                    Action::Keyboard => {
                        if self.touch.keyboard {
                            self.horizon_hide_keyboard();
                        } else {
                            self.horizon_open_keyboard();
                        }
                    }
                    Action::Key(key) => {
                        if key != KeyCode::F3 && !self.pad.horizon.ui.key_held(KeyCode::F3) {
                            self.pad.horizon.ui.expanded = false;
                        }
                        if key == KeyCode::Escape {
                            self.horizon_hide_keyboard();
                            self.pad_back()?;
                        } else if matches!(
                            key,
                            KeyCode::Tab | KeyCode::ShiftLeft | KeyCode::ControlLeft
                        ) {
                            let down = if key == KeyCode::Tab {
                                if self.physical_keys.remove(&key) {
                                    false
                                } else {
                                    self.physical_keys.insert(key);
                                    true
                                }
                            } else {
                                self.pad.horizon.ui.toggle_modifier(key)
                            };
                            if !self.pad_in_menu() {
                                self.mapped_key(key, down, false)?;
                            }
                        } else {
                            if !self.pad.horizon.ui.key_down(t.id, key) {
                                return Ok(true);
                            }
                            self.physical_keys.insert(key);
                            if !self.debug_key(key, KeyAction::Press) {
                                self.key_pressed(key)?;
                            }
                            if matches!(key, KeyCode::KeyT | KeyCode::Slash)
                                && self.screen == Screen::Chat
                            {
                                self.horizon_open_keyboard();
                            }
                        }
                    }
                }
            }
            return Ok(true);
        }
        let _ = (events, id);
        Ok(false)
    }

    pub(super) fn android_key_up(&mut self, key: KeyCode) {
        if self.pad.horizon.custom_keys.contains(&key) {
            return;
        }
        self.physical_keys.remove(&key);
        self.keys.remove(&key);
        self.debug_key(key, KeyAction::Release);
    }

    pub(super) fn android_release_touch_keys(&mut self, cancelled: bool) {
        self.pad.horizon.ui.clear_modifiers();
        if cancelled && matches!(self.screen, Screen::Inventory | Screen::Creative) {
            self.inv_clicks = Default::default();
            self.inv_drag = None;
            self.creative_screen.release();
            self.loom_release();
        }
        self.pad.horizon.ui.cancel_relative();
        for right in self.pad.horizon.ui.release_all_mouse() {
            let button = if right {
                MouseButton::Right
            } else {
                MouseButton::Left
            };
            if let Err(error) = self.android_custom_mouse(button, false) {
                log::warn!("Release virtual mouse: {error}");
            }
        }

        if cancelled {
            self.pad.horizon.hardware_keys.clear();
            self.pad.horizon.touch.clear();
            self.pad.horizon.aim = None;
            if self.pad.horizon.touch.editor.is_some() {
                self.pad.horizon.touch.cancel_editor();
                self.pause = false;
            }
        }
        self.android_custom_events();
        for key in self.pad.horizon.ui.release_keys() {
            if cancelled && key == KeyCode::F3 {
                self.physical_keys.remove(&key);
                self.debug_keys.modifier_down = false;
                self.debug_keys.used_debug_key_as_modifier = false;
                if self.screen == Screen::GameModeSwitcher {
                    self.close_gamemode_switcher(false);
                }
            } else {
                self.android_key_up(key);
            }
        }
    }

    pub(super) fn android_menu_touch(
        &mut self,
        t: &Touch,
        events: &dyn super::event_loop::EventLoopCtl,
        id: WindowId,
    ) {
        self.android_sync_pointer();
        if self.pad.horizon.ui.virtual_mouse {
            self.android_virtual_touch(t, events, id);
            return;
        }
        let Some(b) = self.gui_scale() else {
            return;
        };
        let p = [t.location.x as f32 / b.scale, t.location.y as f32 / b.scale];
        let direct = self.pad.horizon.ui.direct_pointer(t.id, p, t.phase);
        self.handle_window_event(
            events,
            id,
            WindowEvent::CursorMoved {
                device_id: t.device_id,
                position: t.location,
            },
            InputOrigin::User,
        );
        if direct {
            if t.phase != TouchPhase::Moved {
                self.handle_window_event(
                    events,
                    id,
                    WindowEvent::MouseInput {
                        device_id: t.device_id,
                        button: MouseButton::Left,
                        state: if t.phase == TouchPhase::Started {
                            ElementState::Pressed
                        } else {
                            ElementState::Released
                        },
                    },
                    InputOrigin::User,
                );
            }
            return;
        }
        match self
            .pad
            .horizon
            .ui
            .gesture(t.id, [t.location.x as f32, t.location.y as f32], t.phase)
        {
            Gesture::Tap => {
                let field = self.on_text_field() == Some(true);
                for state in [ElementState::Pressed, ElementState::Released] {
                    self.handle_window_event(
                        events,
                        id,
                        WindowEvent::MouseInput {
                            device_id: t.device_id,
                            button: MouseButton::Left,
                            state,
                        },
                        InputOrigin::User,
                    );
                }
                if field || self.sign_edit.is_some() {
                    self.horizon_open_keyboard();
                }
            }
            Gesture::Scroll(y) => self.handle_window_event(
                events,
                id,
                WindowEvent::MouseWheel {
                    device_id: t.device_id,
                    delta: MouseScrollDelta::LineDelta(0.0, y),
                    phase: TouchPhase::Moved,
                },
                InputOrigin::User,
            ),
            Gesture::None => {}
        }
    }

    pub(super) fn android_virtual_touch(
        &mut self,
        t: &Touch,
        events: &dyn super::event_loop::EventLoopCtl,
        id: WindowId,
    ) {
        let Some(b) = self.gui_scale() else {
            return;
        };
        let position = self.pad.horizon.ui.relative_touch(
            t.id,
            [t.location.x as f32 / b.scale, t.location.y as f32 / b.scale],
            t.phase,
            [self.cursor.0, self.cursor.1],
            [b.width, b.height],
            self.pad.horizon.touch.settings.virtual_mouse_sensitivity,
            !self.pad.horizon.ui.mouse_held(),
        );
        self.cursor = (position[0], position[1]);
        self.handle_window_event(
            events,
            id,
            WindowEvent::CursorMoved {
                device_id: t.device_id,
                position: winit::dpi::PhysicalPosition::new(
                    f64::from(position[0] * b.scale),
                    f64::from(position[1] * b.scale),
                ),
            },
            InputOrigin::User,
        );
        if t.phase == TouchPhase::Ended {
            self.android_virtual_mouse_clicks(events);
        }
    }

    pub(super) fn android_virtual_mouse_clicks(
        &mut self,
        events: &dyn super::event_loop::EventLoopCtl,
    ) {
        if !self.focused || !self.pad.horizon.ui.virtual_mouse {
            self.pad.horizon.ui.cancel_relative();
            for right in self.pad.horizon.ui.release_all_mouse() {
                if let Some(id) = self.window.as_ref().map(|w| w.id())
                    && let Err(error) = self.android_menu_mouse(events, id, right, false)
                {
                    log::warn!("Release virtual mouse: {error}");
                }
            }
            return;
        }
        if !self.pad_in_menu() {
            return;
        }
        let Some(right) = self.pad.horizon.ui.relative_click(true) else {
            return;
        };
        let button = if right {
            MouseButton::Right
        } else {
            MouseButton::Left
        };
        if matches!(self.screen, Screen::Inventory | Screen::Creative) {
            if let Err(e) = self
                .android_custom_mouse(button, true)
                .and_then(|()| self.android_custom_mouse(button, false))
            {
                log::warn!("Virtual mouse click: {e}");
            }
        } else if let Some(id) = self.window.as_ref().map(|w| w.id()) {
            for state in [ElementState::Pressed, ElementState::Released] {
                self.handle_window_event(
                    events,
                    id,
                    WindowEvent::MouseInput {
                        device_id: DeviceId::dummy(),
                        state,
                        button,
                    },
                    InputOrigin::User,
                );
            }
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(super) fn touch_shield_available(&self) -> bool {
        self.held()
            .into_iter()
            .chain(self.offhand.as_ref())
            .any(|s| self.blocks_attacks(s).is_some())
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(super) fn android_inventory_controls(&self) -> bool {
        self.settings.touch.inventory_controls
            && matches!(self.screen, Screen::Inventory | Screen::Creative)
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(super) fn touch_entity_accepts_use(&self) -> bool {
        if [
            self.boat_hit().map(|h| h.1),
            self.cart_hit().map(|h| h.1),
            self.armor_stand_hit().map(|h| h.1),
            self.frame_hit().map(|h| h.1),
        ]
        .into_iter()
        .flatten()
        .any(|distance| self.before_block(distance))
        {
            return true;
        }
        self.mob_hit()
            .filter(|(_, distance)| self.before_block(*distance))
            .is_some_and(|(i, _)| {
                self.remote_mob_takes(i, horizon_client::first_person::Hand::Main)
                    || self.remote_mob_takes(i, horizon_client::first_person::Hand::Off)
            })
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(super) fn touch_context_use(&self) -> bool {
        if self.touch_entity_accepts_use()
            || [
                self.boat_hit().map(|h| h.1),
                self.cart_hit().map(|h| h.1),
                self.armor_stand_hit().map(|h| h.1),
                self.frame_hit().map(|h| h.1),
            ]
            .into_iter()
            .flatten()
            .any(|d| self.before_block(d))
        {
            return true;
        }
        let Some(hit) = self.pick() else {
            return false;
        };
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        let b = self.res.behaviours();
        let data = b.data(state);
        let p = b.props;
        let table = &b.table;
        use horizon_world::level::classes::Class;
        data.chain.iter().any(|c| match c {
            Class::Door | Class::TrapDoor => {
                data.set_type.as_ref().is_some_and(|s| s.can_open_by_hand)
            }
            Class::Cake | Class::CandleCake => {
                self.creative
                    || self
                        .player
                        .as_ref()
                        .is_some_and(|p| p.health.food.food < 20)
            }
            Class::Composter => table.int(state, p.level) == 8,
            Class::CaveVines | Class::CaveVinesPlant => table.bool(state, p.berries),
            Class::SweetBerryBush => table.int(state, p.age) > 1,
            Class::Repeater | Class::Comparator | Class::DaylightDetector | Class::RedstoneWire => {
                self.mode != horizon_client::command::args::GameMode::Adventure
            }
            Class::FenceGate
            | Class::Button
            | Class::Lever
            | Class::Note
            | Class::Bell
            | Class::FlowerPot
            | Class::DragonEgg
            | Class::AbstractBed
            | Class::RespawnAnchor
            | Class::AbstractFurnace
            | Class::Anvil
            | Class::Barrel
            | Class::Beacon
            | Class::BrewingStand
            | Class::CartographyTable
            | Class::Chest
            | Class::Crafter
            | Class::CraftingTable
            | Class::Dispenser
            | Class::EnchantingTable
            | Class::EnderChest
            | Class::Grindstone
            | Class::Hopper
            | Class::Loom
            | Class::ShulkerBox
            | Class::Sign
            | Class::SmithingTable
            | Class::Stonecutter
            | Class::Lectern => true,
            _ => false,
        })
    }

    /// Touch fly/swim down and dismount are held even when ground sneaking
    /// is toggled. Any water counts, not only the sprint-swim pose: a toggled
    /// crouch there sinks the player (#71).
    pub(super) fn android_touch_descending(&self) -> bool {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            self.pad.horizon.touch.vertical_movement
                || self.pad.horizon.touch.swimming
                || self.pad.horizon.touch.mounted
                || self
                    .player
                    .as_ref()
                    .is_some_and(|p| p.fluids.was_touching_water)
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            false
        }
    }

    /// Whether a world hold should use the held item instead of mining.
    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(super) fn touch_projectile_crosshair(&self) -> bool {
        self.held()
            .is_some_and(|s| horizon_client::horizon_touch::projectile_crosshair(&s.item))
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(super) fn touch_item_held_use(&self) -> bool {
        self.held().is_some_and(|stack| {
            if horizon_client::horizon_touch::item_uses_on_tap(&stack.item) {
                return false;
            }
            #[cfg(feature = "mods")]
            if self.mods.mods.claims(&stack.item) {
                return true;
            }
            let Some(info) = self.items().get(&stack.item) else {
                return false;
            };
            remote::item_has_use(info)
                || info.bundle
                || matches!(
                    &*stack.item,
                    "minecraft:firework_rocket" | "minecraft:lead" | "minecraft:name_tag"
                )
                || info.block.is_some_and(|state| {
                    let block = self.res.facts().get(state);
                    ["CropBlock", "StemBlock", "NetherWartBlock", "CocoaBlock"]
                        .iter()
                        .any(|class| block.has_class(class))
                })
                || self.blocks_attacks(stack).is_some()
        })
    }

    /// A window event for the Horizon controls, before the game's own handling: true when they
    /// took it (Horizon Android's `handle_window_event`, its controls' part).
    pub(super) fn horizon_window_event(
        &mut self,
        event_loop: &dyn super::event_loop::EventLoopCtl,
        id: WindowId,
        event: &mut WindowEvent,
    ) -> bool {
        // Away from the screen, nothing the fingers or custom buttons held stays down.
        if let WindowEvent::Focused(false) = event {
            self.android_release_touch_keys(true);
        }
        if let WindowEvent::KeyboardInput { event, .. } = &mut *event
            && !matches!(event.physical_key, PhysicalKey::Code(_))
            && let Some(code) =
                horizon_client::keypad::resolve_key(event.physical_key, &event.logical_key)
        {
            event.physical_key = PhysicalKey::Code(code);
        }
        if let WindowEvent::KeyboardInput { event, .. } = &*event
            && event.state == ElementState::Pressed
            && let PhysicalKey::Code(key) = event.physical_key
            && self.pad.horizon.touch.capture_custom_key_code(key)
        {
            self.settings.touch = self.pad.horizon.touch.settings.clone();
            self.horizon_hide_keyboard();
            return true;
        }
        if let WindowEvent::KeyboardInput { event, .. } = &*event
            && let PhysicalKey::Code(key) = event.physical_key
        {
            if event.state == ElementState::Pressed {
                self.pad.horizon.hardware_keys.insert(key);
            } else {
                self.pad.horizon.hardware_keys.remove(&key);
            }
            if self.pad.horizon.custom_keys.contains(&key) {
                return true;
            }
        }
        if let WindowEvent::Touch(_) = &*event {
            self.horizon_input(true);
        }
        if !self.pad_in_menu()
            && matches!(
                horizon_client::android_runtime::input_mode(),
                horizon_client::android_gamepad::InputMode::Touch
                    | horizon_client::android_gamepad::InputMode::Controller
            )
            && matches!(&*event, WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && !matches!(event.logical_key, winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape | winit::keyboard::NamedKey::BrowserBack)))
        {
            return true;
        }
        if let WindowEvent::KeyboardInput { event, .. } = &*event {
            match self.android_keypad_menu_key(event_loop, event) {
                Ok(true) => return,
                Ok(false) => {}
                Err(error) => {
                    log::warn!("Keypad menu: {error}");
                    return true;
                }
            }
        }
        if self.android_aux_mouse(event_loop, id, &*event) {
            return true;
        }
        if let WindowEvent::KeyboardInput { event, .. } = &*event
            && let Some(key) =
                horizon_client::keypad::resolve_key(event.physical_key, &event.logical_key)
            && (!self.pad_in_menu() || event.state == ElementState::Released)
            && horizon_client::android_runtime::keypad_key(
                key,
                event.state == ElementState::Pressed,
            )
        {
            return true;
        }
        if matches!(&*event,WindowEvent::KeyboardInput {event,..} if event.state==ElementState::Pressed && event.logical_key==winit::keyboard::Key::Named(winit::keyboard::NamedKey::BrowserBack))
        {
            if self.touch.keyboard {
                self.horizon_hide_keyboard();
                return true;
            }
            if self.pad.horizon.touch.editor.is_some() {
                if self.pad.horizon.touch.request_editor_back() {
                    let _ = self.pad.horizon.touch.settings.save();
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                    self.pause = false;
                }
                return true;
            }
            self.pad.horizon.touch.clear();
            self.horizon_hide_keyboard();
            if let Err(e) = self.pad_back() {
                log::warn!("Android Back: {e}");
            }
            return true;
        }
        if self.pad.horizon.touch.editor.is_some()
            && matches!(&*event,WindowEvent::KeyboardInput{event,..}
            if event.state==ElementState::Pressed && event.physical_key==PhysicalKey::Code(KeyCode::Escape))
        {
            if self.touch.keyboard {
                self.horizon_hide_keyboard();
                return true;
            }
            if self.pad.horizon.touch.request_editor_back() {
                let _ = self.pad.horizon.touch.settings.save();
                self.settings.touch = self.pad.horizon.touch.settings.clone();
                self.pause = false;
            }
            return true;
        }
        if let WindowEvent::KeyboardInput { event, .. } = &*event
            && event.state == ElementState::Pressed
        {
            if self.pad.horizon.touch.editor.is_some() {
                if let PhysicalKey::Code(code) = event.physical_key
                    && self.pad.horizon.touch.capture_custom_key_code(code)
                {
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                    self.horizon_hide_keyboard();
                    return true;
                }
                if let Some(txt) = event.text.as_deref()
                    && self.pad.horizon.touch.capture_custom_text(txt)
                {
                    self.settings.touch = self.pad.horizon.touch.settings.clone();
                    self.horizon_hide_keyboard();
                    return true;
                }
            }
            if self.pad.horizon.ui.custom_input_picker != horizon_client::horizon_touch::PickerStep::None {
                if let PhysicalKey::Code(code) = event.physical_key
                    && let Some(token) = self.pad.horizon.ui.capture_custom_input_key(code)
                {
                    self.horizon_hide_keyboard();
                    self.android_custom_token_tap(&token);
                    return true;
                }
                if let Some(txt) = event.text.as_deref()
                    && let Some(token) = self.pad.horizon.ui.capture_custom_input_text(txt)
                {
                    self.horizon_hide_keyboard();
                    self.android_custom_token_tap(&token);
                    return true;
                }
            }
        }
        if let WindowEvent::KeyboardInput { event, .. } = &*event
            && event.state == ElementState::Pressed
            && (self
                .pad
                .horizon
                .touch
                .editor
                .as_ref()
                .is_some_and(|e| e.save_dialog_open)
                || self.pad.horizon.ui.preset_dialog.is_some())
        {
            if let PhysicalKey::Code(code) = event.physical_key {
                if matches!(code, KeyCode::Backspace | KeyCode::Enter | KeyCode::Escape) {
                    let out = horizon_client::osk::Output::Key(code);
                    horizon_client::touch_ui::apply_editor_or_preset_output(
                        self.pad.horizon.touch.editor.as_mut(),
                        self.pad.horizon.ui.preset_dialog.as_mut(),
                        &out,
                    );
                    if horizon_client::touch_ui::output_dismisses(&out) {
                        self.horizon_hide_keyboard();
                    }
                    return true;
                }
            }
            if let Some(txt) = event.text.as_deref()
                && !txt.is_empty()
            {
                let out = horizon_client::osk::Output::Text(txt.to_owned());
                horizon_client::touch_ui::apply_editor_or_preset_output(
                    self.pad.horizon.touch.editor.as_mut(),
                    self.pad.horizon.ui.preset_dialog.as_mut(),
                    &out,
                );
                if horizon_client::touch_ui::output_dismisses(&out) {
                    self.horizon_hide_keyboard();
                }
                return true;
            }
        }
        if let WindowEvent::Touch(touch) = &*event {
            use winit::event::TouchPhase;
            self.last_input = Instant::now();
            self.pad.menu.keyboard_focus = false;
            let size = if let Some(r) = self.renderer.as_ref() {
                winit::dpi::PhysicalSize::new(r.config.width, r.config.height)
            } else if let Some(w) = self.window.as_ref() {
                w.inner_size()
            } else {
                return true;
            };
            self.pad.horizon.touch.viewport(size.width, size.height);
            if self.pad.horizon.touch.editor.is_some() {
                if touch.phase == TouchPhase::Cancelled {
                    self.pad.horizon.touch.cancel_editor();
                    self.pause = false;
                } else {
                    let mut b = HudBatch::new([size.width, size.height]);
                    if horizon_client::touch_ui::touch_hud_editor(
                        &mut self.pad.horizon.touch,
                        &mut b,
                        &self.gui.atlas,
                        touch.id,
                        [
                            touch.location.x as f32 / size.width.max(1) as f32,
                            touch.location.y as f32 / size.height.max(1) as f32,
                        ],
                        touch.phase == TouchPhase::Started,
                        touch.phase == TouchPhase::Ended,
                    ) {
                        self.pause = false;
                    }
                }
                self.settings.touch = self.pad.horizon.touch.settings.clone();
                if std::mem::take(&mut self.pad.horizon.touch.clicked) {
                    self.audio.play_button_click();
                    self.android_touch_haptic(10);
                }
                return true;
            }
            match self.android_aux_touch(touch, event_loop, id) {
                Ok(true) => return,
                Ok(false) => {}
                Err(error) => {
                    log::error!("Android input: {error}");
                    return true;
                }
            }
            if touch.phase == TouchPhase::Cancelled {
                self.pad.horizon.touch.clear();
                return true;
            }
            let overlay_point = [
                touch.location.x as f32 / size.width.max(1) as f32,
                touch.location.y as f32 / size.height.max(1) as f32,
            ];
            let in_menu = self.pad_in_menu();
            let waiting_for_key = self.screen == Screen::Options && self.options_waiting_for_key();
            self.pad.horizon.touch.in_menu = in_menu && !waiting_for_key;
            self.pad.horizon.touch.in_inventory =
                matches!(self.screen, Screen::Inventory | Screen::Creative);
            let touch_controls_hidden = !self.touch_shown()
                && self.pad.horizon.touch.hud_opacity_scale <= 0.01
                && self.pad.horizon.touch.editor.is_none();
            if in_menu
                && self.pad.horizon.touch.consume_ignored(
                    touch.id,
                    matches!(touch.phase, TouchPhase::Ended | TouchPhase::Cancelled),
                )
            {
                return true;
            }
            let menu_custom_hit = !touch_controls_hidden
                && in_menu
                && !self.settings.keypad.enabled
                && (self.pad.horizon.touch.owns_pointer(touch.id)
                    || (touch.phase == TouchPhase::Started
                        && self.pad.horizon.touch.custom_at(overlay_point).is_some()));
            let inventory_control = !touch_controls_hidden
                && (self.android_inventory_controls() || self.pad.horizon.touch.in_menu)
                && (self.pad.horizon.touch.owns_pointer(touch.id)
                    || (touch.phase == TouchPhase::Started
                        && self.pad.horizon.touch.control_at_point(overlay_point)));
            if inventory_control
                && touch.phase == TouchPhase::Started
                && self.pad.horizon.touch.custom_at(overlay_point).is_none()
                && self.pad.horizon.touch.control_at(overlay_point)
                    == Some(horizon_client::horizon_touch::Control::Sneak)
            {
                self.pad.horizon.touch.inventory_shift ^= true;
                self.audio.play_button_click();
                self.android_touch_haptic(10);
                self.pad.horizon.touch.ignore(touch.id);
                return true;
            }
            if (menu_custom_hit || inventory_control)
                && self.pad.horizon.touch.menu_update(
                    touch.id,
                    overlay_point,
                    touch.phase == TouchPhase::Started,
                    matches!(touch.phase, TouchPhase::Ended | TouchPhase::Cancelled),
                )
            {
                self.android_custom_events();
                if std::mem::take(&mut self.pad.horizon.touch.clicked) {
                    self.audio.play_button_click();
                }
                return true;
            }
            if inventory_control
                && touch.phase == TouchPhase::Started
                && self.pad.horizon.touch.custom_at(overlay_point).is_none()
                && let Some(control) = self.pad.horizon.touch.control_at(overlay_point)
                && !matches!(
                    control,
                    horizon_client::horizon_touch::Control::Debug
                        | horizon_client::horizon_touch::Control::Settings
                        | horizon_client::horizon_touch::Control::Perspective
                )
            {
                self.pad.horizon.touch.ignore(touch.id);
                if matches!(
                    control,
                    horizon_client::horizon_touch::Control::Inventory
                        | horizon_client::horizon_touch::Control::Pause
                ) && let Err(error) = self.pad_back()
                {
                    log::warn!("Touch inventory close: {error}");
                }
                return true;
            }
            if !self.pad_in_menu() || (inventory_control && self.android_inventory_controls()) {
                if touch_controls_hidden
                    || (!self.touch_shown() && !self.settings.keypad.enabled)
                {
                    return true;
                }
                self.pad.in_use = true;
                if !self.pad_in_menu() && !self.captured && self.focused {
                    self.capture(true);
                }
                if touch.phase == TouchPhase::Started {
                    let batch = HudBatch::new([size.width, size.height]);
                    let point = [
                        touch.location.x as f32 / batch.scale,
                        touch.location.y as f32 / batch.scale,
                    ];
                    if let Some(slot) =
                        horizon_client::horizon_touch::hotbar_slot(batch.width, batch.height, point)
                        && !self.pad.horizon.touch.control_at_point([
                            touch.location.x as f32 / size.width.max(1) as f32,
                            touch.location.y as f32 / size.height.max(1) as f32,
                        ])
                    {
                        self.selected = slot;
                        self.pad.horizon.touch.ignore(touch.id);
                        return true;
                    }
                }
                if touch.phase == TouchPhase::Started {
                    self.pad.horizon.touch.projectile_crosshair = self.touch_projectile_crosshair();
                    self.pad.horizon.touch.hold_uses_item = self.touch_item_held_use();
                }
                let raw_delta = self.pad.horizon.touch.update(
                    touch.id,
                    [
                        touch.location.x as f32 / size.width.max(1) as f32,
                        touch.location.y as f32 / size.height.max(1) as f32,
                    ],
                    touch.phase == TouchPhase::Started,
                    matches!(touch.phase, TouchPhase::Ended | TouchPhase::Cancelled),
                );
                if self.pad.horizon.touch.has_pending_key_events() {
                    self.android_custom_events();
                }
                if self.pad.horizon.ui.virtual_mouse
                    && (raw_delta[0] != 0.0 || raw_delta[1] != 0.0)
                    && let Some(b) = self.gui_scale()
                {
                    let vm_sens = self
                        .pad
                        .horizon
                        .touch
                        .settings
                        .virtual_mouse_sensitivity
                        .clamp(0.25, 4.0);
                    self.cursor = (
                        (self.cursor.0 + raw_delta[0] * b.width * vm_sens).clamp(0.0, b.width),
                        (self.cursor.1 + raw_delta[1] * b.height * vm_sens).clamp(0.0, b.height),
                    );
                }
                let delta = horizon_client::horizon_touch::camera_delta_with_mouse(
                    raw_delta,
                    size.width,
                    size.height,
                    &self.pad.horizon.touch.settings,
                    self.settings.sensitivity,
                    self.settings.invert_x,
                    self.settings.invert_y,
                    self.zoom_turn_scale() as f32,
                );
                if delta[0] != 0.0 || delta[1] != 0.0 {
                    self.tutorial
                        .mouse(f64::from(delta[0]), f64::from(delta[1]));
                    self.turn_player(f64::from(delta[0]) / 0.15, f64::from(delta[1]) / 0.15);
                }
                if self.pad.horizon.touch.chat_requested {
                    self.audio.play_button_click();
                    self.pad.horizon.touch.clear();
                    if let Err(error) = self.key_pressed(KeyCode::KeyT) {
                        log::warn!("Touch chat: {error}");
                    }
                    self.horizon_open_keyboard();
                } else if self.pad.horizon.touch.settings_requested {
                    self.android_release_touch_keys(true);
                    self.audio.play_button_click();
                    self.open_options(false);
                } else if std::mem::take(&mut self.pad.horizon.touch.debug_requested) {
                    self.audio.play_button_click();
                    if !self.pad.horizon.ui.key_held(KeyCode::F3) {
                        use horizon_client::debug_keys::KeyAction;
                        self.debug_key(KeyCode::F3, KeyAction::Press);
                        self.debug_key(KeyCode::F3, KeyAction::Release);
                    }
                } else if std::mem::take(&mut self.pad.horizon.touch.perspective_requested) {
                    self.audio.play_button_click();
                    if let Err(error) = self.key_pressed(KeyCode::F5) {
                        log::warn!("Touch perspective: {error}");
                    }
                }
            } else {
                if !self.android_inventory_controls() {
                    self.pad.horizon.touch.clear_world_gesture();
                }
                self.android_sync_pointer();
                if !matches!(self.screen, Screen::Inventory | Screen::Creative) {
                    self.android_menu_touch(touch, event_loop, id);
                    return true;
                }
                if self.pad.horizon.ui.virtual_mouse {
                    self.android_virtual_touch(touch, event_loop, id);
                    return true;
                }
                self.handle_window_event(
                    event_loop,
                    id,
                    WindowEvent::CursorMoved {
                        device_id: touch.device_id,
                        position: touch.location,
                    },
                    InputOrigin::User,
                );
                if touch.phase != TouchPhase::Moved {
                    self.handle_window_event(
                        event_loop,
                        id,
                        WindowEvent::MouseInput {
                            device_id: touch.device_id,
                            button: MouseButton::Left,
                            state: if touch.phase == TouchPhase::Started {
                                ElementState::Pressed
                            } else {
                                ElementState::Released
                            },
                        },
                        InputOrigin::User,
                    );
                }
                if touch.phase == TouchPhase::Ended {
                    let b = self.gui_scale();
                    if let Some(b) = b {
                        let p = [
                            touch.location.x as f32 / b.scale,
                            touch.location.y as f32 / b.scale,
                        ];
                        if self.on_text_field() == Some(true) {
                            self.horizon_open_keyboard();
                        }
                    }
                }
            }
            return true;
        }
        false
    }

    /// The Horizon controls over the HUD: the editor, the virtual mouse's pointer, the controls,
    /// the GoyControl HUD and the floating menu (Horizon Android's frame).
    pub(super) fn horizon_draw(&mut self, hud: &mut HudBatch) {
        {
            self.pad.horizon.touch.auxiliary_only =
                self.settings.keypad.enabled && self.pad.horizon.touch.editor.is_none();
            self.pad.horizon.touch.update_visibility(
                self.touch_shown(),
                (self.frame_ticks / 20.0).clamp(0.0, 0.25),
            );
        }
        if self.pad.horizon.touch.editor.is_some() {
            let profile_name = self.pad.horizon.touch.settings.preset_name.clone();
            horizon_client::touch_ui::draw_hud_editor(
                &self.pad.horizon.touch,
                hud,
                &self.gui.atlas,
                &profile_name,
                None,
            );
        } else {
                self.android_sync_pointer();
            // Draw the virtual mouse pointer directly above the vanilla game/menu GUI layer
            // and strictly underneath our touch controls and floating menu overlays.
                if self.pad.horizon.ui.virtual_mouse {
                horizon_client::touch_ui::draw_pointer(
                    hud,
                    &self.gui.atlas,
                    self.cursor,
                    self.pad.horizon.touch.settings.virtual_mouse_pointer_scale,
                );
            }
            let in_menu = self.pad_in_menu();
            let waiting_for_key = self.screen == Screen::Options && self.options_waiting_for_key();
            let hud_in_menu = in_menu && !waiting_for_key;
            self.pad.horizon.touch.in_menu = hud_in_menu;
            self.pad.horizon.touch.in_inventory =
                matches!(self.screen, Screen::Inventory | Screen::Creative);
            self.pad.horizon.touch.goy_rendered_by_ui = true;
            if (in_menu || !self.ui.hide_gui)
                && (self.player.is_some() || self.settings.keypad.enabled)
                && (!hud_in_menu
                    || self.android_inventory_controls()
                    || self.pad.horizon.touch.in_inventory
                    || self.pad.horizon.touch.settings.layouts[self.pad.horizon.touch.settings.mode.index()]
                        .iter()
                        .any(|p| p.visible_in_menu)
                    || (0..self.pad.horizon.touch.settings.custom_buttons.len())
                        .any(|i| self.pad.horizon.touch.custom_visible_for_state(i, hud_in_menu)))
            {
                self.pad.horizon.touch.context_use =
                    self.pad.horizon.touch.settings.contextual_interact && self.touch_context_use();
                self.pad.horizon.touch.vertical_movement = self.player.as_ref().is_some_and(|p| p.flying);
                self.pad.horizon.touch.swimming = self.player.as_ref().is_some_and(|p| p.swimming);
                self.pad.horizon.touch.mounted = self.riding.is_some();
                self.pad.horizon.touch.shield_available = self.touch_shield_available();
                self.pad.horizon.touch.sneaking = self.keys.contains(&KeyCode::ShiftLeft);
                self.pad.horizon.touch.break_progress = self.touch_mining_progress();
                self.pad.horizon.touch.draw(hud, &self.gui.atlas);
            }
            if !self.settings.keypad.enabled && (in_menu || !self.ui.hide_gui) {
                horizon_client::touch_ui::draw_goy_hud(
                    &self.pad.horizon.touch,
                    hud,
                    &self.gui.atlas,
                    hud_in_menu,
                    self.pad.horizon.ui.virtual_mouse,
                );
            }
        }
        {
            self.android_sync_pointer();
            if self.pad.horizon.touch.editor.is_none() {
                self.pad.horizon.ui.goy_mode = self.pad.horizon.touch.settings.control_style
                    == horizon_client::horizon_touch::ControlStyle::GoyControl;
                self.pad
                    .horizon
                    .ui
                    .draw(hud, &self.gui.atlas, self.pad_in_menu());
                self.pad.horizon.ui.draw_floating_menu(
                    hud,
                    &self.gui.atlas,
                    &self.pad.horizon.touch.settings,
                );
            }
        }
    }
}
