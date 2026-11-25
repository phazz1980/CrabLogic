use crab_core::{ContactForm, Device, DeviceKind};
use crab_schematic::{Align, Item, ItemKind, Op, Sheet};
use eframe::egui::{self, Pos2};

use super::canvas::*;
use super::properties::*;
use super::tree::*;

use super::*;

#[test]
fn a_coil_moved_to_another_sheet_is_offered_the_next_designation_of_that_sheet() {
    let mut panel = crab_core::Panel::default();
    panel
        .devices
        .push(new_device("K3.1".to_owned(), DeviceKind::Relay));
    assert_eq!(
        sheet_name_offer("K3.1", DeviceKind::Relay, &panel, 5).as_deref(),
        Some("K5.1")
    );
    // The coil stays on its sheet: nothing to offer.
    assert_eq!(sheet_name_offer("K3.1", DeviceKind::Relay, &panel, 3), None);
    // Designations not named by a sheet are not offered anything.
    for id in ["K1", "K_1", "K3.", "k3.1", "K3.1a"] {
        assert_eq!(
            sheet_name_offer(id, DeviceKind::Relay, &panel, 5),
            None,
            "{id}"
        );
    }
    // The next free one on the sheet: after K5.1 comes K5.2.
    panel
        .devices
        .push(new_device("K5.1".to_owned(), DeviceKind::Relay));
    assert_eq!(
        sheet_name_offer("K3.1", DeviceKind::Relay, &panel, 5).as_deref(),
        Some("K5.2")
    );
}

#[test]
fn a_branch_on_400_v_chooses_among_the_phases_it_is_given() {
    let all: Vec<String> = ["L1", "L2", "L3", "N"].map(str::to_owned).to_vec();
    let text = |c: Vec<Vec<String>>| c.iter().map(|c| c.join("+")).collect::<Vec<_>>();
    // One pole: a phase; two: a phase and the neutral or two phases; three and four: nothing.
    assert_eq!(
        text(phase_choices(DeviceKind::Breaker, Some(1), &all)),
        ["L1", "L2", "L3"]
    );
    assert_eq!(
        text(phase_choices(DeviceKind::Breaker, Some(2), &all)),
        ["L1+N", "L2+N", "L3+N", "L1+L2", "L1+L3", "L2+L3"]
    );
    assert!(phase_choices(DeviceKind::Breaker, None, &all).is_empty());
    // A motor under a three-pole contactor; under one phase there is nothing to choose.
    let three: Vec<String> = ["L1", "L2", "L3"].map(str::to_owned).to_vec();
    assert_eq!(phase_choices(DeviceKind::Motor, None, &three).len(), 6);
    let one: Vec<String> = ["L2", "N"].map(str::to_owned).to_vec();
    assert!(phase_choices(DeviceKind::Load, None, &one).is_empty());
    assert_eq!(text(phase_choices(DeviceKind::Fuse, Some(1), &one)), ["L2"]);
    assert_eq!(phase_caption(&[], "по умолчанию"), "по умолчанию");
    assert_eq!(phase_caption(&one, ""), "L2 + N");
}

#[test]
fn a_diagnostic_is_about_a_device_by_its_designation() {
    assert!(is_about(
        "-K1 не привязано к входу или выходу проекта",
        "K1"
    ));
    assert!(is_about("устройство «K1»: недопустимое обозначение", "K1"));
    assert!(is_about(
        "обозначение -K1 занято несколькими устройствами",
        "K1"
    ));
    // Not another device that starts the same way.
    assert!(!is_about("-K12 не привязано", "K1"));
    assert!(!is_about("-K1_a не привязано", "K1"));
    assert!(!is_about("-XK1 не привязано", "K1"));
    assert!(!is_about("всё в порядке", "K1"));
}

#[test]
fn the_language_of_the_sheets_follows_the_editor_until_chosen() {
    let mut view = PanelView::default();
    let editor = editor_lang();
    assert_eq!(view.language(), editor);
    view.lang = Some(if editor == Lang::Ru {
        Lang::En
    } else {
        Lang::Ru
    });
    assert_ne!(view.language(), editor);
}

#[test]
fn every_kind_has_a_title_and_belongs_to_one_group() {
    for kind in DeviceKind::ALL {
        assert!(!kind_title(kind).is_empty());
        let groups = GROUPS
            .iter()
            .filter(|(_, kinds)| kinds.contains(&kind))
            .count();
        assert_eq!(groups, 1, "{kind:?}");
    }
}

