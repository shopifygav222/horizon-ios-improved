//! Whole-frame resolution on iOS, and the window choices the launcher makes. UIKit sizes the view;
//! the render scale lowers its contentScaleFactor, so the drawable and the touch coordinates winit
//! reports shrink together and input stays on the picture (Android scales its native window and
//! its input instead).
use serde::{Deserialize, Serialize};
use std::{
    cell::Cell,
    sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering},
};
use winit::platform::ios::{ScreenEdge, ValidOrientations, WindowExtIOS};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RenderScale(u32);

impl Default for RenderScale {
    fn default() -> Self {
        Self(100)
    }
}

impl RenderScale {
    pub const PRESETS: [u32; 6] = [100, 85, 75, 67, 50, 25];

    pub fn new(percent: u32) -> Self {
        Self(percent.clamp(25, 100))
    }

    pub fn parse(text: &str) -> Result<Self, &'static str> {
        let percent = text
            .trim()
            .trim_end_matches('%')
            .trim()
            .parse::<u32>()
            .map_err(|_| "Enter a whole percentage from 25 to 100")?;
        if !(25..=100).contains(&percent) {
            return Err("Enter a percentage from 25 to 100");
        }
        Ok(Self(percent))
    }

    pub fn percent(self) -> u32 {
        self.0.clamp(25, 100)
    }

    pub fn buffer_size(self, view: [u32; 2]) -> [u32; 2] {
        let [w, h] = view.map(|v| v.max(1));
        // Keep standard menus usable even on a narrow screen. Never
        // enlarge a surface that is already smaller than the GUI minimum.
        let minimum = (320.0 / w as f64).max(240.0 / h as f64).min(1.0);
        let scale = (self.percent() as f64 / 100.0).max(minimum);
        [w, h].map(|v| (v as f64 * scale).round().max(1.0) as u32)
    }
}

static REQUESTED_SCALE: AtomicU32 = AtomicU32::new(100);
static ORIENTATION: AtomicU8 = AtomicU8::new(0);
static IMMERSIVE: AtomicBool = AtomicBool::new(false);
/// The content's shape, `width << 16 | height` (0: the screen's own).
static ASPECT: AtomicU32 = AtomicU32::new(0);

/// What was last applied to which window, so a new window (the game's, then the launcher's again)
/// gets the choices too and an unchanged one is left alone.
#[derive(Clone, Copy, PartialEq)]
struct Applied {
    window: winit::window::WindowId,
    orientation: u8,
    immersive: bool,
    aspect: u32,
}

thread_local! {
    static APPLIED: Cell<Option<Applied>> = const { Cell::new(None) };
}

pub fn set_scale(scale: RenderScale) {
    REQUESTED_SCALE.store(scale.percent(), Ordering::Relaxed);
}

pub fn request_orientation(orientation: crate::catalog::Orientation) {
    use crate::catalog::Orientation;
    let code = match orientation {
        Orientation::Landscape | Orientation::ReverseLandscape => 0,
        Orientation::Portrait | Orientation::ReversePortrait => 1,
        Orientation::Auto => 2,
    };
    ORIENTATION.store(code, Ordering::Relaxed);
}

/// Immersive hides the status bar and home indicator and keeps the first swipe from an edge for
/// the game. iOS draws under the sensor housing either way; the image is never cut for it. The
/// aspect letterboxes the game in the screen (winit's `set_content_aspect`): rendering and
/// touches share those bounds.
pub fn configure_screen(immersive: bool, _avoid_cutout: bool, aspect: Aspect) {
    IMMERSIVE.store(immersive, Ordering::Relaxed);
    let [w, h] = aspect.ratio();
    ASPECT.store((w << 16) | h, Ordering::Relaxed);
}

