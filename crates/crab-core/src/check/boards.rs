use super::*;

/// Blocks on pins of an expander that the board carries: the pin must be one the board
/// brings out and must do what the block wants.
pub(super) fn check_board_expanders(
    project: &Project,
    lib: &Library,
    layout: &BoardLayout,
    report: &mut Report,
) {
    for u in crate::expander::expander_uses(&project.logic, lib) {
        let Some(e) = project.logic.expander(&u.expander) else {
            continue;
        };
        let Some(board) = layout.expander_for(e) else {
            continue;
        };
        let id = u.block.as_str();
        if u.pin < 0 || u.pin >= i64::from(e.chip.pins()) {
            continue;
        }
        let problem = if board.pin(u.pin).is_none() {
            "не выведен на плате"
        } else if board.fits(u.pin, u.mode) {
            continue;
        } else {
            match u.mode {
                crate::PinMode::In => "не работает как цифровой вход",
                crate::PinMode::Out => "не работает как цифровой выход",
                _ => "не работает как цифровой вход и выход",
            }
        };
        report.block_error(
            DiagKind::ExpanderPin,
            id,
            format!(
                "блок «{id}»: вывод P{} расширителя «{}» {problem} у {}",
                u.pin,
                u.expander,
                layout.name()
            ),
        );
    }
}

/// A block made for a user board (its relays) works only in a project of that board.
pub(super) fn check_board_blocks(project: &Project, lib: &Library, report: &mut Report) {
    let model = project.config.board.model.as_deref();
    for block in &project.logic.blocks {
        let Some(owner) = lib
            .get(&block.kind)
            .and_then(|b| b.def.info.board.as_deref())
            .filter(|&owner| Some(owner) != model)
        else {
            continue;
        };
        let owner_name = lib
            .boards()
            .get(owner)
            .map_or(owner, |b| b.def.info.name.as_str());
        let id = block.id.as_str();
        report.block_error(
            DiagKind::BoardBlock,
            id,
            format!(
                "блок «{id}»: тип «{}» сделан для платы «{owner_name}», а в проекте выбрана другая; выберите эту плату в настройках проекта или замените блок",
                block.kind
            ),
        );
    }
}

/// Declarations of the expanders and the blocks standing on their pins.
pub(super) fn check_expanders(logic: &Logic, lib: &Library, report: &mut Report) {
    let mut ids: HashSet<&str> = HashSet::new();
    for (i, e) in logic.expanders.iter().enumerate() {
        let message = if !ids.insert(e.id.as_str()) {
            Some(format!("расширитель «{}» объявлен дважды", e.id))
        } else if let Some(problem) = e.problem() {
            Some(problem)
        } else {
            logic.expanders[..i]
                .iter()
                .find(|o| o.address == e.address && o.sda == e.sda && o.scl == e.scl)
                .map(|o| {
                    format!(
                        "расширитель «{}»: адрес 0x{:02X} на этой шине уже занят расширителем «{}»",
                        e.id, e.address, o.id
                    )
                })
        };
        if let Some(message) = message {
            report.push(DiagKind::Expander, Severity::Error, None, None, message);
        }
    }
    for block in logic.blocks.iter().filter(|b| b.tag.is_none()) {
        let Some(name) = &block.expander else {
            continue;
        };
        let id = block.id.as_str();
        let supported = lib
            .resolve(logic, block)
            .is_none_or(|l| l.def.info.expander);
        if !supported {
            report.block_error(
                DiagKind::Expander,
                id,
                format!(
                    "блок «{id}»: тип «{}» не может стоять на выводе расширителя",
                    block.kind
                ),
            );
        } else if logic.expander(name).is_none() {
            report.block_error(
                DiagKind::Expander,
                id,
                format!("блок «{id}»: расширитель «{name}» не найден"),
            );
        }
    }
    let uses = crate::expander::expander_uses(logic, lib);
    for (i, u) in uses.iter().enumerate() {
        let Some(e) = logic.expander(&u.expander) else {
            continue;
        };
        let id = u.block.as_str();
        if u.pin < 0 || u.pin >= i64::from(e.chip.pins()) {
            report.block_error(
                DiagKind::ExpanderPin,
                id,
                format!(
                    "блок «{id}»: у {} расширителя «{}» нет вывода P{}, допустимы P0–P{}",
                    e.chip.name(),
                    e.id,
                    u.pin,
                    e.chip.pins() - 1
                ),
            );
        } else if let Some(first) = uses[..i]
            .iter()
            .find(|o| o.expander == u.expander && o.pin == u.pin)
        {
            report.block_error(
                DiagKind::ExpanderPin,
                id,
                format!(
                    "блок «{id}»: вывод P{} расширителя «{}» уже занят блоком «{}»",
                    u.pin, u.expander, first.block
                ),
            );
        }
    }
}

/// Two blocks (or two parameters of one block) on one pin, unless the pin is
/// shared on purpose (a 1-Wire bus).
pub(super) fn check_pin_conflicts(uses: &[PinUse], report: &mut Report) {
    for (i, u) in uses.iter().enumerate() {
        let Some(first) = uses[..i]
            .iter()
            .find(|other| other.gpio == u.gpio && !other.can_share(u))
        else {
            continue;
        };
        let message = if first.block == u.block {
            format!(
                "блок «{}»: параметры «{}» и «{}» на одном выводе GPIO{}",
                u.block, first.param, u.param, u.gpio
            )
        } else {
            format!(
                "блок «{}»: вывод GPIO{} уже занят блоком «{}»",
                u.block, u.gpio, first.block
            )
        };
        report.block_error(DiagKind::PinBusy, &u.block, message);
    }
}

/// Pins a block uses against the board: missing ones and unfit ones are errors, the
/// pin's own remark a warning, except for the `quiet` blocks.
pub(super) fn check_board_pins(
    layout: &BoardLayout,
    uses: &[PinUse],
    quiet: &HashSet<&str>,
    report: &mut Report,
) {
    for u in uses {
        let (id, gpio) = (u.block.as_str(), u.gpio);
        let Some(pin) = layout.pin(gpio) else {
            report.block_error(
                DiagKind::PinUnavailable,
                id,
                format!("блок «{id}»: у {} нет вывода GPIO{gpio}", layout.name()),
            );
            continue;
        };
        let problem = if pin.flash {
            Some("занят флеш-памятью")
        } else if !pin.fits(u.mode) {
            Some(match u.mode {
                crate::PinMode::Adc => "без АЦП",
                crate::PinMode::Dac => "без ЦАП",
                crate::PinMode::Pwm => "не умеет ШИМ",
                crate::PinMode::In => "не работает как цифровой вход",
                _ if pin.input_only => "работает только на вход",
                crate::PinMode::Out => "не работает как цифровой выход",
                crate::PinMode::Io => "не работает как цифровой вход и выход",
            })
        } else {
            None
        };
        if let Some(problem) = problem {
            report.block_error(
                DiagKind::PinUnavailable,
                id,
                format!(
                    "блок «{id}»: вывод GPIO{gpio} {problem} у {}",
                    layout.name()
                ),
            );
        } else if let Some(warning) = pin.warning.as_ref().filter(|_| !quiet.contains(id)) {
            report.push(
                DiagKind::PinWarning,
                Severity::Warning,
                Some(id),
                None,
                format!("блок «{id}»: вывод GPIO{gpio} — {warning}"),
            );
        }
    }
}