/// The sheets of a panel are drawn without a panic, with every kind of operation, at the
/// scales of the fit and of a close look.
#[test]
fn drawing_the_operations_of_every_sheet_does_not_panic() {
    let (lib, _) =
        crab_core::Library::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library")).unwrap();
    let mut project =
        crab_core::Project::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/pump"))
            .unwrap();
    project.panel = toml::from_str(
        r#"
supply = "ac400"
[[device]]
id = "X1"
kind = "feed"
[[device]]
id = "Q1"
kind = "breaker"
rating = "C16"
load = "K1"
[[device]]
id = "K1"
kind = "contactor"
bind = "pump"
load = "M1"
[[device]]
id = "M1"
kind = "motor"
[[device]]
id = "B1"
kind = "sensor"
bind = "sensor.out"
"#,
    )
    .unwrap();
    for lang in [Lang::Ru, Lang::En] {
        let (document, _) = build_document_in(&project, &lib, lang, Format::A3).unwrap();
        let (page_w, _) = document.page_size();
        let ctx = egui::Context::default();
        for scale in [0.4, 2.0] {
            let mut drawn = 0;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let view = View {
                    origin: Pos2::new(10.0, 10.0),
                    scale: scale * 1000.0 / page_w,
                };
                let painter = ui.painter().clone();
                for (i, sheet) in document.sheets.iter().enumerate() {
                    draw_ops(&painter, &view, &document.frame_ops(i, lang));
                    draw_ops(&painter, &view, &sheet.ops);
                    drawn += sheet.ops.len();
                }
            });
            // The textures of the fonts were made for nobody: say they are not needed.
            output.textures_delta.clear();
            assert!(drawn > 100, "{drawn}");
        }
    }
}

fn symbol(view: &'static str, device: &str, ops: Vec<usize>) -> Item {
    Item {
        kind: ItemKind::Symbol {
            view,
            device: device.to_owned(),
            origin: (0.0, 0.0),
            ports: Vec::new(),
        },
        ops,
    }
}

fn line(points: &[(f32, f32)]) -> Op {
    Op::Line {
        points: points.to_vec(),
        width: 1.0,
        dashed: false,
        rgb: (0, 0, 0),
    }
}

fn sheet_of(ops: Vec<Op>, items: Vec<Item>) -> Sheet {
    Sheet {
        title: String::new(),
        id: None,
        body: String::new(),
        ops,
        items,
    }
}

#[test]
fn a_wire_from_a_part_ends_at_its_device() {
    let part = |view: &'static str, device: &str, name: &str| Item {
        kind: ItemKind::Symbol {
            view,
            device: device.to_owned(),
            origin: (0.0, 0.0),
            ports: vec![crab_schematic::scene::Port {
                name: name.to_owned(),
                x: 10.0,
                y: 10.0,
                dir: crab_schematic::scene::Dir::Down,
            }],
        },
        ops: Vec::new(),
    };
    let panel = crab_core::Panel::default();
    let channel = sheet_of(Vec::new(), vec![part("board", "A1/IO5", "IO5")]);
    let (device, terminal, _) =
        port_at(&channel, &panel, Pos2::new(10.0, 10.0), 2.0).expect("a port");
    assert_eq!((device.as_str(), terminal.as_str()), ("A1", "IO5"));
    let coil = sheet_of(Vec::new(), vec![part("control", "K1/coil", "A1")]);
    let (device, terminal, _) = port_at(&coil, &panel, Pos2::new(10.0, 10.0), 2.0).expect("a port");
    assert_eq!((device.as_str(), terminal.as_str()), ("K1", "A1"));
}

