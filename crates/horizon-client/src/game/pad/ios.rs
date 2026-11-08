//! iOS's controllers (this port): the pad Apple's GameController framework reads (DualSense,
//! DualShock 4, Xbox, Switch Pro, MFi), and its family for the button pictures. iOS keeps a pad's
//! state current on the main thread's run loop, the one the game runs on, so reading it is
//! reading properties (horizon-gamecontroller). The desktop's are gilrs' (pad/hardware.rs),
//! Android's come from its events (pad/android.rs).
use super::{super::Game, Pads};
use horizon_client::gamepad::{Button, GlyphStyle, PadState, Pulse, Rumble};
use serde_json::{Value, json};

pub(in crate::game) use super::no_router::{PadId, PadRouter};

/// The controller playing.
#[derive(Default)]
pub(super) struct Devices {
    /// Its product category ("DualSense", "Xbox One"...), once one played.
    active: Option<String>,
}

impl Pads {
    /// Splitscreen gives pads away; there are no gilrs pads here to close.
    pub(in crate::game) fn release_hardware(&mut self) {}

    /// The active pad's state; its family picks the button pictures.
    pub(super) fn read(&mut self) -> PadState {
        let Some(pad) = horizon_gamecontroller::active() else {
            return PadState::default();
        };
        if self.devices.active.as_deref() != Some(pad.category.as_str()) {
            log::info!("Controller in use: {}", pad.category);
            self.devices.active = Some(pad.category.clone());
        }
        self.style = family(&pad.category);
        state(&pad)
    }
}

/// The button pictures for iOS's product category.
fn family(category: &str) -> GlyphStyle {
    let category = category.to_ascii_lowercase();
    if ["dualsense", "dualshock", "playstation"]
        .iter()
        .any(|name| category.contains(name))
    {
        GlyphStyle::PlayStation
    } else if ["switch", "joy-con", "nintendo"]
        .iter()
        .any(|name| category.contains(name))
    {
        GlyphStyle::Nintendo
    } else {
        GlyphStyle::Xbox
    }
}

/// A GameController snapshot as the game's pad: sticks up positive, triggers 0..1.
fn state(pad: &horizon_gamecontroller::Snapshot) -> PadState {
    let mut state = PadState {
        buttons: 0,
        left: pad.left_stick,
        right: pad.right_stick,
        triggers: pad.triggers,
    };
    for (button, down) in [
        (Button::South, pad.a),
        (Button::East, pad.b),
        (Button::West, pad.x),
        (Button::North, pad.y),
        (Button::LeftBumper, pad.left_shoulder),
        (Button::RightBumper, pad.right_shoulder),
        (Button::Select, pad.options),
        (Button::Start, pad.menu),
        (Button::LeftStick, pad.left_stick_button),
        (Button::RightStick, pad.right_stick_button),
        (Button::DPadUp, pad.dpad_up),
        (Button::DPadDown, pad.dpad_down),
        (Button::DPadLeft, pad.dpad_left),
        (Button::DPadRight, pad.dpad_right),
    ] {
        state.set(button, down);
    }
    state
}

impl Game<'_> {
    pub(super) fn pad_output_off(&mut self) {}
    pub(super) fn pad_output_frame(&mut self) {}
    pub(in crate::game) fn pad_close_output(&mut self) {}

    /// iOS lends a game no pad motors without a Core Haptics engine of its own: the pad stays
    /// still (the phone's own taps are the touch controls', ios.rs).
    pub(super) fn pad_vibrate(&mut self, _: Rumble, _: Pulse) {}

    /// The hardware part of `pad status`.
    pub(super) fn pad_devices_status(&self) -> Value {
        let active = self.pad.devices.active.as_deref();
        let pads: Vec<Value> = horizon_gamecontroller::controllers()
            .iter()
            .map(|pad| {
                json!({
                    "name": pad.name,
                    "category": pad.category,
                    "gamepad": pad.gamepad,
                    "active": active == Some(pad.category.as_str()),
                })
            })
            .collect();
        json!({ "backend": true, "controllers": pads })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pads_family_picks_its_pictures() {
        assert_eq!(family("DualSense"), GlyphStyle::PlayStation);
        assert_eq!(family("DualShock 4"), GlyphStyle::PlayStation);
        assert_eq!(family("Switch Pro Controller"), GlyphStyle::Nintendo);
        assert_eq!(family("Xbox One"), GlyphStyle::Xbox);
        assert_eq!(family("MFi"), GlyphStyle::Xbox);
    }
}
