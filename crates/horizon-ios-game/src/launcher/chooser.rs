//! Explicit, scrollable selections. Values never cycle on repeated button taps.
use super::{
    Launcher,
    pages::{Action, button},
};
use horizon_client::android_control_theme::{Face, Ui};

pub(super) enum Kind {
    Aspect,
    Profile,
    Orientation,
    UiScale,
}
pub(super) struct Chooser {
    title: &'static str,
    help: &'static str,
    device: Option<String>,
    choices: Vec<(String, String, Action, bool)>,
    pub scroll: f32,
}
impl Launcher {
    pub(super) fn open_chooser(&mut self, kind: Kind) {
        crate::platform::hide();
        self.catalog.editing = None;
        let device = None;
        let c = &self.catalog.config;
        let (title, help, choices) = match kind {
            Kind::Aspect => (
                "Aspect ratio",
                "Fits the entire picture without stretching. Touch coordinates follow the picture. Portrait swaps width and height.",
                [
                    crate::display::Aspect::Native,
                    crate::display::Aspect::Wide,
                    crate::display::Aspect::Classic,
                    crate::display::Aspect::Square,
                ]
                .into_iter()
                .map(|a| {
                    (
                        a.label().into(),
                        String::new(),
                        Action::ChooseAspect(a),
                        c.screen.aspect == a,
                    )
                })
                .collect(),
            ),
            Kind::Orientation => (
                "Screen orientation",
                "Applies to the launcher and game. Does not change touch bindings.",
                crate::catalog::Orientation::ALL
                    .into_iter()
                    .map(|mode| {
                        (
                            mode.label().into(),
                            String::new(),
                            Action::ChooseOrientation(mode),
                            c.orientation == mode,
                        )
                    })
                    .collect(),
            ),
            Kind::Profile => {
                let mut ids = vec![0, c.profile];
                ids.extend(
                    std::fs::read_dir(self.home.join("profiles"))
                        .into_iter()
                        .flatten()
                        .flatten()
                        .filter(|e| e.path().is_dir())
                        .filter_map(|e| {
                            e.file_name()
                                .to_str()?
                                .strip_prefix("profile-")?
                                .parse::<u32>()
                                .ok()
                        }),
                );
                ids.sort_unstable();
                ids.dedup();
                let next = ids.last().copied().unwrap_or(0).checked_add(1);
                let mut choices: Vec<_> = ids
                    .into_iter()
                    .map(|id| {
                        (
                            format!("Profile {id}"),
                            if id == 0 {
                                "Default profile".into()
                            } else {
                                "Separate saves, mods and packs".into()
                            },
                            Action::ChooseProfile(id),
                            id == c.profile,
                        )
                    })
                    .collect();
                if let Some(id) = next {
                    choices.push((
                        "Create profile".into(),
                        format!("Create Profile {id}"),
                        Action::ChooseProfile(id),
                        false,
                    ));
                }
                (
                    "Choose profile",
                    "Each profile keeps its own game settings and saves.",
                    choices,
                )
            }
            Kind::UiScale => (
                "Launcher UI scale",
                "Adapts the launcher to your phone's DPI. Increase or decrease if touch controls and text feel too small or too large.",
                [
                    (
                        0u32,
                        "Auto (Phone DPI)",
                        "Automatically matches your display's DPI density",
                    ),
                    (85, "85% (Compact)", "Fits more rows on screen"),
                    (100, "100% (Default DPI)", "Standard phone DPI scaling"),
                    (115, "115% (Large)", "Larger buttons, cards and text"),
                    (
                        130,
                        "130% (Extra Large)",
                        "Maximum touch comfort on high-DPI phones",
                    ),
                ]
                .into_iter()
                .map(|(pct, title, desc)| {
                    (
                        title.into(),
                        desc.into(),
                        Action::ChooseUiScale(pct),
                        c.ui_scale_percent == pct,
                    )
                })
                .collect(),
            ),
        };
        self.chooser = Some(Chooser {
            title,
            help,
            choices,
            device,
            scroll: 0.0,
        });
    }
}
impl Chooser {
    pub(super) fn confirm_delete(name: String) -> Self {
        Self {
            title: "Remove add-on?",
            help: "Deletes only the selected file from this profile. This cannot be undone.",
            device: Some(name),
            choices: vec![
                (
                    "Remove".into(),
                    String::new(),
                    Action::ConfirmDeleteLocal,
                    false,
                ),
                ("Cancel".into(), String::new(), Action::Dismiss, true),
            ],
            scroll: 0.0,
        }
    }
    pub(super) fn navigation(&self, width: f32) -> Vec<([f32; 4], Action)> {
        self.choices
            .iter()
            .enumerate()
            .map(|(i, (_, _, action, _))| {
                (
                    [
                        32.0,
                        137.0 + i as f32 * 84.0 - self.scroll,
                        width - 64.0,
                        76.0,
                    ],
                    *action,
                )
            })
            .collect()
    }
    pub(super) fn viewport(&self, height: f32) -> [f32; 3] {
        [
            137.0,
            height - 100.0,
            (self.choices.len() as f32 * 84.0 - (height - 237.0)).max(0.0),
        ]
    }
    pub fn draw(&mut self, ui: &mut Ui, targets: &mut Vec<([f32; 4], Action)>, modal_anim: f32) {
        let (w, h) = (ui.w, ui.h);
        let theme = horizon_client::android_control_theme::active_theme();
        let slide = (1.0 - modal_anim.clamp(0.0, 1.0)) * 16.0;
        targets.clear();
        ui.draw_theme_background(0.0, 0.0, w, h, &theme);
        ui.text(
            Face::SemiBold,
            24.0,
            32.0,
            32.0 + slide,
            theme.text_primary,
            self.title,
        );
        ui.text(
            Face::Regular,
            14.0,
            32.0,
            74.0 + slide,
            theme.text_muted,
            self.help,
        );
        if let Some(device) = &self.device {
            ui.hint_line(32.0, 100.0 + slide, w - 64.0, device, theme.accent);
        }
        let top = 132.0 + slide;
        let bottom = h - 94.0;
        self.scroll = self.scroll.clamp(
            0.0,
            (self.choices.len() as f32 * 84.0 - (bottom - top)).max(0.0),
        );
        ui.set_clip(Some([0.0, top, w, (bottom - top).max(1.0)]));
        for (i, (label, detail, action, selected)) in self.choices.iter().enumerate() {
            let y = top + i as f32 * 84.0 - self.scroll;
            if y + 76.0 < top || y > bottom {
                continue;
            }
            ui.rounded(
                32.0,
                y,
                w - 64.0,
                74.0,
                14.0,
                if *selected {
                    theme.surface_selected
                } else {
                    theme.surface
                },
            );
            ui.stroke_rounded(
                32.0,
                y,
                w - 64.0,
                74.0,
                14.0,
                if *selected { 1.6 } else { 1.0 },
                if *selected {
                    theme.accent
                } else {
                    theme.border
                },
            );
            ui.stroke_rounded(
                48.0,
                y + 26.0,
                22.0,
                22.0,
                11.0,
                1.8,
                if *selected {
                    theme.accent
                } else {
                    theme.text_muted
                },
            );
            if *selected {
                ui.rounded(53.0, y + 31.0, 12.0, 12.0, 6.0, theme.accent);
            }
            ui.text(
                Face::SemiBold,
                17.0,
                84.0,
                y + 11.0,
                theme.text_primary,
                label,
            );
            ui.text(
                Face::Regular,
                13.0,
                84.0,
                y + 40.0,
                theme.text_muted,
                detail,
            );
            if let Some(c) = ui.clip_rect([32.0, y, w - 64.0, 74.0]) {
                targets.push((c, *action));
            }
        }
        ui.set_clip(None);
        button(
            ui,
            targets,
            [32.0, h - 74.0, 160.0, 44.0],
            "Cancel",
            Action::Dismiss,
            false,
        );
    }
}