/// The rewiring gesture on the pump example: the end of every wire is found under the
/// pointer, every terminal marked as allowed is accepted by the core, and the panel still
/// builds after the change.
#[test]
fn every_marked_terminal_takes_the_end_of_the_wire() {
    let (lib, _) =
        crab_core::Library::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library")).unwrap();
    let project =
        crab_core::Project::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/pump"))
            .unwrap();
    let (document, _) = build_document_in(&project, &lib, Lang::Ru, Format::A3).unwrap();
    let mut moved = 0;
    for sheet in &document.sheets {
        for item in &sheet.items {
            let ItemKind::Wire { key, .. } = &item.kind else {
                continue;
            };
            let lines: Vec<&Vec<(f32, f32)>> = item
                .ops
                .iter()
                .filter_map(|&o| match &sheet.ops[o] {
                    Op::Line { points, .. } => Some(points),
                    _ => None,
                })
                .collect();
            let (Some(first), Some(last)) = (
                lines.first().and_then(|l| l.first()),
                lines.last().and_then(|l| l.last()),
            ) else {
                continue;
            };
            for (end, at) in [
                (crab_core::WireEnd::From, *first),
                (crab_core::WireEnd::To, *last),
            ] {
                let found = wire_end_at(sheet, Pos2::new(at.0, at.1), 1.0);
                assert!(found.is_some(), "конец провода {key} не найден");
                for (target, _) in rewire_targets(sheet, &project, &lib, key, end) {
                    let mut copy = project.clone();
                    crab_core::rewire(&mut copy, &lib, key, end, &target)
                        .unwrap_or_else(|e| panic!("{key} → {target}: {e}"));
                    build_document_in(&copy, &lib, Lang::Ru, Format::A3)
                        .unwrap_or_else(|e| panic!("{key} → {target}: {e:?}"));
                    moved += 1;
                }
            }
        }
    }
    assert!(moved > 0, "ни один конец провода нельзя перепаять");
}

