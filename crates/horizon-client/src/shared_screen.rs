//! The pause menu's Friends button in a shared world (`game/shared_worlds.rs` fills it): who is
//! playing and who hosts, the host's Give Host and the answers to players who ask to host, and a
//! guest's Ask to Host. Drawn as Play with Friends is (`friends_screen.rs`).
use crate::{
    hud::{HudAtlas, HudBatch, rgba},
    multiplayer::wrap,
    text::Text,
    ui,
};

/// The English title, and what the screen's widgets are named by.
pub const TITLE: &str = "Shared World";

fn tr(key: &str, english: &str) -> String {
    crate::lang::get_or(&format!("horizon.shared.{key}"), english)
}

/// A button's job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// The host gives the world to this player (a UUID).
    GiveHost(String),
    /// The host lets this player (a UUID) host, as they asked.
    Accept(String),
    /// The host says no to this player (a UUID).
    Deny(String),
    /// A guest asks the host for the world.
    AskHost,
    /// The invite code that joins the host now, to the clipboard.
    CopyCode,
    Done,
}

/// What the screen shows.
#[derive(Clone, Debug, Default)]
pub struct View {
    pub world: String,
    /// What is true now, a line each: who hosts, what the host's storage is doing.
    pub lines: Vec<String>,
    /// Buttons above the footer, at most six.
    pub buttons: Vec<(String, Action, bool)>,
    /// The invite code that joins the host now (Play with Friends' code), once it is known.
    pub code: Option<String>,
    /// The code was just copied.
    pub copied: bool,
}

const WHITE: u32 = u32::MAX;
fn grey() -> u32 {
    rgba(170, 170, 170, 255)
}

struct Button {
    x: f32,
    y: f32,
    w: f32,
    label: String,
    action: Action,
    active: bool,
}

fn layout(
    atlas: &HudAtlas,
    (w, h): (f32, f32),
    view: &View,
) -> (Vec<(String, u32, f32)>, Vec<Button>) {
    let width = (w - 40.0).min(310.0);
    let cx = (w / 2.0).floor();
    let mut lines = Vec::new();
    let mut y = 36.0;
    for line in wrap(&view.world, width, atlas) {
        lines.push((line, WHITE, y));
        y += 10.0;
    }
    y += 4.0;
    for text in &view.lines {
        for line in wrap(text, width, atlas) {
            lines.push((line, grey(), y));
            y += 10.0;
        }
    }
    let mut buttons = Vec::new();
    // The code (Copy Code is beside Done, so that it always fits).
    if let Some(code) = &view.code {
        y += 4.0;
        lines.push((
            crate::lang::get_or("horizon.multiplayer.invite_code", "Invite Code"),
            WHITE,
            y,
        ));
        y += 10.0;
        for line in crate::friends_screen::code_lines(atlas, code, width) {
            lines.push((line, WHITE, y));
            y += 10.0;
        }
    }
    y += 6.0;
    for (label, action, active) in view.buttons.iter().take(6) {
        if y + 20.0 > h - 33.0 {
            break;
        }
        buttons.push(Button {
            x: cx - 100.0,
            y,
            w: 200.0,
            label: label.clone(),
            action: action.clone(),
            active: *active,
        });
        y += 24.0;
    }
    let done = if view.code.is_some() {
        buttons.push(Button {
            x: cx - 100.0,
            y: h - 27.0,
            w: 98.0,
            label: if view.copied {
                crate::lang::get_or("horizon.friends.copied", "Copied")
            } else {
                crate::lang::get_or("horizon.friends.copy_code", "Copy Code")
            },
            action: Action::CopyCode,
            active: true,
        });
        (cx + 2.0, 98.0)
    } else {
        (cx - 100.0, 200.0)
    };
    buttons.push(Button {
        x: done.0,
        y: h - 27.0,
        w: done.1,
        label: "gui.done".into(),
        action: Action::Done,
        active: true,
    });
    (lines, buttons)
}

fn label(key: &str) -> Text {
    if key.starts_with("gui.") {
        Text::translate(key, &[])
    } else {
        Text::literal(key)
    }
}

pub fn draw(batch: &mut HudBatch, atlas: &HudAtlas, cursor: (f32, f32), view: &View) {
    let (w, h) = (batch.width, batch.height);
    ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
    let title = Text::literal(tr("world_title", TITLE));
    let tw = atlas.font.width_text(&title);
    batch.text_styled(atlas, &title, ((w - tw) / 2.0).floor(), 15.0, WHITE, true);
    let (lines, buttons) = layout(atlas, (w, h), view);
    for (line, color, y) in lines {
        let lw = HudBatch::text_width(atlas, &line);
        batch.text(atlas, &line, ((w - lw) / 2.0).floor(), y, color, true);
    }
    for b in buttons {
        let hover = b.active && ui::hovered(cursor, b.x, b.y, b.w, 20.0);
        ui::button(
            batch,
            atlas,
            b.x,
            b.y,
            b.w,
            20.0,
            &label(&b.label),
            b.active,
            hover,
        );
    }
}

