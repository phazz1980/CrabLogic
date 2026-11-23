//! Pictures of the palette entries (`specs/panel-palette`, task 5).

use crab_core::{DeviceKind, Supply};
use crab_schematic::{preview, Op, PalettePart, Preview};

/// The entries the palette shows, as the editor asks for them.
const PALETTE: &[(DeviceKind, PalettePart)] = &[
    (DeviceKind::Breaker, PalettePart::Whole),
    (DeviceKind::Rcd, PalettePart::Whole),
    (DeviceKind::Fuse, PalettePart::Whole),
    (DeviceKind::Switch, PalettePart::Whole),
    (DeviceKind::Psu, PalettePart::Whole),
    (DeviceKind::Contactor, PalettePart::Whole),
    (DeviceKind::Contactor, PalettePart::Coil),
    (DeviceKind::Relay, PalettePart::Coil),
    (DeviceKind::Relay, PalettePart::Contact),
    (DeviceKind::Motor, PalettePart::Whole),
    (DeviceKind::Load, PalettePart::Whole),
    (DeviceKind::Terminal, PalettePart::Whole),
    (DeviceKind::Sensor, PalettePart::Whole),
    (DeviceKind::Button, PalettePart::Whole),
    (DeviceKind::Feed, PalettePart::Whole),
];

const SUPPLIES: [Supply; 2] = [Supply::Ac230, Supply::Ac400];

#[test]
fn every_palette_entry_has_a_picture_with_bounds() {
    for &(kind, part) in PALETTE {
        for supply in SUPPLIES {
            let p = preview(kind, part, supply);
            assert!(!p.ops.is_empty(), "{kind:?} {part:?} {supply:?}: no ops");
            assert!(
                p.min.0 < p.max.0 && p.min.1 < p.max.1,
                "{kind:?} {part:?} {supply:?}: bounds {:?} {:?}",
                p.min,
                p.max
            );
        }
    }
}

#[test]
fn bounds_hold_every_point_of_the_ops() {
    let p = preview(DeviceKind::Motor, PalettePart::Whole, Supply::Ac400);
    for op in &p.ops {
        if let Op::Line { points, .. } = op {
            for &(x, y) in points {
                assert!(p.min.0 <= x && x <= p.max.0 && p.min.1 <= y && y <= p.max.1);
            }
        }
    }
}

#[test]
fn coil_is_the_wide_body() {
    for kind in [DeviceKind::Contactor, DeviceKind::Relay] {
        let p = preview(kind, PalettePart::Coil, Supply::Ac230);
        let wide = p.ops.iter().any(|op| {
            matches!(op, Op::Rect { w, h, .. } if (*w - 30.0).abs() < 1e-3 && (*h - 15.0).abs() < 1e-3)
        });
        assert!(wide, "{kind:?} coil has no 30 x 15 body");
    }
}

#[test]
fn preview_is_deterministic() {
    for &(kind, part) in PALETTE {
        for supply in SUPPLIES {
            let a: Preview = preview(kind, part, supply);
            let b: Preview = preview(kind, part, supply);
            assert_eq!(a, b, "{kind:?} {part:?} {supply:?}");
        }
    }
}

#[test]
fn combinations_the_palette_does_not_show_are_empty() {
    let p = preview(DeviceKind::Relay, PalettePart::Whole, Supply::Ac230);
    assert!(p.ops.is_empty());
    assert_eq!(p.min, p.max);
    let p = preview(DeviceKind::Breaker, PalettePart::Coil, Supply::Ac230);
    assert!(p.ops.is_empty());
}
