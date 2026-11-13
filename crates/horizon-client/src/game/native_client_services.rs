//! Host functions newer Horizon gives native mods (its `native_client_services.rs`), brought to
//! this base for the registry's mods that call them: Atlas's minimap faces and nine-slice
//! widgets. Each matches Horizon's at the revision those mods pin (f319701e8c), so a mod written
//! against it compiles here unchanged (IOS-SYNC.md, engine-edit ledger).
use super::Game;
use horizon_client::hud::{HudBatch, rgba};
use horizon_client::text::Text;

/// A listed player as a native mod sees them: their UUID, name and the skin the client draws.
#[allow(dead_code)]
pub struct NativeClientProfile {
    pub uuid: u128,
    pub name: String,
    pub skin: String,
}

// Called only by the native mods compiled in, so unused in a build without them.
#[allow(dead_code)]
impl Game<'_> {
    /// Player-list profiles with skin/cosmetic resolution through the client's normal cache.
    pub(super) fn native_client_profiles(&mut self) -> Vec<NativeClientProfile> {
        if !self.native_window() {
            return Vec::new();
        }
        let Some((players, _, _)) = self.remote_tab_players() else {
            return Vec::new();
        };
        players
            .into_iter()
            .map(|p| NativeClientProfile {
                uuid: p.uuid,
                skin: self.face_skin(p.uuid, &p.skin),
                name: p.name,
            })
            .collect()
    }

    /// Vanilla's nine-slice widget rendering. `color` is ARGB, as `fill` uses.
    pub(super) fn native_gui_nine_slice(
        &self,
        batch: &mut HudBatch,
        sprite: &str,
        rect: [f32; 4],
        color: u32,
    ) {
        let [a, r, g, b] = color.to_be_bytes();
        horizon_client::ui::nine_slice(
            batch,
            &self.gui.atlas,
            sprite,
            rect[0],
            rect[1],
            rect[2],
            rect[3],
            rgba(r, g, b, a),
        );
    }

    /// A mod's chat line or command (`/` first), sent as `send_chat` sends it
    /// (ClientPacketListener.sendChat / sendCommand), to any server: no chat history, and no
    /// client chat event. An error when the player isn't in a server's play state or the Chat
    /// option blocks the message.
    pub(super) fn native_send_chat(&mut self, message: &str) -> Result<(), String> {
        let Some(remote) = self.remote.as_ref() else {
            return Err("not connected to a server".into());
        };
        // A server that reconfigures the player has them out of the play state.
        if !remote.joined || remote.reconfiguring.is_some() {
            return Err("the player has not joined yet".into());
        }
        let message = message.trim();
        if message.is_empty() {
            return Err("the message is empty".into());
        }
        if !self.chat_abilities().allows(message) {
            return Err("the Chat option blocks this message".into());
        }
        self.send_chat(message);
        Ok(())
    }

    /// Save a mod binding, or restore its default with None. No write when unchanged.
    pub(super) fn native_set_mod_key(
        &mut self,
        mod_id: &str,
        name: &str,
        key: Option<&str>,
    ) -> bool {
        if !self.native_window()
            || mod_id.is_empty()
            || name.is_empty()
            || mod_id.len() > 256
            || name.len() > 256
            || key.is_some_and(|k| k.len() > 512)
        {
            return false;
        }
        let old = self
            .settings
            .mods
            .keys
            .get(mod_id)
            .and_then(|keys| keys.get(name))
            .map(String::as_str);
        if old == key {
            return false;
        }
        if let Some(key) = key {
            self.settings
                .mods
                .keys
                .entry(mod_id.into())
                .or_default()
                .insert(name.into(), key.into());
        } else if let Some(keys) = self.settings.mods.keys.get_mut(mod_id) {
            keys.remove(name);
        }
        self.settings.mods.save(&Self::game_dir());
        true
    }

    /// Set this client's action bar, with the normal timer; no server command or packet.
    pub(super) fn native_client_action_bar(&mut self, text: Text, animate: bool) {
        if self.native_window() {
            self.titles.set_overlay(text, animate);
        }
    }

    /// A mod screen's ask for the blurred background behind it. Here the screens a mod draws
    /// over (Options, the pause menu) already blur what is behind them as vanilla's do
    /// (`Game::menu_blur`), so there is nothing more to apply: false.
    pub(super) fn native_apply_screen_blur(&self) -> bool {
        false
    }
}
