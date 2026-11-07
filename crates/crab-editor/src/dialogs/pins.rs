use super::*;

/// Board pins and the pins other blocks of the schema take, for the pin list.
pub(super) struct Pins {
    /// `None` for a board without a pin table: the number is typed then.
    pub(super) layout: Option<BoardLayout>,
    pub(super) others: Vec<PinUse>,
    /// Expanders of the project with the pins taken on each, for blocks that may stand
    /// on one (empty otherwise).
    pub(super) expanders: Vec<ExpanderPins>,
    /// The block type may stand on an expander pin.
    pub(super) can_expander: bool,
    /// Id a new expander would get.
    pub(super) free_id: String,
}

pub(super) struct ExpanderPins {
    pub(super) expander: Expander,
    /// Pin of the chip and the block that holds it.
    pub(super) taken: Vec<(i64, String)>,
}

/// Chooses where the pin parameter points: a board GPIO, an expander of the project or
/// a new expander. A change of source clears the pin, since its number means another thing.
pub(super) fn pin_source_combo(ui: &mut Ui, p: &ParamDef, d: &mut BlockDialog, pins: &Pins) {
    let label = |source: &PinSource| match source {
        PinSource::Gpio => crate::i18n::tr("GPIO платы").to_owned(),
        PinSource::Expander(id) => match pins.expanders.iter().find(|e| e.expander.id == *id) {
            Some(e) => expander_title(&e.expander),
            None => crate::i18n::fmt("{0} (нет в проекте)", &[&id]),
        },
        PinSource::New => crate::i18n::tr("+ добавить расширитель…").to_owned(),
    };
    let mut chosen = None;
    egui::ComboBox::from_id_salt(("pin_source", &p.name))
        .selected_text(label(&d.source))
        .show_ui(ui, |ui| {
            let mut options = vec![PinSource::Gpio];
            options.extend(
                pins.expanders
                    .iter()
                    .map(|e| PinSource::Expander(e.expander.id.clone())),
            );
            options.push(PinSource::New);
            for option in options {
                if ui
                    .selectable_label(d.source == option, label(&option))
                    .clicked()
                {
                    chosen = Some(option);
                }
            }
        })
        .response
        .on_hover_text(crate::i18n::tr(
            "Вывод платы или вывод микросхемы-расширителя PCF по I²C",
        ));
    if let Some(source) = chosen.filter(|s| *s != d.source) {
        d.source = source;
        d.values.insert(p.name.clone(), None);
    }
}

/// Pins P0… of the chosen expander; pins other blocks hold are shown but cannot be chosen.
pub(super) fn expander_pin_combo(ui: &mut Ui, p: &ParamDef, d: &mut BlockDialog, pins: &Pins) {
    let (chip, taken, chosen_chip): (ExpanderChip, &[(i64, String)], &Expander) = match &d.source {
        PinSource::Expander(id) => match pins.expanders.iter().find(|e| e.expander.id == *id) {
            Some(e) => (e.expander.chip, &e.taken, &e.expander),
            None => {
                ui.colored_label(
                    Color32::from_rgb(186, 74, 66),
                    crate::i18n::tr("расширителя нет в проекте"),
                );
                return;
            }
        },
        _ => (d.pending.chip, &[], &d.pending),
    };
    let current = match d.values.get(&p.name) {
        Some(Some(ParamValue::Int(i))) => Some(*i),
        _ => None,
    };
    let selected = match current {
        Some(v) if (0..i64::from(chip.pins())).contains(&v) => format!("P{v}"),
        Some(v) => crate::i18n::fmt("P{0} (нет у {1})", &[&v, &chip.name()]),
        None => crate::i18n::tr("выбрать…").to_owned(),
    };
    let rows = expander_rows(p, chip, taken, None, |pin| {
        PinChoice::Expander(chosen_chip.id.clone(), pin)
    });
    let mut chosen = None;
    egui::ComboBox::from_id_salt(("expander_pin", &p.name))
        .selected_text(selected)
        .height(400.0)
        .show_ui(ui, |ui| {
            for row in rows {
                if let PinRow::Pin {
                    choice: PinChoice::Expander(_, pin),
                    text,
                    free,
                } = row
                {
                    let button = egui::Button::selectable(current == Some(pin), text);
                    if ui.add_enabled(free, button).clicked() {
                        chosen = Some(pin);
                    }
                }
            }
        });
    if let Some(pin) = chosen {
        d.values.insert(p.name.clone(), Some(ParamValue::Int(pin)));
    }
    if current.is_none() {
        ui.colored_label(Color32::from_rgb(186, 74, 66), crate::i18n::tr("не задан"));
    }
}

