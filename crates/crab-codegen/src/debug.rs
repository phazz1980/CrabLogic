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
    DebugTable { entries, texts }
}
