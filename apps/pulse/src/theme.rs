//! Theme integration for Nixe Pulse, supporting Omarchy live restyling.

use crate::prefs;
pub use nixe_theme::*;

pub fn install() {
    nixe_theme::install(Style {
        window_class: "pulse-window",
        base_skip: &[],
        css: nixe_playback::controls::stylesheet(include_str!("style.css")),
        no_glow_css: "",
        extra_colors: Some(|p| format!("@define-color theme_video {};\n", p.bg)),
        prefs: || {
            let p = prefs::get();
            ThemePrefs { mode: p.mode, theme: p.theme, glow: p.glow, reduce_motion: p.reduce_motion }
        },
        set_theme: Some(|mode, theme| {
            prefs::update(|p| {
                p.mode = mode;
                p.theme = theme;
            })
        }),
    });
}