/// A signal wire is drawn from the device to the channel of the board, though its key starts
/// at the board: the end at the channel is still the `from` end.
#[test]
fn the_end_at_the_board_is_the_board_end_of_the_key() {
    let (lib, _) =
        crab_core::Library::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library")).unwrap();
    let project =
        crab_core::Project::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/pump"))
            .unwrap();
    let (document, _) = build_document_in(&project, &lib, Lang::Ru, Format::A3).unwrap();
    let mut checked = 0;
    for sheet in &document.sheets {
        let board_ports: Vec<(f32, f32)> = sheet
            .items
            .iter()
            .filter_map(|i| match &i.kind {
                ItemKind::Symbol {
                    view: "board",
                    ports,
                    ..
                } => Some(ports.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>()),
                _ => None,
            })
            .flatten()
            .collect();
        for item in &sheet.items {
            let ItemKind::Wire { key, from, .. } = &item.kind else {
                continue;
            };
            if from.device != crab_schematic::BOARD {
                continue;
            }
            let points: Vec<(f32, f32)> = item
                .ops
                .iter()
                .filter_map(|&o| match &sheet.ops[o] {
                    Op::Line { points, .. } => Some(points.clone()),
                    _ => None,
                })
                .flatten()
                .collect();
            let at_board = points.iter().copied().find(|q| {
                board_ports
                    .iter()
                    .any(|b| (b.0 - q.0).hypot(b.1 - q.1) <= 3.0)
            });
            let Some(q) = at_board else {
                continue;
            };
            let (k, end, _) = wire_end_at(sheet, Pos2::new(q.0, q.1), 0.5).expect("конец");
            if k == *key {
                assert_eq!(end, crab_core::WireEnd::From, "{key}");
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "нет сигнальных проводов к плате");
}

/// An entry of the palette dropped on a sheet makes its device and puts it there: a coil is
/// named by the sheet in the order of the drops, a contact comes without its coil.
#[test]
fn an_entry_of_the_palette_dropped_on_a_sheet_makes_its_device_there() {
    use crab_schematic::PalettePart::{Coil, Contact, Whole};
    let mut project =
        crab_core::Project::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/pump"))
            .unwrap();
    project.panel = crab_core::Panel::default();
    let on = SheetId::Free(0);
    let drop = |p: &mut crab_core::Project, kind, part| {
        drop_new(
            p,
            PalettePayload { kind, part },
            on,
            Some(4),
            (120.0, 200.0),
        )
    };
    assert_eq!(drop(&mut project, DeviceKind::Relay, Coil), "K4.1");
    assert_eq!(drop(&mut project, DeviceKind::Contactor, Coil), "K4.2");
    let contact = drop(&mut project, DeviceKind::Relay, Contact);
    let breaker = drop(&mut project, DeviceKind::Breaker, Whole);
    let layout = &project.panel_layout;
    assert_eq!(layout.placed["K4.1/coil"].sheet, on);
    assert_eq!(layout.placed["K4.2/coil"].sheet, on);
    assert_eq!(contact, "K_1");
    let c = project.panel.device(&contact).unwrap();
    assert_eq!((c.kind, c.of.as_str()), (DeviceKind::Aux, ""));
    assert!(layout.placed.contains_key(&contact));
    assert_eq!(
        project.panel.device(&breaker).unwrap().kind,
        DeviceKind::Breaker
    );
    assert_eq!(
        (layout.placed[&breaker].x, layout.placed[&breaker].y),
        (120.0, 200.0)
    );
    // Every entry of the palette has a picture.
    for (_, kinds) in GROUPS {
        for &kind in kinds {
            for (part, _) in palette_entries(kind) {
                let pv = crab_schematic::preview(kind, part, crab_core::Supply::Ac230);
                assert!(!pv.ops.is_empty(), "{kind:?} {part:?}");
            }
        }
    }
}

/// The terminals of a coil connect to every line of the panel and to the terminals of the
/// chosen sheet, contacts of the power sheet among them, but not to the coil itself.
#[test]
fn a_coil_terminal_can_go_to_a_line_or_to_any_terminal_of_a_sheet() {
    let (lib, _) =
        crab_core::Library::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library")).unwrap();
    let project =
        crab_core::Project::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/pump"))
            .unwrap();
    let (doc, _) = build_document_in(&project, &lib, Lang::Ru, Format::A3).unwrap();
    let power = doc
        .sheets
        .iter()
        .position(|s| s.id == Some(SheetId::Power(0)))
        .unwrap()
        + 1;
    let targets = connect_targets(&doc, &project.panel, power, "K1");
    for line in ["bus:+24V", "bus:0V", "bus:L", "bus:N"] {
        assert!(targets.iter().any(|t| t == line), "{line} in {targets:?}");
    }
    assert!(targets.iter().any(|t| t.starts_with("Q1:")), "{targets:?}");
    assert!(!targets.iter().any(|t| t.starts_with("K1:")), "{targets:?}");
    assert_eq!(target_title("bus:L"), format!("{} L", tr("линия")));
}

#[test]
fn a_dropped_entry_is_drawn_on_its_sheet() {
    use crab_schematic::PalettePart::{Coil, Contact, Whole};
    let (lib, _) =
        crab_core::Library::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library")).unwrap();
    for (kind, part) in [
        (DeviceKind::Relay, Coil),
        (DeviceKind::Relay, Contact),
        (DeviceKind::Breaker, Whole),
        (DeviceKind::Contactor, Whole),
        (DeviceKind::Motor, Whole),
    ] {
        for on in [SheetId::Power(0), SheetId::Control(0)] {
            let mut project = crab_core::Project::load(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../examples/pump"
            ))
            .unwrap();
            let (doc0, _) = build_document_in(&project, &lib, Lang::Ru, Format::A3).unwrap();
            let number = doc0.sheets.iter().position(|s| s.id == Some(on)).unwrap() + 1;
            let id = drop_new(
                &mut project,
                PalettePayload { kind, part },
                on,
                Some(number),
                (300.0, 300.0),
            );
            let (doc, _) = build_document_in(&project, &lib, Lang::Ru, Format::A3).unwrap();
            let sheet = doc.sheets.iter().find(|s| s.id == Some(on)).unwrap();
            let drawn = sheet
                .items
                .iter()
                .any(|i| i.device().is_some_and(|d| crab_core::split_part(d).0 == id));
            assert!(drawn, "{kind:?} {part:?} on {on:?}: {id} not drawn");
        }
    }
}

#[test]
fn a_coil_drawn_by_the_generator_cannot_be_dropped_but_a_hand_coil_moves() {
    let mut coil_at = crab_schematic::Xrefs::new();
    coil_at.insert(
        "K1".to_owned(),
        crab_schematic::Xref {
            sheet: 3,
            zone: "2".to_owned(),
        },
    );
    // The generator draws K1 (its coil in the ladder): the device cannot be dropped, but its
    // coil can: a coil put by hand is drawn as its own symbol, the ladder gives way to it.
    assert!(drop_refusal(&[], &coil_at, "K1/coil").is_none());
    assert!(drop_refusal(&[], &coil_at, "K1").is_some());
    // A coil already put by hand is drawn as its own symbol: it can be moved to another sheet.
    let hand = sheet_of(Vec::new(), vec![symbol("control", "K1/coil", Vec::new())]);
    assert!(drop_refusal(&[hand], &coil_at, "K1/coil").is_none());
    // A device whose coil nobody draws can be dropped, and so can its coil.
    assert!(drop_refusal(&[], &coil_at, "K2/coil").is_none());
    assert!(drop_refusal(&[], &coil_at, "K2").is_none());
}

#[test]
fn the_topmost_symbol_under_a_point_is_found() {
    let sheet = sheet_of(
        vec![
            Op::Rect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 100.0,
                width: 1.0,
                rgb: (0, 0, 0),
            },
            Op::Rect {
                x: 50.0,
                y: 50.0,
                w: 100.0,
                h: 100.0,
                width: 1.0,
                rgb: (0, 0, 0),
            },
        ],
        vec![
            symbol("power", "Q1", vec![0]),
            symbol("power", "K1", vec![1]),
        ],
    );
    let at = |x: f32, y: f32| symbol_at(&sheet, Pos2::new(x, y)).and_then(Item::device);
    assert_eq!(at(10.0, 10.0), Some("Q1"));
    // Where they overlap, the one drawn last is on top.
    assert_eq!(at(70.0, 70.0), Some("K1"));
    assert_eq!(at(400.0, 400.0), None);
}

