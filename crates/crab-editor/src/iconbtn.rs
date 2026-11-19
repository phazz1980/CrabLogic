//! Small icon buttons of the left panel: a one-colour glyph (egui draws no colour emoji) painted
//! in a fixed colour by meaning, with the old caption kept as the hover text.

use eframe::egui::{self, Color32, Response, RichText, Ui};

/// What the button does; picks the glyph and its colour.
#[derive(Clone, Copy)]
pub enum Icon {
    Designer,
}

impl Icon {
    fn glyph(self) -> &'static str {
        match self {
            Icon::Designer => "🎨",
        }
    }

    fn color(self) -> Color32 {
        match self {
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
