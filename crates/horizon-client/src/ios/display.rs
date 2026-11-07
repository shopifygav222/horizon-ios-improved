//! Display scaling for iOS. iOS handles resolution natively via UIKit.
use serde::{Deserialize, Serialize};

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
        let minimum = (320.0 / w as f64).max(240.0 / h as f64).min(1.0);
        let scale = (self.percent() as f64 / 100.0).max(minimum);
        [w, h].map(|v| (v as f64 * scale).round().max(1.0) as u32)
    }
}

static REQUESTED_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(100);

pub fn set_scale(scale: RenderScale) {
    REQUESTED_SCALE.store(scale.percent(), std::sync::atomic::Ordering::Relaxed);
}

/// iOS lets winit/UIKit manage the display; no manual surface resizing needed.
pub fn prepare(window: &winit::window::Window) -> Result<Option<[u32; 2]>, String> {
    let scale = RenderScale::new(REQUESTED_SCALE.load(std::sync::atomic::Ordering::Relaxed));
    let current = window.inner_size();
    let view = [current.width.max(1), current.height.max(1)];
    let size = scale.buffer_size(view);
    if size == [current.width, current.height] {
        Ok(None)
    } else {
        Ok(Some(size))
    }
}

/// Reconcile the renderer with the current window size.
pub fn apply(
    window: &winit::window::Window,
    renderer: &mut horizon_client::gfx::Renderer,
) -> Result<(), String> {
    if renderer.suspended {
        return Ok(());
    }
    let size = window.inner_size();
    let size = [size.width, size.height];
    if size[0] > 0 && size[1] > 0 && size != [renderer.config.width, renderer.config.height] {
        renderer.resize(size[0], size[1]);
    }
    Ok(())
}

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
