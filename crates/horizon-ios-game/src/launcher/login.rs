//! The launcher's account panel, over Starling's own account list (horizon_client::accounts):
//! the same profiles the title screen's account button keeps, so either place adds, removes and
//! picks them, and the game plays as the one picked.
use super::pages::{Action, avatar_head, button};
use horizon_client::accounts::{self, State};
use horizon_client::android_control_keyboard::Output;
use horizon_client::android_control_theme::{Face, Icon, Ui, color};
use starling_account::{Account, Player, Provider};

fn login_surface() -> u32 {
    horizon_client::android_control_theme::active_theme().surface
}
fn login_surface_high() -> u32 {
    horizon_client::android_control_theme::active_theme().surface_bright
}
fn login_border() -> u32 {
    horizon_client::android_control_theme::active_theme().border
}
fn login_text() -> u32 {
    horizon_client::android_control_theme::active_theme().text_primary
}
fn login_muted() -> u32 {
    horizon_client::android_control_theme::active_theme().text_muted
}
fn login_accent() -> u32 {
    horizon_client::android_control_theme::active_theme().accent
}
fn login_accent_soft() -> u32 {
    horizon_client::android_control_theme::active_theme().accent_soft
}

pub(super) struct Login {
    pub choosing: bool,
    pub providers: bool,
    pub offline: bool,
    /// The Microsoft and Ely.by profiles Starling keeps.
    pub saved_accounts: Vec<Account>,
    pub account_offset: usize,
    pub ely: bool,
    pub username: String,
    pub password: String,
    pub otp: String,
    pub visible: bool,
    /// An account plays (not an offline profile).
    pub signed_in: bool,
    /// The account that plays, by name; none while an offline profile does.
    pub profile: Option<String>,
    /// A sign-in or a change of profile is under way.
    busy: bool,
    auto_open_device: bool,
    /// Microsoft's code and where to enter it, while it waits for the player.
    code: Option<(String, String)>,
    message: String,
}
impl Login {
    pub fn new() -> Self {
        let mut login = Self {
            choosing: false,
            providers: false,
            offline: false,
            saved_accounts: vec![],
            account_offset: 0,
            ely: false,
            username: String::new(),
            password: String::new(),
            otp: String::new(),
            visible: false,
            signed_in: false,
            profile: None,
            busy: false,
            auto_open_device: false,
            code: None,
            message: String::new(),
        };
        // A saved sign-in renews now, as at the game's start: the panel shows who plays.
        accounts::resume();
        login.refresh_status();
        login
    }
    pub(super) fn refresh_status(&mut self) {
        let profiles = accounts::profiles();
        let account = match profiles.player() {
            Some(Player::Account(account)) => Some(account),
            _ => None,
        };
        self.saved_accounts = profiles.accounts;
        self.profile = account.as_ref().map(|a| a.name.clone());
        self.signed_in = account.is_some();
    }
    fn provider_name(&self) -> &'static str {
        self.profile
            .as_deref()
            .and_then(|name| self.saved_accounts.iter().find(|a| a.name == name))
            .map_or("Microsoft", |a| match a.provider {
                Provider::Ely => "Ely.by",
                Provider::Microsoft => "Microsoft",
            })
    }

    pub fn status(&self, offline_name: &str) -> String {
        match &self.profile {
            Some(name) => format!("Signed in as {name} · {}", self.provider_name()),
            _ if self.pending() => "Signing in...".into(),
            _ if !offline_name.trim().is_empty() => format!("Offline: {offline_name}"),
            _ => "Offline / Sign in".into(),
        }
    }

    pub fn pending(&self) -> bool {
        self.busy
    }
    pub fn hide(&mut self) {
        self.password.clear();
        self.otp.clear();
        self.visible = false;
    }
    pub fn close(&mut self) {
        self.password.clear();
        self.otp.clear();
        self.auto_open_device = false;
        if self.busy {
            accounts::cancel();
        }
        self.busy = false;
        self.code = None;
        self.visible = false;
    }
    /// Microsoft's sign-in, its page opened as soon as the code is there.
    pub fn start(&mut self) {
        self.start_device();
        self.auto_open_device = true;
    }
    /// Microsoft's sign-in by its code: the player opens the page.
    pub fn start_device(&mut self) {
        self.close();
        self.ely = false;
        self.choosing = false;
        self.visible = true;
        self.message.clear();
        accounts::sign_in();
        self.busy = true;
    }
    /// The profile `key` plays from now on (an account's).
    pub fn select(&mut self, key: String) {
        self.message.clear();
        accounts::select(key);
        self.busy = true;
    }
    /// The offline profile `name` plays, added if Starling does not keep it yet.
    pub fn select_offline(&mut self, name: &str) {
        self.message.clear();
        let known = accounts::profiles().offline.iter().any(|n| n == name);
        if known {
            accounts::select(format!("offline:{name}"));
        } else {
            accounts::add_offline(name.to_owned());
        }
        self.busy = true;
    }
    /// Follows Starling's account state. True when a sign-in or a change just finished, so views
    /// can show the profile that plays now (also when the panel was closed meanwhile).
    pub fn poll(&mut self) -> bool {
        let was_busy = self.busy;
        match accounts::state() {
            State::Starting | State::Progress(_) | State::Resuming => self.busy = true,
            State::Code { code, link } => {
                self.busy = true;
                self.code = Some((code, link));
            }
            State::SignedIn(account) => {
                if was_busy {
                    self.message = format!("Signed in as {}", account.name);
                }
                self.busy = false;
            }
            State::OfflineProfile(_) | State::Offline => self.busy = false,
            State::Failed(error) => {
                if was_busy {
                    self.message = error;
                }
                self.busy = false;
            }
        }
        if !self.busy {
            self.code = None;
        }
        if self.auto_open_device && self.code.is_some() {
            self.auto_open_device = false;
            self.open_link();
        }
        let completed = was_busy && !self.busy;
        if completed {
            self.refresh_status();
        }
        completed
    }
    pub fn start_ely(&mut self) {
        self.message.clear();
        accounts::sign_in_ely(
            self.username.clone(),
            std::mem::take(&mut self.password),
            std::mem::take(&mut self.otp),
        );
        self.busy = true;
    }
    pub fn field(&self, name: &str) -> Option<&str> {
        match name {
            "ely_username" => Some(&self.username),
            "ely_password" => Some(&self.password),
            "ely_otp" => Some(&self.otp),
            _ => None,
        }
    }
    pub fn edit(&mut self, field: Option<&str>, output: &Output) -> bool {
        let value = match field {
            Some("ely_username") => &mut self.username,
            Some("ely_password") => &mut self.password,
            Some("ely_otp") => &mut self.otp,
            _ => return false,
        };
        match output {
            Output::Text(text) if value.len() + text.len() <= 1024 => {
                value.push_str(text);
            }
            Output::Backspace => {
                value.pop();
            }
            _ => {}
        }
        true
    }
    /// Signs the profile that plays out: Starling forgets it, the others stay.
    pub fn sign_out(&mut self) {
        self.close();
        self.visible = true;
        self.message = match accounts::sign_out() {
            Ok(()) => "Signed out. Offline play is available.".into(),
            Err(e) => e,
        };
        self.busy = true;
    }
    pub fn copy_code(&mut self) {
        if let Some((code, _)) = &self.code {
            self.message = if crate::platform::copy_login_code(code) {
                "Code copied. Paste it on the Microsoft page.".into()
            } else {
                "Could not copy the code. Try again.".into()
            };
        }
    }
    pub fn open_link(&mut self) {
        if let Some((code, link)) = self.code.clone() {
            let mut url = url::Url::parse(&link).ok();
            if let Some(uri) = &mut url {
                uri.query_pairs_mut().append_pair("otc", &code);
            }
            let url = url.map_or(link, String::from);
            self.copy_code();
            if let Err(e) = crate::platform::open_login_url(&url) {
                self.message = e;
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn account_row(
        &self,
        ui: &mut Ui,
        targets: &mut Vec<([f32; 4], Action)>,
        rect: [f32; 4],
        name: &str,
        detail: &str,
        current: bool,
        action: Action,
    ) {
        let [x, y, w, h] = rect;
        ui.rounded(x, y, w, h, 14.0, login_surface());
        ui.border(
            x,
            y,
            w,
            h,
            14.0,
            if current {
                login_accent()
            } else {
                login_border()
            },
        );
        let avatar = (h - 12.0).clamp(30.0, 40.0);
        avatar_head(ui, x + 10.0, y + (h - avatar) * 0.5, avatar);
        let tx = x + avatar + 22.0;
        let fitted = ui.fit_text(Face::SemiBold, 14.0, name, (w - avatar - 56.0).max(60.0));
        ui.text(Face::SemiBold, 14.0, tx, y + 7.0, login_text(), &fitted);
        let detail = if current {
            format!("{detail} · Current")
        } else {
            detail.to_owned()
        };
        ui.text(
            Face::Regular,
            11.5,
            tx,
            y + h - 21.0,
            if current {
                login_accent()
            } else {
                login_muted()
            },
            &detail,
        );
        if current {
            ui.icon(
                Icon::Check,
                x + w - 30.0,
                y + (h - 18.0) * 0.5,
                18.0,
                login_accent(),
            );
        }
        targets.push(([x, y, w, h], action));
    }

    pub fn draw(
        &self,
        ui: &mut Ui,
        targets: &mut Vec<([f32; 4], Action)>,
        offline_name: &str,
        editing: Option<&str>,
        modal_anim: f32,
    ) {
        targets.clear();
        let modal_t = modal_anim.clamp(0.2, 1.0);
        let scale_in = 0.95 + 0.05 * modal_t;
        let w = (ui.w - 24.0).min(620.0) * scale_in;
        let h = (ui.h - 20.0).min(600.0) * scale_in;
        let x = (ui.w - w) * 0.5;
        let mut y = (ui.h - h) * 0.5 + (1.0 - modal_t) * 14.0;
        if editing.is_some()
            && let Some(bottom_px) = crate::native_ime::viewport_bottom()
        {
            let ime_top_dp = bottom_px as f32 / ui.density;
            if ime_top_dp < ui.h - 16.0 {
                y = y.min((ime_top_dp - h - 8.0).max(6.0));
            }
        }
        let theme = horizon_client::android_control_theme::active_theme();
        ui.fill(
            0.0,
            0.0,
            ui.w,
            ui.h,
            color::with_alpha(theme.bg_solid, 0.65 * modal_t),
        );
        ui.rounded(
            x + 2.0,
            y + 8.0,
            w,
            h,
            26.0,
            color::hex(0x000000, (90.0 * modal_t) as u8),
        );
        ui.rounded(x, y, w, h, 26.0, theme.bg_solid);
        ui.border(x, y, w, h, 26.0, login_border());

        let (title, subtitle): (&str, String) = if self.providers {
            ("Accounts", "Choose how you want to play".into())
        } else if self.choosing {
            let line = if let Some(name) = &self.profile {
                if self.signed_in {
                    format!("Signed in as {name} with {}", self.provider_name())
                } else {
                    format!("Sign in required for {name}")
                }
            } else {
                "Choose an account or play offline".into()
            };
            ("Your accounts", line)
        } else if self.ely {
            (
                "Ely.by account",
                "Credentials go only to authserver.ely.by".into(),
            )
        } else if self.offline {
            (
                "Offline profile",
                "A local name for single-player and LAN".into(),
            )
        } else {
            let line = if let Some(name) = &self.profile {
                if self.signed_in {
                    format!("Already signed in as {name}")
                } else {
                    format!("Sign in to use {name}")
                }
            } else {
                "Xbox and Minecraft profile".into()
            };
            ("Microsoft account", line)
        };
        if !self.providers {
            let back = [x + 16.0, y + 16.0, 38.0, 38.0];
            ui.rounded(
                back[0],
                back[1],
                back[2],
                back[3],
                12.0,
                login_surface_high(),
            );
            ui.icon(
                Icon::ArrowLeft,
                back[0] + 10.0,
                back[1] + 10.0,
                18.0,
                login_text(),
            );
            targets.push((back, Action::Account));
        }
        let close = [x + w - 54.0, y + 16.0, 38.0, 38.0];
        ui.rounded(
            close[0],
            close[1],
            close[2],
            close[3],
            12.0,
            login_surface_high(),
        );
        ui.icon(
            Icon::Close,
            close[0] + 11.0,
            close[1] + 11.0,
            16.0,
            login_text(),
        );
        targets.push((close, Action::CloseLogin));
        let text_x = if self.providers { x + 22.0 } else { x + 64.0 };
        ui.text(Face::SemiBold, 20.0, text_x, y + 14.0, login_text(), title);
        let fitted = ui.fit_text(
            Face::Regular,
            12.5,
            &subtitle,
            (x + w - 70.0 - text_x).max(70.0),
        );
        ui.text(
            Face::Regular,
            12.5,
            text_x,
            y + 42.0,
            login_muted(),
            &fitted,
        );
        ui.rounded(x + 20.0, y + 72.0, w - 40.0, 1.0, 0.5, login_border());

        let content_y = y + 88.0;
        let content_h = (y + h - 20.0 - content_y).max(60.0);
        let footer_y = y + h - 58.0;
        let gap = 8.0;
        let field_h = ((content_h - 116.0) / 3.0 - gap).clamp(42.0, 56.0);

        if self.providers {
            let gap = 12.0;
            let cw = ((w - 40.0 - gap) * 0.5).max(120.0);
            let ch = ((content_h - 54.0 - gap) * 0.5).clamp(84.0, 96.0);
            for (i, (label, note, action, mark)) in [
                (
                    "Microsoft",
                    "Xbox and Minecraft",
                    Action::MicrosoftAccount,
                    "M",
                ),
                ("Ely.by", "Alternative auth", Action::ElySignIn, "E"),
                (
                    "Offline",
                    "Play without signing in",
                    Action::OfflineAccount,
                    "O",
                ),
                (
                    "Saved accounts",
                    "Switch account",
                    Action::SavedAccounts,
                    "S",
                ),
            ]
            .into_iter()
            .enumerate()
            {
                let cx = x + 20.0 + (i % 2) as f32 * (cw + gap);
                let cy = content_y + (i / 2) as f32 * (ch + gap);
                ui.rounded(cx, cy, cw, ch, 16.0, login_surface());
                ui.border(cx, cy, cw, ch, 16.0, login_border());
                ui.rounded(cx + 14.0, cy + 14.0, 34.0, 34.0, 11.0, login_accent_soft());
                ui.text(
                    Face::SemiBold,
                    15.0,
                    cx + 23.0,
                    cy + 21.0,
                    login_accent(),
                    mark,
                );
                ui.text(
                    Face::SemiBold,
                    15.0,
                    cx + 14.0,
                    cy + 50.0,
                    login_text(),
                    label,
                );
                let note = ui.fit_text(Face::Regular, 11.5, note, cw - 28.0);
                ui.text(
                    Face::Regular,
                    11.5,
                    cx + 14.0,
                    cy + 68.0,
                    login_muted(),
                    &note,
                );
                targets.push(([cx, cy, cw, ch], action));
            }
            button(
                ui,
                targets,
                [x + 20.0, footer_y, w - 40.0, 42.0],
                "Close",
                Action::CloseLogin,
                false,
            );
            return;
        }

        if self.choosing {
            let row_h = ((footer_y - content_y - 26.0) / 4.0).clamp(36.0, 56.0);
            let offline_current = self.profile.is_none();
            let offline_label = if offline_name.trim().is_empty() {
                "Offline player"
            } else {
                offline_name
            };
            self.account_row(
                ui,
                targets,
                [x + 20.0, content_y, w - 40.0, row_h],
                offline_label,
                "Offline profile",
                offline_current,
                Action::SelectOffline,
            );
            for (row, (index, account)) in self
                .saved_accounts
                .iter()
                .enumerate()
                .skip(self.account_offset)
                .take(3)
                .enumerate()
            {
                let detail = if account.provider == Provider::Ely {
                    "Ely.by"
                } else {
                    "Microsoft"
                };
                let current = self.profile.as_deref() == Some(account.name.as_str());
                let cy = content_y + (row + 1) as f32 * (row_h + 6.0);
                self.account_row(
                    ui,
                    targets,
                    [x + 20.0, cy, w - 40.0, row_h],
                    &account.name,
                    detail,
                    current,
                    Action::SelectAccount(index),
                );
            }
            let nav = 36.0;
            button(
                ui,
                targets,
                [x + 20.0, footer_y, nav, 38.0],
                "<",
                Action::SavedPage(false),
                false,
            );
            button(
                ui,
                targets,
                [x + 20.0 + nav + 6.0, footer_y, nav, 38.0],
                ">",
                Action::SavedPage(true),
                false,
            );
            let after = x + 20.0 + 2.0 * (nav + 6.0) + 8.0;
            let rest = x + w - 20.0 - after;
            let add_w = (rest - 8.0) * 0.5;
            button(
                ui,
                targets,
                [after, footer_y, add_w, 38.0],
                "Add account",
                Action::AddAccount,
                false,
            );
            button(
                ui,
                targets,
                [after + add_w + 8.0, footer_y, rest - add_w - 8.0, 38.0],
                "Done",
                Action::CloseLogin,
                true,
            );
            return;
        }

        if self.ely {
            for (i, (name, label, value, secret, placeholder)) in [
                (
                    "ely_username",
                    "Username or email",
                    self.username.as_str(),
                    false,
                    "Username or email",
                ),
                (
                    "ely_password",
                    "Password",
                    self.password.as_str(),
                    true,
                    "Password",
                ),
                (
                    "ely_otp",
                    "Two-factor code",
                    self.otp.as_str(),
                    true,
                    "Optional",
                ),
            ]
            .into_iter()
            .enumerate()
            {
                let fy = content_y + i as f32 * (field_h + gap);
                let display = if secret {
                    "*".repeat(value.chars().count().min(40))
                } else {
                    value.to_owned()
                };
                let focused = editing == Some(name);
                ui.text_field(
                    x + 20.0,
                    fy,
                    w - 40.0,
                    field_h,
                    label,
                    &display,
                    placeholder,
                    focused,
                );
                targets.push(([x + 20.0, fy, w - 40.0, field_h], Action::Field(name)));
            }
            if editing.is_some_and(|field| field.starts_with("ely_")) {
                button(
                    ui,
                    targets,
                    [x + 20.0, footer_y, w - 40.0, 42.0],
                    "Done editing",
                    Action::FinishNameEdit,
                    true,
                );
                return;
            }
            let note_y = content_y + 3.0 * (field_h + gap) + 4.0;
            if !self.message.is_empty() {
                paragraph(
                    ui,
                    x + 20.0,
                    note_y,
                    w - 40.0,
                    &self.message,
                    login_text(),
                    2,
                );
            }
            button(
                ui,
                targets,
                [x + 20.0, footer_y, w - 40.0, 42.0],
                if self.pending() {
                    "Signing in..."
                } else {
                    "Sign in to Ely.by"
                },
                Action::SubmitEly,
                true,
            );
            button(
                ui,
                targets,
                [x + 20.0, footer_y - 48.0, w - 40.0, 36.0],
                "Back to accounts",
                Action::Account,
                false,
            );
            return;
        }

        if self.offline {
            let focused = editing == Some("name");
            ui.text_field(
                x + 20.0,
                content_y,
                w - 40.0,
                58.0,
                "Player name",
                offline_name,
                "Enter a name",
                focused,
            );
            targets.push(([x + 20.0, content_y, w - 40.0, 58.0], Action::Field("name")));
            let count = format!("{}/16 characters", offline_name.chars().count());
            ui.text(
                Face::Regular,
                11.5,
                x + 20.0,
                content_y + 66.0,
                login_muted(),
                &count,
            );
            ui.hint_line(
                x + 20.0,
                content_y + 88.0,
                w - 40.0,
                "This name is local; it is not an online identity.",
                login_muted(),
            );
            if focused {
                button(
                    ui,
                    targets,
                    [x + 20.0, footer_y, w - 40.0, 42.0],
                    "Done editing",
                    Action::FinishNameEdit,
                    true,
                );
                return;
            }
            button(
                ui,
                targets,
                [x + 20.0, footer_y, w - 40.0, 42.0],
                "Use offline account",
                Action::SelectOffline,
                true,
            );
            button(
                ui,
                targets,
                [x + 20.0, footer_y - 48.0, w - 40.0, 36.0],
                "Back to accounts",
                Action::Account,
                false,
            );
            return;
        }

        // Microsoft
        let card_h = 84.0;
        let card = [x + 20.0, content_y, w - 40.0, card_h];
        ui.rounded(card[0], card[1], card[2], card[3], 16.0, login_surface());
        ui.border(card[0], card[1], card[2], card[3], 16.0, login_border());
        let display_name = self.profile.as_deref().unwrap_or(offline_name);
        avatar_head(ui, card[0] + 14.0, card[1] + 14.0, 56.0);
        let name_x = card[0] + 84.0;
        let fitted = ui.fit_text(Face::SemiBold, 17.0, display_name, card[2] - 100.0);
        ui.text(
            Face::SemiBold,
            17.0,
            name_x,
            card[1] + 14.0,
            login_text(),
            &fitted,
        );
        let status = self.status(offline_name);
        let fitted = ui.fit_text(Face::Regular, 12.5, &status, card[2] - 100.0);
        ui.text(
            Face::Regular,
            12.5,
            name_x,
            card[1] + 40.0,
            if self.signed_in {
                login_accent()
            } else {
                login_muted()
            },
            &fitted,
        );
        ui.text(
            Face::Regular,
            11.0,
            name_x,
            card[1] + 60.0,
            login_muted(),
            if self.signed_in {
                "Microsoft · Connected"
            } else {
                "Microsoft · Not connected"
            },
        );

        let mut row_y = content_y + card_h + 12.0;
        if let Some((user_code, _)) = &self.code {
            let code_h = 84.0;
            ui.rounded(
                x + 20.0,
                row_y,
                w - 40.0,
                code_h,
                14.0,
                login_surface_high(),
            );
            ui.border(x + 20.0, row_y, w - 40.0, code_h, 14.0, login_border());
            ui.text(
                Face::Regular,
                11.0,
                x + 34.0,
                row_y + 12.0,
                login_muted(),
                "SIGN-IN CODE",
            );
            ui.text(
                Face::SemiBold,
                28.0,
                x + 34.0,
                row_y + 30.0,
                login_text(),
                user_code,
            );
            row_y += code_h + 10.0;
            let half = (w - 52.0) * 0.5;
            button(
                ui,
                targets,
                [x + 20.0, row_y, half, 38.0],
                "Copy code",
                Action::CopyLoginCode,
                false,
            );
            button(
                ui,
                targets,
                [x + 32.0 + half, row_y, half, 38.0],
                "Open browser",
                Action::LoginLink,
                false,
            );
            row_y += 46.0;
        } else {
            let text = if self.signed_in {
                "Your account is saved on this device. Sessions renew automatically."
            } else {
                "You sign in inside Microsoft's browser; this app never sees your password."
            };
            row_y = paragraph(ui, x + 20.0, row_y, w - 40.0, text, login_muted(), 2) + 8.0;
        }
        let message = if self.busy && self.code.is_none() {
            "Requesting a sign-in code..."
        } else {
            self.message.as_str()
        };
        if !message.is_empty() {
            row_y = paragraph(ui, x + 20.0, row_y, w - 40.0, message, login_text(), 2) + 4.0;
        }
        let (label, action) = if self.code.is_some() {
            ("Copy code and open browser", Action::LoginLink)
        } else if self.pending() {
            ("Restart browser sign-in", Action::SignIn)
        } else if self.signed_in {
            ("Switch Microsoft account", Action::SignIn)
        } else {
            ("Sign in with browser", Action::SignIn)
        };
        button(
            ui,
            targets,
            [x + 20.0, footer_y, w - 40.0, 42.0],
            label,
            action,
            true,
        );
        let half = (w - 52.0) * 0.5;
        button(
            ui,
            targets,
            [x + 20.0, footer_y - 48.0, half, 38.0],
            "Device code",
            Action::DeviceSignIn,
            false,
        );
        let (second_label, second_action) = if self.pending() {
            ("Cancel", Action::CancelLogin)
        } else if self.profile.is_some() {
            ("Sign out", Action::SignOut)
        } else {
            ("Back", Action::Account)
        };
        button(
            ui,
            targets,
            [x + 32.0 + half, footer_y - 48.0, half, 38.0],
            second_label,
            second_action,
            false,
        );
    }
}

fn paragraph(ui: &mut Ui, x: f32, y: f32, w: f32, text: &str, tone: u32, max_lines: usize) -> f32 {
    let limit = (w / 6.8).max(16.0) as usize;
    let mut line = String::new();
    let mut row = 0usize;
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + word.len() + 1 > limit {
            ui.text(Face::Regular, 12.5, x, y + row as f32 * 18.0, tone, &line);
            row += 1;
            line.clear();
            if row >= max_lines {
                break;
            }
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if row < max_lines && !line.is_empty() {
        ui.text(Face::Regular, 12.5, x, y + row as f32 * 18.0, tone, &line);
        row += 1;
    }
    y + row as f32 * 18.0
}
