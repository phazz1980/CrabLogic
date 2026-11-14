//! Built-in blocks without hardware, computed by Rust code. Every value is `f64`: bool is `0`/`1`,
//! int is a whole number; strings travel beside them (see `text`). Port and parameter indexes follow `block.toml` order.

mod bench;
mod counters;
mod hardware;
mod more;
mod netsim;
mod num;
mod text;
mod wasm;
mod web;

pub use bench::{Bench, Signal, NO_BENCH};
pub use hardware::ds18b20_channel;
pub use more::{next_random, RANDOM_SEED};
pub use netsim::OWN_SLAVE;
pub use num::{Types, V};
pub use wasm::{get as wasm_block, Instruction, WTy, WasmEmit, WasmFn};
pub use web::device_page;

/// What a block sees during one step.
pub struct Ctx<'a> {
    /// Simulated time since the start, already advanced by `dt_ms`.
    pub now_ms: u64,
    /// Values of the inputs in `block.toml` order.
    pub inputs: &'a [f64],
    /// Which inputs exist for the current parameters (`when` conditions).
    pub active: &'a [bool],
    pub params: &'a [f64],
    /// Texts of the inputs; empty for ports that are not strings.
    pub texts: &'a [String],
    /// Texts of the parameters; empty for those that are not strings.
    pub param_texts: &'a [String],
    /// The world outside the board: input pin levels, sensor signals.
    pub bench: &'a Bench,
}

impl Ctx<'_> {
    pub fn flag(&self, i: usize) -> bool {
        self.inputs.get(i).is_some_and(|v| *v != 0.0)
    }

    pub fn input(&self, i: usize) -> f64 {
        self.inputs.get(i).copied().unwrap_or(0.0)
    }

    pub fn param(&self, i: usize) -> f64 {
        self.params.get(i).copied().unwrap_or(0.0)
    }

    pub fn text(&self, i: usize) -> &str {
        self.texts.get(i).map_or("", String::as_str)
    }

    pub fn param_text(&self, i: usize) -> &str {
        self.param_texts.get(i).map_or("", String::as_str)
    }

    pub fn live_flags(&self) -> impl Iterator<Item = bool> + '_ {
        self.inputs
            .iter()
            .zip(self.active)
            .filter(|(_, a)| **a)
            .map(|(v, _)| *v != 0.0)
    }
}

/// Behaviour of one block instance; keeps its own state between steps.
pub trait Behavior {
    /// `out` holds the previous outputs, so a block may keep a value by not writing it.
    fn step(&mut self, ctx: &Ctx, out: &mut [f64]);

    /// Blocks with string ports: `out` is the same slice `step` got, `out_text` the
    /// texts of the outputs (only string ports are written).
    fn step_text(&mut self, _ctx: &Ctx, _out: &mut [f64], _out_text: &mut [String]) {}

    /// Real port types and parameter kinds of the block, given once after creation.
    fn set_types(&mut self, _: &Types) {}

    /// What an output block puts on its pin after the last step: `0`/`1` or a duty in percent.
    fn drive(&self) -> Option<f64> {
        None
    }
}

pub fn b(v: bool) -> f64 {
    f64::from(u8::from(v))
}

/// A block without state.
struct Plain(fn(&Ctx, &mut [f64]));

impl Behavior for Plain {
    fn step(&mut self, ctx: &Ctx, out: &mut [f64]) {
        (self.0)(ctx, out);
    }
}

/// `switch` and `mux`: pick one of the inputs, numbers and texts alike.
struct Select {
    mux: bool,
}

impl Select {
    /// Index of the chosen input.
    fn chosen(&self, c: &Ctx) -> usize {
        if self.mux {
            // Inputs after `sel` that exist for the current port count.
            let count = c.active.iter().skip(1).filter(|a| **a).count().max(1);
            1 + (c.input(0).max(0.0) as usize).min(count - 1)
        } else if c.flag(0) {
            2
        } else {
            1
        }
    }
}

impl Behavior for Select {
    fn step(&mut self, ctx: &Ctx, out: &mut [f64]) {
        out[0] = ctx.input(self.chosen(ctx));
    }

    fn step_text(&mut self, ctx: &Ctx, _: &mut [f64], text: &mut [String]) {
        if let Some(t) = text.first_mut() {
            *t = ctx.text(self.chosen(ctx)).to_owned();
        }
    }
}

/// A stateless block that computes in the numeric model and so needs the real types.
struct Typed {
    f: fn(&Ctx, &Types, &mut [f64]),
    types: Types,
}

impl Behavior for Typed {
    fn step(&mut self, ctx: &Ctx, out: &mut [f64]) {
        (self.f)(ctx, &self.types, out);
    }

    fn set_types(&mut self, t: &Types) {
        self.types = t.clone();
    }
}

