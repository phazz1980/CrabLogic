//! Messages of the master's own services (PCF8574, Modbus, the web page) for the journal: queued
//! from any task, written by the main loop with their source as the tag.

use super::note_queue::{dropped_text, NoteQueue};

static NOTES: NoteQueue<(&'static str, String)> = NoteQueue::new();

/// From any task: a task at the scan priority never waits for the queue (see `NoteQueue`).
pub fn push(source: &'static str, text: String) {
    NOTES.push_any((source, text));
}

/// For the scan, between steps: never waits.
pub fn take() -> Vec<(&'static str, String)> {
    let (mut lines, dropped) = NOTES.take_try();
    if dropped > 0 {
        lines.push(("journal", dropped_text(dropped)));
    }
    lines
}
