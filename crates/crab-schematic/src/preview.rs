//! Pictures of the palette entries of the panel editor. They are drawn by the same symbol functions
//! as the sheets (`symbols.rs`), so that a device looks the same in the palette and on a sheet.

use crab_core::{ContactForm, Device, DeviceKind, Supply};

use crate::canvas::{Align, Canvas, Op, DOT, INK, TEXT, TEXT_M, THIN, WEAK};
use crate::control::ContactTable;
use crate::lang::Lang;
use crate::symbols::{draw_aux_field, draw_device, draw_field_device, Notes, POLE};

/// Which part of a device a palette entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PalettePart {
    /// The device as a whole, drawn as on the power or the control sheet.
    Whole,
    /// The coil of a contactor or a relay.
    Coil,
    /// A normally open contact of a relay.
    Contact,
}

/// The drawing of a palette entry: operations in sheet units around (0, 0) and their bounds.
#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    pub ops: Vec<Op>,
    pub min: (f32, f32),
    pub max: (f32, f32),
}

/// The drawing of `kind` (`part` of it) on a supply of `supply`, around (0, 0). A combination the
/// palette does not show (a relay as a whole, a coil of a breaker) gives an empty preview.
pub fn preview(kind: DeviceKind, part: PalettePart, supply: Supply) -> Preview {
    let mut c = Canvas::new();
    // A sample device: its designation is the letter of its kind and `?` (`-Q?`).
    let device = Device {
        id: format!("{}?", kind.letter()),
        kind,
        rating: String::new(),
        part: String::new(),
        bind: String::new(),
        load: String::new(),
        poles: None,
        of: String::new(),
        form: ContactForm::No,
        pins: String::new(),
        from: String::new(),
        free: false,
        series: String::new(),
    };
    let ops = match (part, kind) {
        (
            PalettePart::Whole,
            DeviceKind::Breaker
            | DeviceKind::Rcd
            | DeviceKind::Fuse
            | DeviceKind::Switch
            | DeviceKind::Contactor
            | DeviceKind::Psu
            | DeviceKind::Motor
            | DeviceKind::Load
            | DeviceKind::Terminal,
        ) => {
            // The conductors of the supply, as the power sheet draws them.
            let poles = supply.lines().len();
            draw_device(
                &mut c,
                &device,
                (0.0, 0.0),
                poles,
                supply,
                Lang::default(),
                &Notes::default(),
            );
            let ops = c.into_parts().1;
            if kind == DeviceKind::Terminal {
                // The numbers of the terminals are the sheet's wire numbers: not in a picture.
                ops.into_iter()
                    .filter(|op| !matches!(op, Op::Text { vertical: true, .. }))
                    .collect()
            } else {
                ops
            }
        }
        (PalettePart::Whole, DeviceKind::Feed) => {
            draw_feed(&mut c, &device, supply);
            c.into_parts().1
        }
        (PalettePart::Whole, DeviceKind::Sensor | DeviceKind::Button) => {
            draw_field_device(&mut c, &device, (0.0, 0.0), &ContactTable::default());
            c.into_parts().1
        }
        (PalettePart::Coil, DeviceKind::Contactor | DeviceKind::Relay) => {
            draw_field_device(&mut c, &device, (0.0, 0.0), &ContactTable::default());
            c.into_parts().1
        }
        (PalettePart::Contact, DeviceKind::Relay) => {
            // The normally open contact as the control ladder draws it, with its terminals `13-14`.
            draw_aux_field(
                &mut c,
                &device.id,
                ContactForm::No,
                (0.0, 0.0),
                ("13", "14"),
                None,
                None,
            );
            c.into_parts().1
        }
        _ => Vec::new(),
    };
    let (min, max) = bounds(&ops);
    Preview { ops, min, max }
}

/// The incoming supply: one conductor per pole from a terminal at the top, named, with the
/// designation left of the first one. A feed is the bus of its sheet, so the sheets draw nothing for
/// it; the palette needs a picture of it.
fn draw_feed(c: &mut Canvas, device: &Device, supply: Supply) {
    const LEN: f32 = 30.0;
    let lines = supply.lines();
    for (k, name) in lines.iter().enumerate() {
        let x = k as f32 * POLE;
        c.circle(x, 0.0, DOT, THIN, INK);
        c.line(&[(x, DOT), (x, LEN)], THIN, INK);
        c.text(x + 6.0, 8.0, name, TEXT_M, Align::Left, WEAK);
    }
    c.bold(-8.0, 12.0, &device.designation(), TEXT, Align::Right, INK);
}

/// The bounds of the operations. A text is taken as wide as its characters at a rough width of
/// 0.6 of its size, so the bounds stay a little wider than the glyphs.
fn bounds(ops: &[Op]) -> ((f32, f32), (f32, f32)) {
    let mut min = (f32::MAX, f32::MAX);
    let mut max = (f32::MIN, f32::MIN);
    let mut add = |x: f32, y: f32| {
        min = (min.0.min(x), min.1.min(y));
        max = (max.0.max(x), max.1.max(y));
    };
    for op in ops {
        match op {
            Op::Line { points, .. } => {
                for &(x, y) in points {
                    add(x, y);
                }
            }
            Op::Rect { x, y, w, h, .. } => {
                add(*x, *y);
                add(x + w, y + h);
            }
            Op::Circle { x, y, r, .. } | Op::Dot { x, y, r, .. } => {
                add(x - r, y - r);
                add(x + r, y + r);
            }
            Op::Text {
                x,
                y,
                text,
                size,
                align,
                vertical,
                ..
            } => {
                let length = text.chars().count() as f32 * size * 0.6;
                if *vertical {
                    // Turned text: `x` is its middle line; `Left` runs up from `y` (5.2).
                    let (from, to) = match align {
                        Align::Left => (-length, 0.0),
                        Align::Center => (-length / 2.0, length / 2.0),
                        Align::Right => (0.0, length),
                    };
                    add(x - size / 2.0, y + from);
                    add(x + size / 2.0, y + to);
                } else {
                    let (from, to) = match align {
                        Align::Left => (0.0, length),
                        Align::Center => (-length / 2.0, length / 2.0),
                        Align::Right => (-length, 0.0),
                    };
                    add(x + from, y - size / 2.0);
                    add(x + to, y + size / 2.0);
                }
            }
        }
    }
    if min.0 > max.0 {
        return ((0.0, 0.0), (0.0, 0.0));
    }
    (min, max)
}
