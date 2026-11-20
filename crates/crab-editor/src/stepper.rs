//! Small number window of the debug and simulation modes: left click on the box adds a step,
//! right click subtracts one, holding the button repeats with growing speed, and a number can
//! be typed. The logic (repeat schedule, limits of the type) is plain functions.

use eframe::egui::{self, vec2, Align2, Id, Sense, Ui};

/// Steps of a `float` value offered in the window.
pub const FLOAT_STEPS: [f64; 4] = [0.01, 0.1, 1.0, 10.0];
/// Index of the step used first (0.1).
const DEFAULT_STEP: usize = 1;
/// Pause before a held button starts to repeat, seconds.
const HOLD_DELAY: f64 = 0.4;
/// Repeat rate at the start and after `HOLD_RAMP` seconds, changes per second.
const RATE_FROM: f64 = 8.0;
const RATE_TO: f64 = 30.0;
const HOLD_RAMP: f64 = 3.0;

/// How many extra changes a button held for `held` seconds owes (the click itself is not counted).
pub fn due(held: f64) -> u32 {
    if held < HOLD_DELAY {
        return 0;
    }
    let t = held - HOLD_DELAY;
    let ramp = t.min(HOLD_RAMP);
    // Integral of the rate: it grows linearly from `RATE_FROM` to `RATE_TO`, then stays.
    let mut total = RATE_FROM * ramp + (RATE_TO - RATE_FROM) * ramp * ramp / (2.0 * HOLD_RAMP);
    total += RATE_TO * (t - ramp);
    1 + total as u32
}

/// Value after `dir` (+1 or -1) steps, kept in the limits of its type: an `int` neither wraps
/// round nor leaves `i32`, a `float` never becomes NaN or infinite.
pub fn bump(value: f64, dir: f64, step: f64, is_int: bool) -> f64 {
    fix(value + dir * step, is_int)
}

/// Brings any number (typed or computed) into the limits of its type.
pub fn fix(value: f64, is_int: bool) -> f64 {
    if value.is_nan() {
        return 0.0;
    }
    if is_int {
        return value
            .round()
            .clamp(f64::from(i32::MIN), f64::from(i32::MAX));
    }
    let v = value.clamp(f64::from(f32::MIN), f64::from(f32::MAX));
    // Rounds the noise of adding 0.1 ten times.
    if v.abs() < 1e9 {
        (v * 1e6).round() / 1e6
    } else {
        v
    }
}

/// Reads a typed number: a comma is a point.
pub fn parse(text: &str, is_int: bool) -> Option<f64> {
    let v: f64 = text.trim().replace(',', ".").parse().ok()?;
    v.is_finite().then(|| fix(v, is_int))
}

/// What the window remembers between frames.
#[derive(Clone, Default)]
struct Memo {
    step: Option<usize>,
    hold: Option<Hold>,
    text: String,
}

#[derive(Clone, Copy)]
struct Hold {
    secondary: bool,
    since: f64,
    applied: u32,
}

/// Text of a number: no trailing zeros, a comma as in the interface.
pub fn show_number(value: f64, is_int: bool) -> String {
    if is_int {
        return format!("{}", value as i64);
    }
    let text = format!("{value:.6}");
    text.trim_end_matches('0')
        .trim_end_matches('.')
        .replace('.', ",")
}