/// What a row of the pin list stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PinChoice {
    /// A GPIO of the board (`-1`: not used).
    Gpio(i64),
    /// A pin of an expander of the project.
    Expander(String, i64),
    /// A pin of an expander soldered on the board that the project does not have yet
    /// (index in the board's list); choosing it declares the expander in the project.
    Board(usize, i64),
    /// Declare an expander that is not on the board.
    NewExpander,
}

/// A row of the pin list: a group heading or something to choose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PinRow {
    Heading(String),
    Pin {
        choice: PinChoice,
        text: String,
        free: bool,
    },
}

/// `P3` or `P3 · K4` for a pin of an expander.
pub(super) fn expander_pin_name(pin: i64, label: &str) -> String {
    if label.is_empty() {
        format!("P{pin}")
    } else {
        format!("P{pin} · {label}")
    }
}

/// Rows for the pins of an expander chip. With a `board` table only the pins the board
/// brings out are listed, under the names printed on it; pins other blocks hold and pins
/// without the function of the parameter are shown but cannot be chosen.
pub(super) fn expander_rows(
    p: &ParamDef,
    chip: ExpanderChip,
    taken: &[(i64, String)],
    board: Option<&crab_core::pins::BoardExpander>,
    choice: impl Fn(i64) -> PinChoice,
) -> Vec<PinRow> {
    let mut rows = Vec::new();
    for pin in 0..i64::from(chip.pins()) {
        let mut name = format!("P{pin}");
        let mut fits = true;
        if let Some(board) = board {
            let Some(info) = board.pin(pin) else { continue };
            name = expander_pin_name(pin, &info.label);
            fits = board.fits(pin, p.mode);
        }
        let holder = taken.iter().find(|(n, _)| *n == pin).map(|(_, b)| b);
        let text = match holder {
            Some(block) => crate::i18n::fmt("{0} — занят: «{1}»", &[&name, &block]),
            None if !fits => crate::i18n::fmt("{0} — не подходит блоку", &[&name]),
            None => name,
        };
        rows.push(PinRow::Pin {
            choice: choice(pin),
            text,
            free: holder.is_none() && fits,
        });
    }
    rows
}

/// Rows for the GPIOs of the board that fit the parameter; pins taken by other blocks
/// (or other parameters of this one) are shown but cannot be chosen.
pub(super) fn gpio_rows(
    p: &ParamDef,
    d: &BlockDialog,
    def: &BlockDef,
    pins: &Pins,
    layout: &BoardLayout,
) -> Vec<PinRow> {
    let me = PinUse {
        block: d.id.clone(),
        kind: d.kind.clone(),
        param: p.name.clone(),
        gpio: 0,
        mode: p.mode,
        shared: p.shared,
        bus: p.bus.clone(),
    };
    // Other pin parameters of this block, with the values being edited.
    let own: Vec<(i64, String)> = def
        .params
        .iter()
        .filter(|q| q.kind == ParamKind::Pin && q.name != p.name)
        .filter_map(|q| {
            let v = d
                .values
                .get(&q.name)
                .cloned()
                .flatten()
                .or(q.default.clone());
            match v {
                Some(ParamValue::Int(g)) if g >= 0 => {
                    Some((g, crate::i18n::fmt("«{0}» этого блока", &[&q.title()])))
                }
                _ => None,
            }
        })
        .collect();
    let mut rows = Vec::new();
    if p.min.is_some_and(|min| min < 0.0) {
        rows.push(PinRow::Pin {
            choice: PinChoice::Gpio(-1),
            text: crate::i18n::tr("не используется").to_owned(),
            free: true,
        });
    }
    for pin in layout.pins().iter().filter(|pin| pin.fits(p.mode)) {
        let mut text = pin.title();
        let notes = pin.notes();
        if !notes.is_empty() {
            text += &format!(" · {notes}");
        }
        let taken = pins
            .others
            .iter()
            .find(|u| u.gpio == pin.gpio && !u.can_share(&me))
            .map(|u| format!("«{}»", u.block))
            .or_else(|| {
                own.iter()
                    .find(|(g, _)| *g == pin.gpio)
                    .map(|(_, who)| who.clone())
            });
        let free = taken.is_none();
        if let Some(who) = taken {
            text += &crate::i18n::fmt(" — занят: {0}", &[&who]);
        } else if let Some(u) = pins.others.iter().find(|u| u.gpio == pin.gpio) {
            text += &crate::i18n::fmt(" — общий с «{0}»", &[&u.block]);
        }
        rows.push(PinRow::Pin {
            choice: PinChoice::Gpio(pin.gpio),
            text,
            free,
        });
    }
    rows
}

