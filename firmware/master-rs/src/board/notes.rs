//! Messages of the master's own services (PCF8574, Modbus, the web page) for the journal: queued
//! from any task, written by the main loop with their source as the tag.

use std::collections::VecDeque;
use std::sync::Mutex;

const MAX_NOTES: usize = 16;

static NOTES: Mutex<VecDeque<(&'static str, String)>> = Mutex::new(VecDeque::new());

pub fn push(source: &'static str, text: String) {
    let mut q = NOTES.lock().unwrap_or_else(|e| e.into_inner());
    if q.len() == MAX_NOTES {
        q.pop_front();
    }
    q.push_back((source, text));
}

pub fn take() -> Vec<(&'static str, String)> {
    NOTES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain(..)
        .collect()
}