/// The button under a click, if it is active.
pub fn click(
    atlas: &HudAtlas,
    size: (f32, f32),
    cursor: (f32, f32),
    view: &View,
) -> Option<Action> {
    layout(atlas, size, view)
        .1
        .into_iter()
        .find(|b| b.active && ui::hovered(cursor, b.x, b.y, b.w, 20.0))
        .map(|b| b.action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_button_fits_above_the_footer_and_done_is_always_there() {
        let atlas =
            HudAtlas::build(std::path::Path::new("/nonexistent/horizon-test-assets")).unwrap();
        let view = View {
            world: "Island".into(),
            lines: vec!["You host this world.".into()],
            buttons: (0..9)
                .map(|i| {
                    (
                        format!("Give Host to P{i}"),
                        Action::GiveHost(i.to_string()),
                        true,
                    )
                })
                .collect(),
            ..View::default()
        };
        let (_, buttons) = layout(&atlas, (427.0, 240.0), &view);
        assert_eq!(buttons.last().unwrap().action, Action::Done);
        assert!(buttons.len() <= 7);
        for b in &buttons[..buttons.len() - 1] {
            assert!(b.y + 20.0 <= 240.0 - 33.0);
        }
        let done = buttons.last().unwrap();
        assert_eq!(
            click(&atlas, (427.0, 240.0), (done.x + 5.0, done.y + 5.0), &view),
            Some(Action::Done)
        );
    }

    #[test]
    fn the_code_shows_with_copy_code_first_whatever_else_is_there() {
        let atlas =
            HudAtlas::build(std::path::Path::new("/nonexistent/horizon-test-assets")).unwrap();
        let code = "0B33K-Q405Q-1Z5CV-X6NHY-R12N0-VK8T1-60N00-RA134-952PE-1HA08-910P0-K5M01-ST1NB-JDTB6-K4W1P-0";
        let view = View {
            world: "Island".into(),
            lines: vec![
                "You host this world. Here: Steve, Alex, Sam".into(),
                "Friends on other networks may play through bore.pub (slower): your router didn't open a port. Turning on UPnP in its settings fixes that. Direct: 1, bore.pub: 1.".into(),
            ],
            buttons: (0..7)
                .map(|i| {
                    (
                        format!("Give Host to P{i}"),
                        Action::GiveHost(i.to_string()),
                        true,
                    )
                })
                .collect(),
            code: Some(code.into()),
            copied: false,
        };
        for size in [(427.0, 240.0), (320.0, 240.0), (640.0, 360.0)] {
            let (lines, buttons) = layout(&atlas, size, &view);
            // Every group of the code is on screen, in order.
            let shown: Vec<&str> = lines
                .iter()
                .map(|(l, _, _)| l.as_str())
                .skip_while(|l| *l != "Invite Code")
                .skip(1)
                .take_while(|l| l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
                .collect();
            assert_eq!(shown.join("-"), code, "{size:?}");
            let copy = &buttons[buttons.len() - 2];
            let done = &buttons[buttons.len() - 1];
            assert_eq!((&copy.action, &done.action), (&Action::CopyCode, &Action::Done));
            assert!(copy.active && copy.x + copy.w <= done.x, "{size:?}");
            assert_eq!(
                click(&atlas, size, (copy.x + 5.0, copy.y + 5.0), &view),
                Some(Action::CopyCode)
            );
            assert_eq!(
                click(&atlas, size, (done.x + 5.0, done.y + 5.0), &view),
                Some(Action::Done)
            );
            // Above the footer: the text, then at least three Give Host buttons.
            let give: Vec<&Button> = buttons
                .iter()
                .filter(|b| matches!(b.action, Action::GiveHost(_)))
                .collect();
            assert!(give.len() >= 3, "{size:?}: {}", give.len());
            let bottom = lines.iter().map(|(_, _, y)| *y + 10.0).fold(0.0, f32::max);
            assert!(give.iter().all(|b| b.y >= bottom && b.y + 20.0 <= size.1 - 33.0), "{size:?}");
        }
        // Before the code is known there is nothing to copy.
        let (_, buttons) = layout(
            &atlas,
            (427.0, 240.0),
            &View {
                code: None,
                ..view
            },
        );
        assert!(buttons.iter().all(|b| b.action != Action::CopyCode));
    }
}
