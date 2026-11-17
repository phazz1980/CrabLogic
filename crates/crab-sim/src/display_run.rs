//! The display driver (`drivers/display`) running on the host: in the simulation it draws the
//! screens and the menu of the `oled` / `lcd` blocks as on the board, and its I²C writes go to
//! the model of the chip (`DisplayDevice`).
//!
//! The driver sees the same input and output cells as in a module (layout in
//! `crab-codegen/src/wasm/display.rs`): its own ports, the numeric widgets, the string widgets,
//! the buttons `up/down/ok/back`, the menu variables; outputs: its own ports, `open`, `item`,
//! `changed`, the slot of a confirmed variable and the new values.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use display::driver::{Driver, Io, Sent};

/// Scan period of the board the driver is paced for, ms.
const DRIVER_STEP_MS: u32 = 10;

/// How the simulation drives one display block: the config blob and the blocks that feed the
/// driver's extra input slots, in slot order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayDrive {
    /// `crab.config` of the display block.
    pub config: Vec<u8>,
    /// Numeric widget blocks that have an input.
    pub widgets: Vec<String>,
    /// String widget blocks, after the numeric ones.
    pub string_widgets: Vec<String>,
    /// The `menu` block and its variables `(name, action)`.
    pub menu: Option<(String, Vec<(String, bool)>)>,
}

/// The config blob lives as long as its driver; one copy per distinct blob keeps a long session
/// of schema edits from piling up memory.
fn leak_blob(bytes: &[u8]) -> &'static [u8] {
    static CACHE: OnceLock<Mutex<HashMap<Vec<u8>, &'static [u8]>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    cache
        .entry(bytes.to_vec())
        .or_insert_with(|| Box::leak(bytes.to_vec().into_boxed_slice()))
}

/// Where one numeric or string widget gets its value.
#[derive(Clone)]
pub(crate) struct Feed {
    /// Node of the widget block (`None`: the block is gone).
    pub node: Option<usize>,
    /// Variable the input is bound to.
    pub var: Option<String>,
}

pub(crate) struct MenuRun {
    pub node: usize,
    /// Input positions of `up`, `down`, `ok`, `back` in the menu block.
    pub keys: [Option<usize>; 4],
    /// Output positions of `open`, `item`, `changed`.
    pub outs: [Option<usize>; 3],
    pub vars: Vec<(String, bool)>,
}

pub(crate) struct DisplayRun {
    /// Id of the display block (and of its chip model).
    pub id: String,
    /// Node of the display block.
    pub node: usize,
    pub own_in: usize,
    pub own_out: usize,
    pub widgets: Vec<Feed>,
    pub strings: Vec<Feed>,
    pub menu: Option<MenuRun>,
    blob: &'static [u8],
    driver: Driver<'static>,
}

/// What came out of a driver run.
#[derive(Default)]
pub(crate) struct Outcome {
    /// I²C writes for the chip model.
    pub writes: Vec<Vec<u8>>,
    /// Own outputs (`ok`): `(index, value)`.
    pub own: Vec<(usize, f64)>,
    /// Menu block outputs: `(output position, value)`.
    pub menu_outs: Vec<(usize, f64)>,
    /// New variable values.
    pub vars: Vec<(String, f64)>,
}

struct SimIo<'a> {
    cells: &'a [f64],
    texts: &'a [String],
    out: Vec<Option<f64>>,
    writes: Vec<Vec<u8>>,
}

impl Io for SimIo<'_> {
    fn input_i(&self, i: usize) -> i32 {
        self.cells.get(i).copied().unwrap_or(0.0) as i32
    }
    fn input_f(&self, i: usize) -> f32 {
        self.cells.get(i).copied().unwrap_or(0.0) as f32
    }
    fn input_text(&self, i: usize, buf: &mut [u8]) -> usize {
        let Some(text) = self.texts.get(i) else {
            return 0;
        };
        let n = text.len().min(buf.len());
        buf[..n].copy_from_slice(&text.as_bytes()[..n]);
        n
    }
    fn output_i(&mut self, i: usize, v: i32) {
        self.put(i, f64::from(v));
    }
    fn output_f(&mut self, i: usize, v: f32) {
        self.put(i, f64::from(v));
    }
    fn send(&mut self, data: &[u8]) -> Sent {
        if !data.is_empty() {
            self.writes.push(data.to_vec());
        }
        Sent::Queued
    }
}

impl SimIo<'_> {
    fn put(&mut self, i: usize, v: f64) {
        if i >= self.out.len() {
            self.out.resize(i + 1, None);
        }
        self.out[i] = Some(v);
    }
}

impl DisplayRun {
    pub fn new(
        id: &str,
        node: usize,
        own: (usize, usize),
        config: &[u8],
        widgets: Vec<Feed>,
        strings: Vec<Feed>,
        menu: Option<MenuRun>,
    ) -> Self {
        let mut run = DisplayRun {
            id: id.to_owned(),
            node,
            own_in: own.0,
            own_out: own.1,
            widgets,
            strings,
            menu,
            blob: leak_blob(config),
            driver: Driver::new(),
        };
        run.restart();
        run
    }

    /// Back to power-on: the driver forgets the screen and the menu.
    pub fn restart(&mut self) {
        self.driver = Driver::new();
        self.driver.init(self.blob, self.own_in, self.own_out);
    }

    /// One scan of `dt_ms` over the gathered input cells: the driver steps at the pace of the
    /// board's cycle, so a whole screen is drawn within one scan of the simulation.
    pub fn run(&mut self, dt_ms: u32, cells: &[f64], texts: &[String]) -> Outcome {
        let mut io = SimIo {
            cells,
            texts,
            out: Vec::new(),
            writes: Vec::new(),
        };
        let mut left = dt_ms.max(1);
        while left > 0 {
            let dt = left.min(DRIVER_STEP_MS);
            self.driver.step(&mut io, dt as i32);
            left -= dt;
        }
        let value = |i: usize| io.out.get(i).copied().flatten();
        let mut outcome = Outcome {
            own: (0..self.own_out)
                .filter_map(|i| Some((i, value(i)?)))
                .collect(),
            ..Outcome::default()
        };
        if let Some(menu) = &self.menu {
            let q = self.own_out;
            for (k, pos) in menu.outs.iter().enumerate() {
                if let (Some(pos), Some(v)) = (pos, value(q + k)) {
                    outcome.menu_outs.push((*pos, v));
                }
            }
            let slot = value(q + 3).map_or(-1, |v| v as i64);
            for (n, (name, action)) in menu.vars.iter().enumerate() {
                let hit = slot == n as i64;
                if *action {
                    // A pulse of one scan, as the module writes it.
                    outcome.vars.push((name.clone(), f64::from(u8::from(hit))));
                } else if hit {
                    if let Some(v) = value(q + 4 + n) {
                        outcome.vars.push((name.clone(), v));
                    }
                }
            }
        }
        outcome.writes = std::mem::take(&mut io.writes);
        outcome
    }
}