pub fn behavior(kind: &str) -> Option<Box<dyn Behavior>> {
    let typed = |f: fn(&Ctx, &Types, &mut [f64])| -> Option<Box<dyn Behavior>> {
        Some(Box::new(Typed {
            f,
            types: Types::default(),
        }))
    };
    let plain = |f: fn(&Ctx, &mut [f64])| -> Option<Box<dyn Behavior>> { Some(Box::new(Plain(f))) };
    match kind {
        "and" => plain(|c, o| o[0] = b(c.live_flags().all(|v| v))),
        "or" => plain(|c, o| o[0] = b(c.live_flags().any(|v| v))),
        "xor" => plain(|c, o| o[0] = b(c.live_flags().filter(|v| *v).count() % 2 == 1)),
        "not" => plain(|c, o| o[0] = b(!c.flag(0))),
        "rs_trigger" => plain(|c, o| {
            // Reset wins.
            if c.flag(1) {
                o[0] = 0.0;
            } else if c.flag(0) {
                o[0] = 1.0;
            }
        }),
        "sr_trigger" => plain(|c, o| {
            // Set wins.
            if c.flag(0) {
                o[0] = 1.0;
            } else if c.flag(1) {
                o[0] = 0.0;
            }
        }),
        "t_trigger" => Some(Box::<TTrigger>::default()),
        "rising_edge" => Some(Box::new(Edge {
            rising: true,
            prev: false,
        })),
        "falling_edge" => Some(Box::new(Edge {
            rising: false,
            prev: false,
        })),
        "timer_on" => Some(Box::<TimerOn>::default()),
        "timer_off" => Some(Box::<TimerOff>::default()),
        "pulse" => Some(Box::<Pulse>::default()),
        "generator" => Some(Box::<Generator>::default()),
        "debounce" => Some(Box::<Debounce>::default()),
        "counter" => Some(Box::<Counter>::default()),
        "hysteresis" => typed(|c, t, o| {
            let x = t.input(0, c.input(0)).f();
            let (on, off) = (t.param(0, c.param(0)).f(), t.param(1, c.param(1)).f());
            if on >= off {
                if x >= on {
                    o[0] = 1.0;
                } else if x <= off {
                    o[0] = 0.0;
                }
            } else if x <= on {
                o[0] = 1.0;
            } else if x >= off {
                o[0] = 0.0;
            }
        }),
        "compare" => typed(|c, t, o| {
            let (x, y) = (t.input(0, c.input(0)), t.input(1, c.input(1)));
            o[0] = b(num::compare(x, y, c.param(0) as i64));
        }),
        "greater" => typed(|c, t, o| {
            o[0] = b(num::compare(
                t.input(0, c.input(0)),
                t.param(0, c.param(0)),
                4,
            ));
        }),
        "less" => typed(|c, t, o| {
            o[0] = b(num::compare(
                t.input(0, c.input(0)),
                t.param(0, c.param(0)),
                2,
            ));
        }),
        "add" => {
            typed(|c, t, o| o[0] = num::add(t.input(0, c.input(0)), t.input(1, c.input(1))).out())
        }
        "sub" => {
            typed(|c, t, o| o[0] = num::sub(t.input(0, c.input(0)), t.input(1, c.input(1))).out())
        }
        "mul" => {
            typed(|c, t, o| o[0] = num::mul(t.input(0, c.input(0)), t.input(1, c.input(1))).out())
        }
        "div" => {
            typed(|c, t, o| o[0] = num::div(t.input(0, c.input(0)), t.input(1, c.input(1))).out())
        }
        "mod" => {
            typed(|c, t, o| o[0] = num::rem(t.input(0, c.input(0)), t.input(1, c.input(1))).out())
        }
        "min" => {
            typed(|c, t, o| o[0] = num::min(t.input(0, c.input(0)), t.input(1, c.input(1))).out())
        }
        "max" => {
            typed(|c, t, o| o[0] = num::max(t.input(0, c.input(0)), t.input(1, c.input(1))).out())
        }
        "limit" => typed(|c, t, o| {
            let (lo, hi) = (t.param(0, c.param(0)), t.param(1, c.param(1)));
            o[0] = num::limit(t.input(0, c.input(0)), lo, hi).out();
        }),
        "scale" => typed(|c, t, o| {
            let p = |i| t.param(i, c.param(i)).f();
            o[0] = f64::from(num::scale(
                t.input(0, c.input(0)).f(),
                p(0),
                p(1),
                p(2),
                p(3),
                c.param(4) != 0.0,
            ));
        }),
        "constant" => plain(|c, o| o[0] = c.param(0)),
        "switch" => Some(Box::new(Select { mux: false })),
        "mux" => Some(Box::new(Select { mux: true })),
        _ => more::behavior(kind)
            .or_else(|| text::behavior(kind))
            .or_else(|| hardware::behavior(kind)),
    }
}

#[derive(Default)]
struct TTrigger {
    prev: bool,
}