/// Index in the board's list of the expander the dialog declares, if it is one of them.
pub(super) fn pending_board_index(d: &BlockDialog, pins: &Pins) -> Option<usize> {
    let layout = pins.layout.as_ref()?;
    layout.expanders().iter().position(|b| b.is(&d.pending))
}

/// The whole pin list of a parameter: GPIOs of the board first, then, for blocks that can
/// stand on an expander, the pins of the project's expanders and of the ones soldered on
/// the board. A row of an expander also tells where the pin is: no separate source.
pub(super) fn pin_rows(p: &ParamDef, d: &BlockDialog, def: &BlockDef, pins: &Pins) -> Vec<PinRow> {
    let Some(layout) = pins.layout.as_ref() else {
        return Vec::new();
    };
    let mut rows = gpio_rows(p, d, def, pins, layout);
    if !(pins.can_expander && pin_param(def).is_some_and(|q| q.name == p.name)) {
        return rows;
    }
    for e in &pins.expanders {
        rows.push(PinRow::Heading(expander_title(&e.expander)));
        rows.extend(expander_rows(
            p,
            e.expander.chip,
            &e.taken,
            layout.expander_for(&e.expander),
            |pin| PinChoice::Expander(e.expander.id.clone(), pin),
        ));
    }
    // Expanders soldered on the board that the project does not use yet.
    for (i, b) in layout.expanders().iter().enumerate() {
        if pins.expanders.iter().any(|e| b.is(&e.expander)) {
            continue;
        }
        let list = expander_rows(p, b.expander.chip, &[], Some(b), |pin| {
            PinChoice::Board(i, pin)
        });
        if list
            .iter()
            .any(|r| matches!(r, PinRow::Pin { free: true, .. }))
        {
            rows.push(PinRow::Heading(crate::i18n::fmt(
                "Плата: {0} · 0x{1}",
                &[&b.title, &format!("{:02X}", b.expander.address)],
            )));
            rows.extend(list);
        }
    }
    rows
}

/// What the dialog points at now, in terms of the list rows.
pub(super) fn current_choice(
    d: &BlockDialog,
    p: &ParamDef,
    pins: &Pins,
    gpio: Option<i64>,
) -> Option<PinChoice> {
    let own = match d.values.get(&p.name) {
        Some(Some(ParamValue::Int(i))) => Some(*i),
        _ => None,
    };
    match &d.source {
        PinSource::Gpio => gpio.map(PinChoice::Gpio),
        PinSource::Expander(id) => own.map(|v| PinChoice::Expander(id.clone(), v)),
        PinSource::New => pending_board_index(d, pins)
            .zip(own)
            .map(|(i, v)| PinChoice::Board(i, v)),
    }
}

