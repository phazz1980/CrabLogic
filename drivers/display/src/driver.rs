//! The driver proper: ties the config, the inputs/outputs of the sheet, the menu and the two
//! display types together. Host-independent: all hardware access goes through [`Io`].
//!
//! Inputs (contract in specs/display/plan.md): own ports `0..P`, widget values `P + k`, then
//! `up/down/ok/back`, then the current values of the menu variables. Outputs: own `ok` first,
//! then `open`, `item`, `changed`, `changed_slot` and the new value of variable `v` at `Q + 4 + v`.

use crate::fb::{Fb, PAGE_BYTES};
use crate::lcd::{self, Lcd};
use crate::menu::Menu;
use crate::oled;
use crate::safe::{copy, Safe};
use crate::text::{round, Grid};
use crab_ui_format::{ConfigView, DisplayKind, ScreenView};

/// Widgets per screen that take part in change detection.
const MAX_SIG: usize = 64;
/// After a failed transfer the display is initialised again after this long.
const RETRY_MS: i32 = 1000;

/// Hardware and sheet access.
pub trait Io {
    fn input(&self, i: usize) -> f64;
    fn output(&mut self, i: usize, v: f64);
    /// I²C write to the display; true when the display accepted all bytes.
    fn write(&mut self, data: &[u8]) -> bool;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    None,
    Screen,
    Menu,
}

pub struct Driver<'a> {
    cfg: Option<ConfigView<'a>>,
    kind: DisplayKind,
    /// Own input and output ports of the block (P and Q of the contract).
    own_in: usize,
    own_out: usize,
    /// First input index of up/down/ok/back and of the menu variables.
    key_in: usize,
    var_in: usize,
    menu: Menu,
    pub fb: Fb,
    pub lcd: Lcd,
    /// OLED: what the display holds.
    sent: [u8; PAGE_BYTES * 8],
    page_ok: u8,
    next_page: usize,
    /// Change detection for the redraw of the OLED.
    sig: [u32; MAX_SIG],
    sig_screen: usize,
    mode: Mode,
    /// Fingerprint of the menu picture in the framebuffer.
    menu_sig: u32,
    /// Link state: initialised, waiting, LCD init phase.
    ready: bool,
    wait_ms: i32,
    phase: u8,
    pub ok: bool,
    /// Character grid of the LCD frame or the menu.
    grid: Grid,
}