impl Behavior for TTrigger {
    fn step(&mut self, ctx: &Ctx, out: &mut [f64]) {
        if ctx.flag(1) {
            out[0] = 0.0;
        } else if ctx.flag(0) && !self.prev {
            out[0] = b(out[0] == 0.0);
        }
        self.prev = ctx.flag(0);
    }
}

struct Edge {
    rising: bool,
    prev: bool,
}

impl Behavior for Edge {
    fn step(&mut self, ctx: &Ctx, out: &mut [f64]) {
        let now = ctx.flag(0);
        out[0] = b(if self.rising {
            now && !self.prev
        } else {
            !now && self.prev
        });
        self.prev = now;
    }
}

#[derive(Default)]
struct TimerOn {
    running: bool,
    start: u32,
}

impl Behavior for TimerOn {
    fn step(&mut self, ctx: &Ctx, out: &mut [f64]) {
        if ctx.flag(0) {
            if !self.running {
                self.running = true;
                self.start = ctx.now_ms as u32;
            }
            out[0] = b(num::elapsed(ctx.now_ms, self.start) >= num::ms(ctx.param(0)));
        } else {
            self.running = false;
            out[0] = 0.0;
        }
    }
}

#[derive(Default)]
struct TimerOff {
    on: bool,
    off_at: u32,
}

impl Behavior for TimerOff {
    fn step(&mut self, ctx: &Ctx, out: &mut [f64]) {
        if ctx.flag(0) {
            self.on = true;
            out[0] = 1.0;
        } else {
            if self.on {
                self.on = false;
                self.off_at = ctx.now_ms as u32;
            }
            out[0] =
                b(out[0] != 0.0 && num::elapsed(ctx.now_ms, self.off_at) < num::ms(ctx.param(0)));
        }
    }
}

#[derive(Default)]
struct Pulse {
    prev: bool,
    active: bool,
    start: u32,
}

impl Behavior for Pulse {
    fn step(&mut self, ctx: &Ctx, out: &mut [f64]) {
        let now = ctx.flag(0);
        if now && !self.prev && !self.active {
            self.active = true;
            self.start = ctx.now_ms as u32;
        }
        self.prev = now;
        if self.active && num::elapsed(ctx.now_ms, self.start) >= num::ms(ctx.param(0)) {
            self.active = false;
        }
        out[0] = b(self.active);
    }
}

/// Deadline form (`specs/generator-deadline`): the same steps in WASM and in the simulation.
#[derive(Default)]
struct Generator {
    start: u32,
    /// Zeroed state is "running since time 0" (as the old formula measured from 0).
    off_phase: bool,
    idle: bool,
}

impl Behavior for Generator {
    fn step(&mut self, ctx: &Ctx, out: &mut [f64]) {
        if !ctx.flag(0) {
            self.idle = true;
            out[0] = 0.0;
            return;
        }
        let now = ctx.now_ms as u32;
        let on = num::ms(ctx.param(0));
        let off = num::ms(ctx.param(1));
        if self.idle {
            self.idle = false;
            self.off_phase = false;
            self.start = now;
        }
        let len = if self.off_phase { off } else { on };
        let d = num::elapsed(ctx.now_ms, self.start);
        if d >= len {
            if d.wrapping_sub(len) < on.wrapping_add(off).wrapping_sub(len) {
                // one phase passed
                self.off_phase = !self.off_phase;
                self.start = self.start.wrapping_add(len);
            } else {
                // the scan skipped several phases: land where the time would be
                let period = on.wrapping_add(off).max(1);
                let r = d % period;
                if r < len {
                    self.start = now.wrapping_sub(r);
                } else {
                    self.off_phase = !self.off_phase;
                    self.start = now.wrapping_sub(r - len);
                }
            }
        }
        out[0] = b(!self.off_phase && on != 0);
    }
}

#[derive(Default)]
struct Debounce {
    last: bool,
    changed: u32,
}

impl Behavior for Debounce {
    fn step(&mut self, ctx: &Ctx, out: &mut [f64]) {
        let now = ctx.flag(0);
        if now != self.last {
            self.last = now;
            self.changed = ctx.now_ms as u32;
        }
        if num::elapsed(ctx.now_ms, self.changed) >= num::ms(ctx.param(0)) {
            out[0] = b(self.last);
        }
    }
}

#[derive(Default)]
struct Counter {
    up: bool,
    down: bool,
}

impl Behavior for Counter {
    fn step(&mut self, ctx: &Ctx, out: &mut [f64]) {
        if ctx.flag(2) {
            out[0] = 0.0;
        } else {
            let mut n = out[0] as i32;
            if ctx.flag(0) && !self.up {
                n = n.wrapping_add(1);
            }
            if ctx.flag(1) && !self.down {
                n = n.wrapping_sub(1);
            }
            out[0] = f64::from(n);
        }
        self.up = ctx.flag(0);
        self.down = ctx.flag(1);
    }
}
