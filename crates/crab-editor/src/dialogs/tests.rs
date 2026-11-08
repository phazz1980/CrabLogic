use super::*;

#[test]
fn new_expander_is_valid_and_gets_a_free_id() {
    let mut logic = Logic::default();
    let first = new_expander(Some(&logic));
    assert_eq!(first.id, "pcf1");
    assert_eq!(first.problem(), None);
    logic.expanders.push(first);
    assert_eq!(new_expander(Some(&logic)).id, "pcf2");
}

#[test]
fn a_board_expander_becomes_the_projects_own_under_a_free_id() {
    use crab_core::library::PinMode;
    use crab_core::pins::{BoardExpander, BoardExpanderPin, PinFunction};
    let mut chip = Expander::new("relays", ExpanderChip::Pcf8574, 0x24);
    chip.sda = 4;
    chip.scl = 5;
    let board = BoardExpander {
        title: "Реле".into(),
        expander: chip,
        pins: vec![BoardExpanderPin {
            pin: 0,
            label: "K1".into(),
            functions: [PinFunction::Dout].into(),
        }],
    };
    let e = board_expander(&board, "pcf1");
    assert_eq!(e.id, "pcf1");
    assert!(board.is(&e));
    assert_eq!(e.problem(), None);
    // A relay takes an output block, not an input one.
    assert!(board.fits(0, PinMode::Out));
    assert!(!board.fits(0, PinMode::In));
    // The same chip under another address is no longer the board's.
    let mut other = e.clone();
    other.address = 0x25;
    assert!(!board.is(&other));
}

#[test]
fn expander_rows_list_the_pins_of_the_board_with_their_names() {
    use crab_core::pins::{BoardExpander, BoardExpanderPin, PinFunction};
    use crab_core::Library;
    let (lib, _) = Library::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library")).unwrap();
    let pin_param = |kind: &str| {
        lib.get(kind)
            .unwrap()
            .def
            .params
            .iter()
            .find(|p| p.kind == ParamKind::Pin)
            .unwrap()
            .clone()
    };
    let pin = |pin, label: &str| BoardExpanderPin {
        pin,
        label: label.into(),
        functions: [PinFunction::Dout].into(),
    };
    let board = BoardExpander {
        title: "Реле".into(),
        expander: Expander::new("relays", ExpanderChip::Pcf8574, 0x24),
        pins: vec![pin(0, "K1"), pin(1, "K2")],
    };
    let choice = |n| PinChoice::Board(0, n);
    let taken = [(1, "лампа".to_owned())];

    let rows = expander_rows(
        &pin_param("digital_out"),
        ExpanderChip::Pcf8574,
        &taken,
        Some(&board),
        choice,
    );
    let row = |choice, text: &str, free| PinRow::Pin {
        choice,
        text: text.into(),
        free,
    };
    assert_eq!(
        rows,
        [
            row(choice(0), "P0 · K1", true),
            row(choice(1), "P1 · K2 — занят: «лампа»", false),
        ]
    );

    // The relays only drive levels: an input block cannot stand on them.
    let rows = expander_rows(
        &pin_param("digital_in"),
        ExpanderChip::Pcf8574,
        &[],
        Some(&board),
        choice,
    );
    assert_eq!(
        rows[0],
        row(choice(0), "P0 · K1 — не подходит блоку", false)
    );

    // An expander the board knows nothing about offers every pin of the chip.
    let rows = expander_rows(
        &pin_param("digital_out"),
        ExpanderChip::Pcf8574,
        &[],
        None,
        choice,
    );
    assert_eq!(rows.len(), 8);
}

#[test]
fn init_values() {
    assert_eq!(
        parse_init(VarType::Float, "22,5"),
        Ok(Some(ParamValue::Float(22.5)))
    );
    assert_eq!(
        parse_init(VarType::Bool, "да"),
        Ok(Some(ParamValue::Bool(true)))
    );
    assert_eq!(parse_init(VarType::Int, ""), Ok(None));
    assert!(parse_init(VarType::Int, "1.5").is_err());
}