impl<'a> Driver<'a> {
    pub const fn new() -> Driver<'a> {
        Driver {
            cfg: None,
            kind: DisplayKind::Ssd1306_128x64,
            own_in: 0,
            own_out: 0,
            key_in: 0,
            var_in: 0,
            menu: Menu::new(),
            fb: Fb::new(),
            lcd: Lcd::new(),
            sent: [0; PAGE_BYTES * 8],
            page_ok: 0,
            next_page: 0,
            sig: [0; MAX_SIG],
            sig_screen: 0,
            mode: Mode::None,
            menu_sig: 0,
            ready: false,
            wait_ms: 0,
            phase: 0,
            ok: false,
            grid: Grid::new(),
        }
    }

    /// Parses the config; false when it is absent or malformed (the driver then stays idle).
    /// `own_in`/`own_out`: number of own input and output ports of the block.
    pub fn init(&mut self, blob: &'a [u8], own_in: usize, own_out: usize) -> bool {
        self.cfg = ConfigView::parse(blob).ok();
        let Some(cfg) = self.cfg else { return false };
        self.kind = cfg.display;
        self.own_in = own_in;
        self.own_out = own_out;
        let w_total = cfg
            .screens()
            .map(|s| s.widgets().filter(|w| w.input.is_some()).count())
            .sum::<usize>();
        self.key_in = own_in + w_total;
        self.var_in = self.key_in + 4;
        if self.is_lcd() {
            let (c, r) = lcd::size(cfg.display);
            self.lcd.cols = c;
            self.lcd.rows = r;
            self.grid.reset(c, r);
        } else {
            self.fb.pages = oled::pages(cfg.display);
            self.grid.reset(21, self.fb.pages);
        }
        true
    }

    /// LCD or OLED. A module built with exactly one feature knows it at compile time, so the
    /// code of the other display type is dropped from the binary.
    fn is_lcd(&self) -> bool {
        if cfg!(all(feature = "lcd", not(feature = "oled"))) {
            true
        } else if cfg!(all(feature = "oled", not(feature = "lcd"))) {
            false
        } else {
            matches!(self.kind, DisplayKind::Lcd1602 | DisplayKind::Lcd2004)
        }
    }

    /// Screen shown now (0-based), clamped to the existing ones.
    fn screen_index(&self, io: &impl Io, count: usize) -> usize {
        let v = round(io.input(0));
        if v < 1.0 || count == 0 {
            0
        } else {
            ((v as usize) - 1).min(count - 1)
        }
    }

    fn widget_value(own_in: usize, io: &impl Io, input: Option<u16>) -> f64 {
        input.map_or(0.0, |k| io.input(own_in + k as usize))
    }

    /// One scheduler step (`dt_ms` since the previous one).
    pub fn step(&mut self, io: &mut impl Io, dt_ms: i32) {
        let Some(cfg) = self.cfg else {
            io.output(0, 0.0);
            return;
        };
        let mut showing_menu = false;
        if let Some(m) = cfg.menu {
            let key = |k: usize| io.input(self.key_in + k) > 0.5;
            let keys = [key(0), key(1), key(2), key(3)];
            let var_in = self.var_in;
            let out = {
                let io_ref = &*io;
                self.menu
                    .step(&m, dt_ms, keys, &|v| io_ref.input(var_in + v as usize))
            };
            let q = self.own_out;
            io.output(q, f64::from(u8::from(out.open)));
            io.output(q + 1, f64::from(out.item));
            io.output(q + 2, f64::from(u8::from(out.changed)));
            io.output(q + 3, f64::from(out.slot));
            if out.slot >= 0 {
                io.output(q + 4 + out.slot as usize, out.value);
            }
            showing_menu = out.open;
        }
        if self.is_lcd() {
            self.step_lcd(io, &cfg, dt_ms, showing_menu);
        } else {
            self.step_oled(io, &cfg, dt_ms, showing_menu);
        }
        io.output(0, f64::from(u8::from(self.ok)));
    }

    fn menu_grid(&mut self, io: &impl Io, cfg: &ConfigView) {
        if let Some(m) = cfg.menu {
            let var_in = self.var_in;
            self.menu
                .draw(&m, &mut self.grid, &|v| io.input(var_in + v as usize));
        }
    }

    // ---------------------------------------------------------------- OLED

    fn step_oled(&mut self, io: &mut impl Io, cfg: &ConfigView, dt_ms: i32, showing_menu: bool) {
        if !self.link_wait(dt_ms) {
            return;
        }
        if !self.ready {
            let mut buf = [0u8; 32];
            let n = oled::init_commands(self.kind, &mut buf);
            if !io.write(buf.part(n)) {
                self.fail();
                return;
            }
            self.ready = true;
            self.ok = true;
            self.page_ok = 0;
            return; // the first frame goes out on the next step
        }
        if showing_menu {
            if let Some(m) = cfg.menu {
                let var_in = self.var_in;
                let var = |v: u16| io.input(var_in + v as usize);
                let sig = self.menu.signature(&m, &var);
                if self.mode != Mode::Menu || sig != self.menu_sig {
                    self.fb.clear();
                    self.menu.draw_fb(&m, &mut self.fb, &var);
                    self.menu_sig = sig;
                    self.mode = Mode::Menu;
                }
            }
        } else {
            self.render_screen(io, cfg);
        }
        self.send_page(io);
    }

    /// Redraws the selected screen when its inputs changed.
    fn render_screen(&mut self, io: &impl Io, cfg: &ConfigView) {
        let idx = self.screen_index(io, cfg.screen_count());
        let Some(screen) = cfg.screens().nth(idx) else {
            if self.mode != Mode::Screen {
                self.fb.clear();
                self.mode = Mode::Screen;
            }
            return;
        };
        let own_in = self.own_in;
        let mut sig = [0u32; MAX_SIG];
        for (k, w) in screen.widgets().take(MAX_SIG).enumerate() {
            sig.wr(
                k,
                (Self::widget_value(own_in, io, w.input) as f32).to_bits(),
            );
        }
        if self.mode == Mode::Screen && idx == self.sig_screen && sig == self.sig {
            return;
        }
        self.sig = sig;
        self.sig_screen = idx;
        self.mode = Mode::Screen;
        self.fb.clear();
        for w in screen.widgets() {
            oled::draw_widget(&mut self.fb, &w, Self::widget_value(own_in, io, w.input));
        }
    }

    /// Sends at most one changed page.
    fn send_page(&mut self, io: &mut impl Io) {
        let pages = self.fb.pages;
        for k in 0..pages {
            let p = (self.next_page + k) % pages;
            let (lo, hi) = (p * PAGE_BYTES, (p + 1) * PAGE_BYTES);
            if self.page_ok & (1 << p) != 0 && self.sent.span(lo, hi) == self.fb.buf.span(lo, hi) {
                continue;
            }
            let mut buf = [0u8; 7 + PAGE_BYTES];
            let n = oled::page_transfer(self.kind, p, self.fb.page(p), &mut buf);
            if io.write(buf.part(n)) {
                if let Some(dst) = self.sent.get_mut(lo..hi) {
                    copy(dst, self.fb.buf.span(lo, hi));
                }
                self.page_ok |= 1 << p;
                self.next_page = (p + 1) % pages;
                self.ok = true;
            } else {
                self.fail();
            }
            return;
        }
    }

    // ----------------------------------------------------------------- LCD

    fn step_lcd(&mut self, io: &mut impl Io, cfg: &ConfigView, dt_ms: i32, showing_menu: bool) {
        if !self.link_wait(dt_ms) {
            return;
        }
        let bl = io.input(1) > 0.5;
        if !self.ready {
            self.lcd_init(io, bl); // one transfer per step, the frame starts on the next one
            return;
        }
        // Build the frame for this step.
        if showing_menu {
            self.menu_grid(io, cfg);
        } else {
            let (c, r) = (self.lcd.cols, self.lcd.rows);
            self.grid.reset(c, r);
            let idx = self.screen_index(io, cfg.screen_count());
            if let Some(screen) = cfg.screens().nth(idx) {
                self.draw_screen_grid(io, &screen);
            }
        }
        self.lcd.set_backlight(bl);
        let grid = self.grid;
        self.lcd.map_grid(&grid);
        let mut buf = [0u8; lcd::MAX_TRANSFER];
        if let Some((act, n)) = self.lcd.next(&mut buf) {
            if io.write(buf.part(n)) {
                self.lcd.commit(act);
                self.ok = true;
            } else {
                self.fail();
            }
        }
    }

    fn draw_screen_grid(&mut self, io: &impl Io, screen: &ScreenView) {
        let own_in = self.own_in;
        for w in screen.widgets() {
            lcd::draw_widget(&mut self.grid, &w, Self::widget_value(own_in, io, w.input));
        }
    }

    /// The HD44780 power-up sequence in 4-bit mode, one transfer per step with the required waits.
    /// Returns false when a transfer failed.
    fn lcd_init(&mut self, io: &mut impl Io, bl: bool) -> bool {
        let mut nib = [0u8; 2];
        let mut cmds = [0u8; 16];
        let (data, wait): (&[u8], i32) = match self.phase {
            0 => {
                self.phase = 1;
                self.wait_ms = 50;
                return true;
            }
            1..=3 => {
                lcd::put_nibble(&mut nib, 3, bl);
                (&nib, if self.phase == 1 { 5 } else { 1 })
            }
            4 => {
                lcd::put_nibble(&mut nib, 2, bl);
                (&nib, 1)
            }
            _ => {
                let mut n = 0;
                let blb = if bl { 0x08 } else { 0 };
                // function set (4-bit, 2 lines, 5x8), display on, entry mode, clear
                for c in [0x28u8, 0x0C, 0x06, 0x01] {
                    lcd::put_byte(&mut cmds, &mut n, c, 0, blb);
                }
                (cmds.part(n), 5)
            }
        };
        if !io.write(data) {
            self.fail();
            return false;
        }
        self.wait_ms = wait;
        if self.phase >= 5 {
            self.ready = true;
            self.ok = true;
            self.lcd.invalidate();
            self.lcd.backlight = bl;
        } else {
            self.phase += 1;
        }
        true
    }

    // -------------------------------------------------------------- common

    /// Counts down a wait; true when the step may proceed.
    fn link_wait(&mut self, dt_ms: i32) -> bool {
        if self.wait_ms > 0 {
            self.wait_ms -= dt_ms.max(0);
            if self.wait_ms > 0 {
                return false;
            }
        }
        true
    }

    /// A transfer failed: report, and start over after a pause.
    fn fail(&mut self) {
        self.ok = false;
        self.ready = false;
        self.phase = 0;
        self.wait_ms = RETRY_MS;
        self.mode = Mode::None;
        self.page_ok = 0;
        self.lcd.invalidate();
    }
}

impl Default for Driver<'_> {
    fn default() -> Self {
        Driver::new()
    }
}