fn apply_choices(window: &winit::window::Window) {
    let wanted = Applied {
        window: window.id(),
        orientation: ORIENTATION.load(Ordering::Relaxed),
        immersive: IMMERSIVE.load(Ordering::Relaxed),
        aspect: ASPECT.load(Ordering::Relaxed),
    };
    let previous = APPLIED.with(|applied| applied.replace(Some(wanted)));
    let fresh = previous.is_none_or(|p| p.window != wanted.window);
    if fresh || previous.is_some_and(|p| p.orientation != wanted.orientation) {
        window.set_valid_orientations(match wanted.orientation {
            0 => ValidOrientations::Landscape,
            1 => ValidOrientations::Portrait,
            _ => ValidOrientations::LandscapeAndPortrait,
        });
    }
    if fresh || previous.is_some_and(|p| p.aspect != wanted.aspect) {
        let (w, h) = (wanted.aspect >> 16, wanted.aspect & 0xffff);
        window.set_content_aspect((w > 0 && h > 0).then_some((w, h)));
    }
    if fresh || previous.is_some_and(|p| p.immersive != wanted.immersive) {
        window.set_prefers_status_bar_hidden(wanted.immersive);
        window.set_prefers_home_indicator_hidden(wanted.immersive);
        window.set_preferred_screen_edges_deferring_system_gestures(if wanted.immersive {
            ScreenEdge::ALL
        } else {
            ScreenEdge::NONE
        });
    }
}

/// The view's contentScaleFactor for the chosen render scale; true when it changed.
fn apply_scale(window: &winit::window::Window) -> bool {
    let native = f64::from(crate::platform::display_density());
    let factor = window.scale_factor();
    let size = window.inner_size();
    if size.width == 0 || size.height == 0 || !factor.is_finite() || factor <= 0.0 {
        return false;
    }
    let view =
        [size.width, size.height].map(|v| (f64::from(v) / factor * native).round().max(1.0) as u32);
    let target = RenderScale::new(REQUESTED_SCALE.load(Ordering::Relaxed)).buffer_size(view);
    let wanted = native * f64::from(target[0]) / f64::from(view[0]);
    if (wanted - factor).abs() < 1e-3 {
        return false;
    }
    window.set_scale_factor(wanted);
    true
}

/// Set the drawable's size before a renderer recreates its surface.
pub fn prepare(window: &winit::window::Window) -> Result<Option<[u32; 2]>, String> {
    apply_choices(window);
    if !apply_scale(window) {
        return Ok(None);
    }
    let size = window.inner_size();
    Ok(Some([size.width, size.height]))
}

/// Reconcile the renderer with the view: a new render scale, or a rotation UIKit has finished.
pub fn apply(
    window: &winit::window::Window,
    renderer: &mut horizon_client::gfx::Renderer,
) -> Result<(), String> {
    if renderer.suspended {
        return Ok(());
    }
    apply_choices(window);
    apply_scale(window);
    let size = window.inner_size();
    let size = [size.width, size.height];
    if size[0] > 0 && size[1] > 0 && size != [renderer.config.width, renderer.config.height] {
        renderer.resize(size[0], size[1]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_surface_area_dimensions_without_changing_aspect() {
        assert_eq!(RenderScale::new(50).buffer_size([2772, 1280]), [1386, 640]);
        assert_eq!(
            RenderScale::default().buffer_size([2772, 1280]),
            [2772, 1280]
        );
    }

    #[test]
    fn edited_and_old_settings_have_bounded_values() {
        assert_eq!(RenderScale::parse(" 67% ").unwrap().percent(), 67);
        assert!(RenderScale::parse("0").is_err());
        assert!(RenderScale::parse("101").is_err());
        assert_eq!(
            serde_json::from_str::<RenderScale>("0").unwrap().percent(),
            25
        );
    }
}

/// Fixed presentation ratios; logical rendering and input share the same bounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Aspect {
    #[default]
    Native,
    Wide,
    Classic,
    Square,
}
impl Aspect {
    pub fn label(self) -> &'static str {
        match self {
            Self::Native => "Native",
            Self::Wide => "16:9",
            Self::Classic => "4:3",
            Self::Square => "1:1",
        }
    }
    pub fn ratio(self) -> [u32; 2] {
        match self {
            Self::Native => [0, 0],
            Self::Wide => [16, 9],
            Self::Classic => [4, 3],
            Self::Square => [1, 1],
        }
    }
}

/// Client-wide screen choices. Missing preferences start with the system bars visible.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenPreferences {
    pub immersive: bool,
    pub avoid_cutout: bool,
    pub aspect: Aspect,
}
impl Default for ScreenPreferences {
    fn default() -> Self {
        Self {
            immersive: false,
            avoid_cutout: false,
            aspect: Aspect::Native,
        }
    }
}
