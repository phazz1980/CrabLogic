//! Online debugging support: the signal table of a program (what the editor reads from the board).

use crab_core::vars::storage_name;
use crab_core::{PortRef, SignalType};
use crab_debug::{DebugTable, Entry, Source, TextEntry};

use crate::Program;

/// Block outputs in signal order, then variables in declaration order.
/// Strings go to a table of their own: values travel as 4 bytes, strings in `TEXT` frames.
pub fn debug_table(program: &Program) -> DebugTable {
    let mut entries: Vec<Option<Entry>> = vec![None; program.signals.len()];
    let mut texts: Vec<Option<TextEntry>> = vec![None; program.signals.len()];
    for inst in &program.blocks {
        for (&port, &s) in &inst.outputs {
            let signal = &program.signals[s];
            if signal.ty == SignalType::String {
                texts[s] = Some(TextEntry {
                    source: Source::Output(PortRef::new(&inst.block.id, port)),
                    storage: signal.name.clone(),
                });
                continue;
            }
            entries[s] = Some(Entry {
                source: Source::Output(PortRef::new(&inst.block.id, port)),
                ty: signal.ty,
                storage: signal.name.clone(),
                cpp_type: "",
            });
        }
    }
    let mut entries: Vec<Entry> = entries.into_iter().flatten().collect();
    let mut texts: Vec<TextEntry> = texts.into_iter().flatten().collect();
    let vars = program.project.logic.vars.iter();
    texts.extend(
        vars.clone()
            .filter(|v| v.ty.signal() == SignalType::String)
            .map(|v| TextEntry {
                source: Source::Var(v.name.clone()),
                storage: storage_name(&v.name),
            }),
    );
    entries.extend(
        vars.filter(|v| v.ty.signal() != SignalType::String)
            .map(|v| Entry {
                source: Source::Var(v.name.clone()),
                ty: v.ty.signal(),
                storage: storage_name(&v.name),
                cpp_type: v.ty.cpp(),
            }),
    );
    // Elements of the arrays some block works on (as many as a module reports, see `ARRAY_WATCH`).
    let logic = &program.project.logic;
    let used = |name: &str| {
        program.blocks.iter().any(|b| {
            crab_core::array_blocks::array_name(b.block) == Some(name)
                || matches!(b.block.params.get(crab_core::array_blocks::DEST_PARAM), Some(crab_core::ParamValue::Text(d)) if d == name)
        })
    };
    for array in logic
        .arrays
        .iter()
        .filter(|a| a.ty != crab_core::arrays::ElemType::String && used(&a.name))
    {
        for index in 0..array.len.min(crab_debug::ARRAY_WATCH) {
            entries.push(Entry {
                source: Source::Array {
                    name: array.name.clone(),
                    index,
                },
                ty: array.ty.signal(),
                storage: format!("{}_{index}", storage_name(&array.name)),
                cpp_type: "",
            });
        }
    }
    DebugTable { entries, texts }
}