#[test]
fn only_a_symbol_of_a_device_view_can_be_moved() {
    assert_eq!(
        movable(&symbol("power", "K1", vec![])),
        Some(("K1", "power:K1".to_owned()))
    );
    assert_eq!(movable(&symbol("board", "A1", vec![])), None);
}

#[test]
fn the_middle_segment_of_a_routed_wire_is_a_handle_but_a_straight_wire_is_not() {
    let wire = |key: &str, op: usize| Item {
        kind: ItemKind::Wire {
            key: key.to_owned(),
            from: crab_schematic::End::new("A", "out:0"),
            to: crab_schematic::End::new("B", "in:0"),
        },
        ops: vec![op],
    };
    let sheet = sheet_of(
        vec![
            line(&[(0.0, 0.0), (0.0, 50.0), (40.0, 50.0), (40.0, 100.0)]),
            line(&[(200.0, 0.0), (200.0, 100.0)]),
        ],
        vec![wire("routed", 0), wire("straight", 1)],
    );
    let hit = |x: f32, y: f32| bend_at(&sheet, Pos2::new(x, y), 4.0);
    assert_eq!(hit(20.0, 51.0), Some(("routed", 50.0)));
    // Off the segment, and on a vertical part.
    assert_eq!(hit(20.0, 70.0), None);
    assert_eq!(hit(0.0, 20.0), None);
    // A straight wire has nothing to bend.
    assert_eq!(hit(200.0, 50.0), None);
}

fn text(x: f32, y: f32, text: &str) -> Op {
    Op::Text {
        x,
        y,
        text: text.to_owned(),
        size: 17.0,
        align: Align::Left,
        bold: false,
        vertical: false,
        rgb: (0, 0, 0),
    }
}

#[test]
fn a_reference_under_a_point_names_its_sheet_and_zone() {
    let sheet = sheet_of(
        vec![
            text(100.0, 50.0, "/3.4"),
            text(100.0, 200.0, "K1"),
            text(300.0, 50.0, "13 /.2"),
        ],
        Vec::new(),
    );
    assert_eq!(
        ref_at(&sheet, Pos2::new(120.0, 50.0)),
        Some((Some(3), "4".to_owned()))
    );
    // The sheet is left out for a column of the same sheet.
    assert_eq!(
        ref_at(&sheet, Pos2::new(330.0, 50.0)),
        Some((None, "2".to_owned()))
    );
    // A caption that is no reference, and the empty places, are not links.
    assert_eq!(ref_at(&sheet, Pos2::new(110.0, 200.0)), None);
    assert_eq!(ref_at(&sheet, Pos2::new(500.0, 500.0)), None);
}

#[test]
fn a_contact_is_in_one_circuit_only() {
    let mut contact = Device {
        id: "Q1_1".into(),
        kind: DeviceKind::Aux,
        via: String::new(),
        rating: String::new(),
        part: String::new(),
        bind: "button".into(),
        load: String::new(),
        poles: None,
        of: "Q1".into(),
        form: ContactForm::No,
        pins: String::new(),
        from: String::new(),
        free: false,
        series: String::new(),
        conductors: Vec::new(),
    };
    set_series(&mut contact, "K1");
    assert_eq!((contact.series.as_str(), contact.bind.as_str()), ("K1", ""));
    // Back to an input of the board: the coil is let go, the input is chosen again by hand.
    set_series(&mut contact, "");
    assert_eq!(contact.series, "");
}

#[test]
fn only_breakers_rcds_contactors_and_relays_have_contacts() {
    for kind in DeviceKind::ALL {
        let expected = matches!(
            kind,
            DeviceKind::Breaker | DeviceKind::Rcd | DeviceKind::Contactor | DeviceKind::Relay
        );
        assert_eq!(can_have_contacts(kind), expected, "{kind:?}");
    }
    assert!(!can_have_contacts(DeviceKind::Aux));
}
