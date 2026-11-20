//! Schema check: block types, parameters, links, and execution order.
//!
//! The check never stops at the first problem: it collects all diagnostics so the
//! editor and the AI can show everything at once.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::fmt;

use crate::ident::is_valid_ident;
use crate::library::{BlockDef, ParamDef, ParamKind, PortDef, SignalType};
use crate::pins::{pin_uses, BoardLayout, PinUse};
use crate::value::parse_duration_ms;
use crate::{Layout, Library, Link, Logic, ParamValue, PortRef, Project};

mod arrays;
mod blocks;
mod boards;
mod cycle;
mod display;
mod flow;
mod graph;
mod lora;
mod modbus;
mod retain;
#[cfg(test)]
mod tests;
mod types;
#[cfg(test)]
mod wifi_tests;

pub use self::blocks::*;
use self::boards::*;
use self::cycle::check_cycle;
pub use self::display::{screen_size, LCD_CYRILLIC_LIMIT};
pub use self::flow::*;
use self::graph::*;
use self::retain::check_retain_fast;
pub use self::types::*;

/// Checks the schema against the library. `layout` is optional and only produces warnings.
pub fn check(logic: &Logic, lib: &Library, layout: Option<&Layout>) -> Report {
    let mut report = Report::default();
    let defs = check_blocks(logic, lib, &mut report);
    let mut edges = check_links(logic, &defs, &mut report);
    edges.extend(check_vars(logic, &defs, &mut report));
    edges.extend(check_tags(logic, &defs, &mut report));
    check_unconnected_inputs(logic, &defs, &mut report);
    check_retain_fast(logic, &defs, &mut report);
    if let Some(layout) = layout {
        check_layout(logic, layout, &mut report);
    }
    check_pin_conflicts(&pin_uses(logic, lib), &mut report);
    check_expanders(logic, lib, &mut report);
    display::check_displays(logic, &mut report);
    lora::check_lora(logic, &mut report);
    arrays::check_arrays(logic, &mut report);
    arrays::check_array_blocks(logic, &mut report);
    arrays::check_struct_blocks(logic, &mut report);
    order_blocks(logic, &defs, &edges, &mut report);
    if !report.has_errors() {
        check_number_flow(logic, &defs, &mut report);
        check_float_accumulators(logic, &defs, &mut report);
    }
    report
}

/// [`check`] with the layout plus what depends on the board: pins the chip lacks
/// or that do not fit the block, and the UART settings.
pub fn check_project(project: &Project, lib: &Library) -> Report {
    let links = project.enabled_links();
    // Network blocks of a project with no network stay in it but are not checked: say so.
    let skipped_network: Vec<String> = if project.network_off() {
        project
            .network_blocks()
            .into_iter()
            .map(String::from)
            .collect()
    } else {
        Vec::new()
    };
    let project = &project.without_disabled();
    let mut report = check(&project.logic, lib, Some(&project.layout));
    if !skipped_network.is_empty() {
        report.push(
            DiagKind::NetworkOff,
            Severity::Warning,
            None,
            None,
            format!(
                "блоки сети не собираются, пока выключены Wi-Fi, Ethernet и GSM: {}",
                skipped_network.join(", ")
            ),
        );
    }
    // An unused tag waits in the project tree: its inputs are not fed yet.
    let unused = project.unused_tags(lib);
    // The same for a widget of the web page: its images on the sheets are fed, not it.
    report.diagnostics.retain(|d| {
        d.kind != DiagKind::InputEmpty
            || d.block
                .as_ref()
                .is_none_or(|b| !unused.contains(b) && !project.is_widget(b))
    });
    // Link indices refer to the project's own list, not to the trimmed one.
    for d in &mut report.diagnostics {
        d.link = d.link.and_then(|i| links.get(i).copied());
    }
    check_board_blocks(project, lib, &mut report);
    crate::modbus_tags::check_externals(project, &mut report);
    modbus::check_modbus(&project.logic, &mut report);
    let layout = BoardLayout::of_board(&project.config.board, lib.boards());
    let pins = pin_uses(&project.logic, lib);
    let uarts = crate::uart::check_project_part(
        project,
        lib,
        &pins,
        layout.as_ref().map(BoardLayout::chip),
        &mut report,
    );
    if let Some(layout) = &layout {
        // The blocks of the board's own devices sit on the pins the board has fixed for
        // them, so the board's remarks about those pins ("taken by the bus") are not for them.
        let own: HashSet<&str> = project
            .logic
            .blocks
            .iter()
            .filter(|b| lib.get(&b.kind).is_some_and(|l| l.def.info.board.is_some()))
            .map(|b| b.id.as_str())
            .collect();
        check_board_pins(layout, &pins, &own, &mut report);
        let uart_pins: Vec<PinUse> = uarts.iter().flat_map(|u| u.pin_uses()).collect();
        check_board_pins(layout, &uart_pins, &own, &mut report);
        check_board_expanders(project, lib, layout, &mut report);
    }
    check_lan8720(project, layout.as_ref().map(BoardLayout::chip), &mut report);
    check_counters(project, layout.as_ref().map(BoardLayout::chip), &mut report);
    check_cycle(project, &mut report);
    arrays::check_array_limits(
        &project.logic,
        layout.as_ref().map(BoardLayout::chip),
        &mut report,
    );
    report
}