/// Text of the closed list.
pub(super) fn pin_list_caption(
    p: &ParamDef,
    d: &BlockDialog,
    pins: &Pins,
    layout: &BoardLayout,
    gpio: Option<i64>,
) -> String {
    let own = match d.values.get(&p.name) {
        Some(Some(ParamValue::Int(i))) => Some(*i),
        _ => None,
    };
    let on_chip = |chip: ExpanderChip, board: Option<&crab_core::pins::BoardExpander>, v: i64| {
        let label = board
            .and_then(|b| b.pin(v))
            .map_or("", |i| i.label.as_str());
        if (0..i64::from(chip.pins())).contains(&v) {
            expander_pin_name(v, label)
        } else {
            crate::i18n::fmt("P{0} (нет у {1})", &[&v, &chip.name()])
        }
    };
    match &d.source {
        PinSource::Gpio => match gpio {
            None => crate::i18n::tr("выбрать…").to_owned(),
            Some(v) if v < 0 => crate::i18n::tr("не используется").to_owned(),
            Some(v) => match layout.pin(v).filter(|pin| pin.fits(p.mode)) {
                Some(pin) => pin.title(),
                None => crate::i18n::fmt("GPIO{0} (не подходит)", &[&v]),
            },
        },
        PinSource::Expander(id) => match pins.expanders.iter().find(|e| e.expander.id == *id) {
            None => crate::i18n::fmt("{0} (нет в проекте)", &[&id]),
            Some(e) => match own {
                None => crate::i18n::fmt("{0} · выбрать…", &[&e.expander.id]),
                Some(v) => format!(
                    "{} · {}",
                    e.expander.id,
                    on_chip(e.expander.chip, layout.expander_for(&e.expander), v)
                ),
            },
        },
        PinSource::New => {
            let board = layout.expander_for(&d.pending);
            let who = match board {
                Some(b) => b.title.clone(),
                None => crate::i18n::tr("новый расширитель").to_owned(),
            };
            match own {
                None => crate::i18n::fmt("{0} · выбрать…", &[&who]),
                Some(v) => format!("{who} · {}", on_chip(d.pending.chip, board, v)),
            }
        }
    }
}

/// Drop-down list with every pin the parameter can use: GPIOs of the board and, for a
/// digital input or output, pins of expanders. Picking a row sets the source and the pin.
pub(super) fn pin_list_combo(
    ui: &mut Ui,
    p: &ParamDef,
    d: &mut BlockDialog,
    def: &BlockDef,
    pins: &Pins,
    gpio: Option<i64>,
) {
    let Some(layout) = pins.layout.as_ref() else {
        return;
    };
    let rows = pin_rows(p, d, def, pins);
    let current = current_choice(d, p, pins, gpio);
    let expander_pin = pins.can_expander && pin_param(def).is_some_and(|q| q.name == p.name);
    let mut chosen = None;
    egui::ComboBox::from_id_salt(("pin", &p.name))
        .selected_text(pin_list_caption(p, d, pins, layout, gpio))
        .height(400.0)
        .show_ui(ui, |ui| {
            for row in rows {
                match row {
                    PinRow::Heading(title) => {
                        ui.add_space(4.0);
                        ui.weak(title);
                    }
                    PinRow::Pin { choice, text, free } => {
                        let button =
                            egui::Button::selectable(current.as_ref() == Some(&choice), text);
                        if ui.add_enabled(free, button).clicked() {
                            chosen = Some(choice);
                        }
                    }
                }
            }
            if expander_pin {
                ui.separator();
                let own_expander =
                    d.source == PinSource::New && pending_board_index(d, pins).is_none();
                if ui
                    .selectable_label(own_expander, crate::i18n::tr("+ добавить расширитель…"))
                    .clicked()
                {
                    chosen = Some(PinChoice::NewExpander);
                }
            }
        })
        .response
        .on_hover_text(crate::i18n::fmt("Выводы {0}", &[&layout.name()]));
    let unset = if d.source == PinSource::Gpio {
        gpio.is_none()
    } else {
        !has_pin(d, p)
    };
    if unset {
        ui.colored_label(Color32::from_rgb(186, 74, 66), crate::i18n::tr("не задан"));
    }
    let Some(choice) = chosen else { return };
    let name = p.name.clone();
    match choice {
        PinChoice::Gpio(g) => {
            d.source = PinSource::Gpio;
            d.values.insert(name, Some(ParamValue::Int(g)));
        }
        PinChoice::Expander(id, pin) => {
            d.source = PinSource::Expander(id);
            d.values.insert(name, Some(ParamValue::Int(pin)));
        }
        PinChoice::Board(i, pin) => {
            if let Some(board) = layout.expanders().get(i) {
                if pending_board_index(d, pins) != Some(i) || d.source != PinSource::New {
                    d.pending = board_expander(board, &pins.free_id);
                }
                d.source = PinSource::New;
                d.values.insert(name, Some(ParamValue::Int(pin)));
            }
        }
        PinChoice::NewExpander => {
            d.source = PinSource::New;
            d.pending = blank_expander(&pins.free_id);
            d.values.insert(name, None);
        }
    }
}

/// The dialog has a pin number for the parameter.
pub(super) fn has_pin(d: &BlockDialog, p: &ParamDef) -> bool {
    matches!(d.values.get(&p.name), Some(Some(ParamValue::Int(_))))
}
