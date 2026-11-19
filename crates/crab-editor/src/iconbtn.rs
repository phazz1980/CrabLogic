//! Small icon buttons of the left panel: a one-colour glyph (egui draws no colour emoji) painted
//! in a fixed colour by meaning, with the old caption kept as the hover text.

use eframe::egui::{self, Color32, Response, RichText, Ui};

/// What the button does; picks the glyph and its colour.
#[derive(Clone, Copy)]
pub enum Icon {
    Add,
    Delete,
    Settings,
    Find,
    Designer,
}

impl Icon {
    fn glyph(self) -> &'static str {
        match self {
            Icon::Add => "➕",
            Icon::Delete => "🗑",
            Icon::Settings => "⚙",
            Icon::Find => "🔍",
            Icon::Designer => "🎨",
        }
    }

    fn color(self) -> Color32 {
        match self {
            Icon::Add => Color32::from_rgb(70, 170, 90),
            Icon::Delete => Color32::from_rgb(220, 80, 70),
            Icon::Settings => Color32::from_rgb(90, 150, 230),
            Icon::Find => Color32::from_rgb(230, 170, 50),
            Icon::Designer => Color32::from_rgb(190, 100, 210),
        }
    }
}

/// A small button with a coloured glyph. `hint` is the translated caption shown on hover.
pub fn icon_button(ui: &mut Ui, enabled: bool, icon: Icon, hint: &str) -> Response {
    let color = if enabled {
        icon.color()
    } else {
        ui.visuals().weak_text_color()
    };
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(icon.glyph()).size(14.0).color(color))
            .min_size(egui::vec2(26.0, 22.0))
            .frame_when_inactive(false),
    )
    .on_hover_text(hint)
}

/// A small «add» button that keeps its name: a plus and the caption without its own `+`, in the
/// ordinary text colour (white in the dark theme).
pub fn add_button(ui: &mut Ui, enabled: bool, caption: &str, hint: &str) -> Response {
    let name = caption.trim_start_matches('+').trim_start();
    let text = format!("{} {name}", Icon::Add.glyph());
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).size(14.0))
            .min_size(egui::vec2(0.0, 22.0))
            .frame_when_inactive(false),
    );
    if hint.is_empty() {
        response
    } else {
        response.on_hover_text(hint)
    }
}