/// The window body; call it inside a popup. `salt` tells windows of different values apart.
/// Returns the new value when the user changed it.
pub fn show(ui: &mut Ui, salt: Id, value: f64, is_int: bool) -> Option<f64> {
    let memo_id = salt.with("stepper");
    let mut memo: Memo = ui.data_mut(|d| d.get_temp(memo_id).unwrap_or_default());
    let step_index = memo.step.unwrap_or(DEFAULT_STEP);
    let step = if is_int { 1.0 } else { FLOAT_STEPS[step_index] };
    let mut new = None;

    ui.set_min_width(150.0);
    // The box: a click adds, a right click subtracts, holding repeats.
    let (rect, response) = ui.allocate_exact_size(vec2(150.0, 30.0), Sense::hover());
    let hovered = ui.rect_contains_pointer(rect);
    let now = ui.input(|i| i.time);
    let (down_primary, down_secondary, pressed_primary, pressed_secondary) = ui.input(|i| {
        (
            i.pointer.primary_down(),
            i.pointer.secondary_down(),
            i.pointer.primary_pressed(),
            i.pointer.secondary_pressed(),
        )
    });
    let mut current = value;
    if hovered && (pressed_primary || pressed_secondary) {
        let secondary = !pressed_primary;
        current = bump(current, if secondary { -1.0 } else { 1.0 }, step, is_int);
        new = Some(current);
        memo.hold = Some(Hold {
            secondary,
            since: now,
            applied: 0,
        });
    } else if let Some(mut hold) = memo.hold {
        let down = if hold.secondary {
            down_secondary
        } else {
            down_primary
        };
        if down {
            let owed = due(now - hold.since);
            let dir = if hold.secondary { -1.0 } else { 1.0 };
            while hold.applied < owed {
                current = bump(current, dir, step, is_int);
                hold.applied += 1;
                new = Some(current);
            }
            memo.hold = Some(hold);
            ui.ctx().request_repaint();
        } else {
            memo.hold = None;
        }
    }
    let visuals = ui.style().interact(&response);
    ui.painter().rect(
        rect,
        4.0,
        visuals.bg_fill,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        show_number(current, is_int),
        egui::FontId::monospace(16.0),
        visuals.text_color(),
    );
    ui.weak(crate::i18n::tr("ЛКМ — плюс, ПКМ — минус; можно держать"));

    // Typing: the field follows the value until the user edits it.
    let field_id = salt.with("stepper_text");
    let focused = ui.memory(|m| m.has_focus(field_id));
    if !focused {
        memo.text = show_number(current, is_int);
    }
    let edit = ui.add(
        egui::TextEdit::singleline(&mut memo.text)
            .id(field_id)
            .desired_width(150.0),
    );
    if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        if let Some(v) = parse(&memo.text, is_int) {
            new = Some(v);
            memo.text = show_number(v, is_int);
        }
    }
    if !is_int {
        ui.horizontal(|ui| {
            ui.label(crate::i18n::tr("Шаг"));
            for (i, s) in FLOAT_STEPS.iter().enumerate() {
                if ui
                    .selectable_label(i == step_index, show_number(*s, false))
                    .clicked()
                {
                    memo.step = Some(i);
                }
            }
        });
    }
    ui.data_mut(|d| d.insert_temp(memo_id, memo));
    new
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_button_waits_then_repeats_faster() {
        assert_eq!(due(0.0), 0);
        assert_eq!(due(0.39), 0);
        assert_eq!(due(0.4), 1);
        // At least 5 changes a second once repeating (the spec asks for 5 per second).
        assert!(due(1.4) - due(0.4) >= 5);
        // Later it is faster than at the start.
        assert!(due(4.4) - due(3.4) > due(1.4) - due(0.4));
        // Never goes back.
        let mut last = 0;
        for i in 0..2000 {
            let d = due(f64::from(i) / 100.0);
            assert!(d >= last);
            last = d;
        }
    }

    #[test]
    fn numbers_stay_in_the_limits_of_their_type() {
        assert_eq!(
            bump(f64::from(i32::MAX), 1.0, 1.0, true),
            f64::from(i32::MAX)
        );
        assert_eq!(
            bump(f64::from(i32::MIN), -1.0, 10.0, true),
            f64::from(i32::MIN)
        );
        assert_eq!(fix(f64::NAN, false), 0.0);
        assert_eq!(fix(f64::INFINITY, false), f64::from(f32::MAX));
        assert_eq!(fix(f64::NEG_INFINITY, false), f64::from(f32::MIN));
        // Ten steps of 0.1 give 1.0, not 0.9999999999.
        let v = (0..10).fold(0.0, |v, _| bump(v, 1.0, 0.1, false));
        assert_eq!(v, 1.0);
    }

    #[test]
    fn typed_numbers_are_parsed_with_a_comma() {
        assert_eq!(parse("3,5", false), Some(3.5));
        assert_eq!(parse(" -12 ", true), Some(-12.0));
        assert_eq!(parse("2.6", true), Some(3.0));
        assert_eq!(parse("abc", true), None);
        assert_eq!(parse("inf", false), None);
        assert_eq!(parse("NaN", false), None);
        assert_eq!(show_number(0.5, false), "0,5");
        assert_eq!(show_number(10.0, false), "10");
        assert_eq!(show_number(-7.0, true), "-7");
    }
}
