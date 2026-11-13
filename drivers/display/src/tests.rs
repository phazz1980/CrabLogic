//! Host tests: rendering to the framebuffer / LCD cells, the menu, and frame pacing.

use crate::driver::{Driver, Io};
use crate::fb::Fb;
use crate::font::glyph;
use crate::lcd::MAX_TRANSFER;
use crate::menu::{Menu, MenuOut};
use crab_ui_format::Menu as MenuCfg;
use crab_ui_format::*;

#[derive(Default)]
struct Mock {
    inputs: Vec<f64>,
    outputs: Vec<f64>,
    writes: Vec<Vec<u8>>,
    fail: bool,
}

impl Mock {
    fn new(n_in: usize, n_out: usize) -> Mock {
        Mock {
            inputs: vec![0.0; n_in],
            outputs: vec![0.0; n_out],
            ..Default::default()
        }
    }
}

impl Io for Mock {
    fn input(&self, i: usize) -> f64 {
        self.inputs.get(i).copied().unwrap_or(0.0)
    }
    fn output(&mut self, i: usize, v: f64) {
        if i >= self.outputs.len() {
            self.outputs.resize(i + 1, 0.0);
        }
        self.outputs[i] = v;
    }
    fn write(&mut self, data: &[u8]) -> bool {
        if self.fail {
            return false;
        }
        self.writes.push(data.to_vec());
        true
    }
}

fn widget(kind: WidgetKind, x: u8, y: u8, input: Option<u16>) -> Widget {
    Widget {
        kind,
        x,
        y,
        w: 0,
        h: 0,
        font: 8,
        input,
    }
}

fn text(s: &str, x: u8, y: u8) -> Widget {
    widget(WidgetKind::Text { text: s.into() }, x, y, None)
}

fn value(format: u8, decimals: u8, unit: &str) -> Widget {
    widget(
        WidgetKind::Value {
            format,
            decimals,
            unit: unit.into(),
        },
        0,
        0,
        Some(0),
    )
}

fn cfg(display: DisplayKind, widgets: Vec<Widget>, menu: Option<MenuCfg>) -> Vec<u8> {
    let mut c = Config::new(display);
    c.screens.push(Screen { widgets });
    c.menu = menu;
    c.encode()
}

/// Expected ASCII art of glyphs side by side with one blank column between them.
fn art(glyphs: &[[u8; 8]]) -> Vec<String> {
    (0..8)
        .map(|r| {
            glyphs
                .iter()
                .map(|g| {
                    (0..5)
                        .map(|c| if g[r] & (0x10 >> c) != 0 { '#' } else { '.' })
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join(".")
        })
        .collect()
}

fn g(c: char) -> [u8; 8] {
    glyph(c as u16)
}

/// Renders screen 1 of an OLED config to a framebuffer through the driver.
fn render_oled(blob: &[u8], inputs: &[f64]) -> Fb {
    let mut d = Driver::new();
    assert!(d.init(blob, 1, 1));
    let mut io = Mock::new(1 + inputs.len(), 1);
    io.inputs[0] = 1.0;
    io.inputs[1..].copy_from_slice(inputs);
    d.step(&mut io, 10); // init
    d.step(&mut io, 10); // first frame
    let mut fb = Fb::new();
    fb.pages = d.fb.pages;
    fb.buf = d.fb.buf;
    fb
}

#[test]
fn oled_value_widget_art() {
    let fb = render_oled(
        &cfg(DisplayKind::Ssd1306_128x64, vec![value(0, 0, "A")], None),
        &[1.0],
    );
    assert_eq!(fb.ascii(0, 0, 11, 8), art(&[g('1'), g('A')]));
    assert!(fb.ascii(11, 0, 117, 8).iter().all(|r| !r.contains('#')));
    assert!(fb.ascii(0, 8, 128, 56).iter().all(|r| !r.contains('#')));
}

#[test]
fn oled_cyrillic_text_art() {
    let fb = render_oled(
        &cfg(DisplayKind::Ssd1306_128x64, vec![text("Бя", 2, 8)], None),
        &[],
    );
    let got = fb.ascii(2, 8, 11, 8);
    assert_eq!(got[0], "#####......");
    assert_eq!(got, art(&[glyph(0x411), glyph(0x44F)]));
}

#[test]
fn oled_value_template() {
    let fb = render_oled(
        &cfg(DisplayKind::Ssd1306_128x64, vec![value(1, 1, "T=%v")], None),
        &[7.0],
    );
    assert_eq!(
        fb.ascii(0, 0, 29, 8),
        art(&[g('T'), g('='), g('7'), g('.'), g('0')])
    );
}

#[test]
fn oled_bar_and_16px_font() {
    let mut bar = widget(
        WidgetKind::Bar {
            min: 0.0,
            max: 10.0,
        },
        0,
        0,
        Some(0),
    );
    bar.w = 12;
    bar.h = 5;
    let fb = render_oled(&cfg(DisplayKind::Ssd1306_128x64, vec![bar], None), &[5.0]);
    assert_eq!(
        fb.ascii(0, 0, 12, 5),
        [
            "############",
            "######.....#",
            "######.....#",
            "######.....#",
            "############"
        ]
    );
    let mut big = text("A", 0, 0);
    big.font = 16;
    let fb = render_oled(&cfg(DisplayKind::Ssd1306_128x64, vec![big], None), &[]);
    let rows = fb.ascii(0, 0, 10, 16);
    assert_eq!(rows[0], "..######..");
    assert_eq!(rows[1], "..######..");
    assert_eq!(rows[2], "##......##");
    assert_eq!(rows[14], "..........");
}

#[test]
fn oled_clips_outside_screen() {
    let w = text("WWWWWWWWWWWWWWWWWWWWWWWWWW", 100, 28);
    let fb = render_oled(&cfg(DisplayKind::Ssd1306_128x32, vec![w], None), &[]);
    assert_eq!(fb.pages, 4);
}

#[test]
fn lcd_cyrillic_cells_and_cgram() {
    let blob = cfg(
        DisplayKind::Lcd1602,
        vec![text("Привет", 0, 0), text("A°", 0, 1)],
        None,
    );
    let mut d = Driver::new();
    assert!(d.init(&blob, 2, 1));
    let mut io = Mock::new(2, 1);
    io.inputs = vec![1.0, 1.0];
    for _ in 0..30 {
        d.step(&mut io, 60);
    }
    // П, и, в, т need custom glyphs; р and е look like Latin p and e.
    assert_eq!(&d.lcd.cells[0][..6], &[0, b'p', 1, 2, b'e', 3]);
    assert_eq!(&d.lcd.cells[1][..2], &[b'A', 0xDF]);
    assert_eq!(&d.lcd.cg[..4], &[0x41F, 0x438, 0x432, 0x442]);
    assert!(d.lcd.cg[4..].iter().all(|&c| c == 0));
    assert_eq!(io.outputs[0], 1.0);
    // 4 glyph uploads (36 bytes each) and 2 changed rows (4 + 16*4 bytes), nothing resent later.
    assert_eq!(io.writes.iter().filter(|w| w.len() == 36).count(), 4);
    assert_eq!(io.writes.iter().filter(|w| w.len() == 68).count(), 2);
    let n = io.writes.len();
    for _ in 0..5 {
        d.step(&mut io, 60);
    }
    assert_eq!(io.writes.len(), n);
}

#[test]
fn lcd_limits_custom_glyphs_to_eight() {
    let blob = cfg(DisplayKind::Lcd2004, vec![], None);
    let mut d = Driver::new();
    assert!(d.init(&blob, 2, 1));
    let mut grid = crate::text::Grid::new();
    grid.reset(20, 4);
    grid.put_str(0, 0, "БГДЖЗИЛПФЦ");
    assert_eq!(crate::lcd::Lcd::custom_needed(&grid), 10);
    d.lcd.map_grid(&grid);
    assert_eq!(&d.lcd.cells[0][8..10], b"??");
    assert_eq!(&d.lcd.cells[0][..8], &[0, 1, 2, 3, 4, 5, 6, 7]);
}

#[test]
fn lcd_one_transfer_per_step() {
    let blob = cfg(
        DisplayKind::Lcd2004,
        vec![
            text("Привет мир", 0, 0),
            text("Строка 2", 0, 1),
            text("3", 0, 2),
            text("4", 0, 3),
        ],
        None,
    );
    let mut d = Driver::new();
    assert!(d.init(&blob, 2, 1));
    let mut io = Mock::new(2, 1);
    io.inputs = vec![1.0, 1.0];
    for _ in 0..40 {
        let before = io.writes.len();
        d.step(&mut io, 60);
        assert!(io.writes.len() - before <= 1);
        assert!(io.writes.iter().all(|w| w.len() <= MAX_TRANSFER));
    }
}

#[test]
fn oled_frame_pacing_one_page_per_step() {
    let blob = cfg(
        DisplayKind::Sh1106_128x64,
        vec![text("Hello", 0, 24), value(0, 0, "")],
        None,
    );
    let mut d = Driver::new();
    assert!(d.init(&blob, 1, 1));
    let mut io = Mock::new(2, 1);
    io.inputs[0] = 1.0;
    for _ in 0..30 {
        let before = io.writes.len();
        d.step(&mut io, 10);
        assert!(
            io.writes.len() - before <= 1,
            "more than one transfer in a step"
        );
    }
    // init + 8 pages, then quiet.
    assert_eq!(io.writes.len(), 9);
    assert_eq!(io.writes[1][..7], [0x80, 0xB0, 0x80, 2, 0x80, 0x10, 0x40]); // SH1106 offset 2
    assert_eq!(io.writes[1].len(), 135);
    // Changing the value touches only the top page.
    io.inputs[1] = 42.0;
    for _ in 0..10 {
        d.step(&mut io, 10);
    }
    assert_eq!(io.writes.len(), 10);
    assert_eq!(io.writes[9][1], 0xB0);
    // A bus failure is reported and the display is initialised again later.
    io.fail = true;
    io.inputs[1] = 43.0;
    d.step(&mut io, 10);
    assert_eq!(io.outputs[0], 0.0);
    io.fail = false;
    for _ in 0..200 {
        d.step(&mut io, 10);
    }
    assert_eq!(io.outputs[0], 1.0);
    assert_eq!(io.writes.len(), 10 + 9);
}

// ---------------------------------------------------------------- menu

fn item(parent: Option<u16>, kind: MenuItemKind, caption: &str, var: Option<u16>) -> MenuItem {
    MenuItem {
        parent,
        kind,
        caption: caption.into(),
        var_slot: var,
    }
}

fn demo_menu(timeout_s: u16) -> Vec<u8> {
    let number = MenuItemKind::Number {
        min: 0.0,
        max: 30.0,
        step: 0.5,
        decimals: 1,
        unit: "°C".into(),
    };
    let choice = MenuItemKind::Choice {
        choices: vec!["Авто".into(), "Ручн".into()],
    };
    let menu = MenuCfg {
        timeout_s,
        items: vec![
            item(None, MenuItemKind::Submenu, "Уставки", None),
            item(Some(0), number, "Темп", Some(0)),
            item(Some(0), choice, "Режим", Some(1)),
            item(None, MenuItemKind::Bool, "Свет", Some(2)),
            item(None, MenuItemKind::Action, "Сброс", Some(3)),
            item(None, MenuItemKind::View, "Т воды", Some(4)),
        ],
    };
    cfg(DisplayKind::Ssd1306_128x64, vec![], Some(menu))
}

struct Rig {
    menu: Menu,
    vars: [f64; 8],
}

impl Rig {
    fn new() -> Rig {
        Rig {
            menu: Menu::new(),
            vars: [0.0; 8],
        }
    }
    fn step(&mut self, blob: &[u8], dt: i32, keys: [bool; 4]) -> MenuOut {
        let view = ConfigView::parse(blob).unwrap().menu.unwrap();
        let vars = self.vars;
        self.menu.step(&view, dt, keys, &|v| vars[v as usize])
    }
    /// A key press: one step with the key, one without. Returns the output of the first.
    fn press(&mut self, blob: &[u8], key: usize) -> MenuOut {
        let mut k = [false; 4];
        k[key] = true;
        let out = self.step(blob, 10, k);
        self.step(blob, 10, [false; 4]);
        out
    }
}

const UP: usize = 0;
const DOWN: usize = 1;
const OK: usize = 2;
const BACK: usize = 3;

#[test]
fn menu_edit_number_confirms_once() {
    let blob = demo_menu(30);
    let mut r = Rig::new();
    r.vars[0] = 20.0;
    assert!(!r.step(&blob, 10, [false; 4]).open);
    let o = r.press(&blob, OK); // open
    assert!(o.open && o.item == 0 && !o.changed);
    let o = r.press(&blob, OK); // enter "Уставки"
    assert_eq!(o.item, 1);
    r.press(&blob, OK); // start editing "Темп"
    for _ in 0..3 {
        let o = r.press(&blob, UP);
        assert!(
            !o.changed && o.slot == -1,
            "nothing is reported while editing"
        );
    }
    let mut changes = 0;
    let mut seen = None;
    for step in 0..6 {
        let mut k = [false; 4];
        k[OK] = step == 0;
        let o = r.step(&blob, 10, k);
        if o.changed {
            changes += 1;
            seen = Some((o.slot, o.value));
            // The live variable is still 20 (the sheet writes it); the menu shows what was confirmed.
            assert_eq!(r.menu.value_of(&|v| r.vars[v as usize], 0), 21.5);
        }
    }
    assert_eq!(changes, 1);
    assert_eq!(seen, Some((0, 21.5)));
    // Back out: submenu -> root -> closed.
    assert!(r.press(&blob, BACK).open);
    assert!(!r.press(&blob, BACK).open);
}

#[test]
fn menu_back_cancels_edit() {
    let blob = demo_menu(30);
    let mut r = Rig::new();
    r.vars[0] = 10.0;
    r.press(&blob, OK);
    r.press(&blob, OK);
    r.press(&blob, OK);
    r.press(&blob, DOWN);
    let o = r.press(&blob, BACK); // cancel the edit, stay in the submenu
    assert!(o.open && !o.changed);
    r.press(&blob, OK); // edit again: starts from the untouched 10.0
    r.press(&blob, UP);
    let o = r.press(&blob, OK);
    assert!(o.changed && o.value == 10.5 && o.slot == 0);
}

#[test]
fn menu_clamps_choice_bool_action_view() {
    let blob = demo_menu(30);
    let mut r = Rig::new();
    r.vars[0] = 29.8;
    r.press(&blob, OK);
    r.press(&blob, OK);
    r.press(&blob, OK);
    for _ in 0..3 {
        r.press(&blob, UP);
    }
    assert_eq!(r.press(&blob, OK).value, 30.0);
    // Choice: next item, edit, down cycles 0 -> 1, OK.
    r.press(&blob, DOWN);
    r.press(&blob, OK);
    r.press(&blob, DOWN);
    let o = r.press(&blob, OK);
    assert_eq!((o.slot, o.value), (1, 1.0));
    r.press(&blob, BACK); // root, cursor on "Уставки"
    r.press(&blob, DOWN);
    let o = r.press(&blob, OK); // bool toggles at once
    assert_eq!((o.slot, o.value, o.changed), (2, 1.0, true));
    r.press(&blob, DOWN);
    let o = r.press(&blob, OK); // action
    assert_eq!((o.slot, o.changed), (3, true));
    r.press(&blob, DOWN);
    let o = r.press(&blob, OK); // view does nothing
    assert!(!o.changed && o.item == 5);
    // Down from the last item wraps to the first.
    assert_eq!(r.press(&blob, DOWN).item, 0);
}

#[test]
fn menu_timeout_closes_and_zero_never() {
    let blob = demo_menu(5);
    let mut r = Rig::new();
    r.press(&blob, OK);
    assert!(r.step(&blob, 4000, [false; 4]).open);
    assert!(!r.step(&blob, 1500, [false; 4]).open);
    let blob = demo_menu(0);
    let mut r = Rig::new();
    r.press(&blob, OK);
    assert!(r.step(&blob, 3_600_000, [false; 4]).open);
}

fn press_key(d: &mut Driver<'_>, io: &mut Mock, key: usize) -> Vec<f64> {
    io.inputs[1 + key] = 1.0;
    d.step(io, 10);
    let snap = io.outputs.clone();
    io.inputs[1 + key] = 0.0;
    d.step(io, 10);
    snap
}

#[test]
fn menu_drawn_over_screen_with_contract_outputs() {
    let blob = demo_menu(30);
    let mut d = Driver::new();
    assert!(d.init(&blob, 1, 1));
    // P = 1, W = 0: keys at 1..5, variables from 5; outputs: ok, open, item, changed, slot, vars.
    let mut io = Mock::new(10, 12);
    io.inputs[0] = 1.0;
    io.inputs[5] = 20.0;
    let o = press_key(&mut d, &mut io, OK);
    assert_eq!((o[1], o[2]), (1.0, 0.0)); // open, item 0
    press_key(&mut d, &mut io, OK);
    press_key(&mut d, &mut io, OK); // edit
    press_key(&mut d, &mut io, UP);
    let o = press_key(&mut d, &mut io, OK);
    assert_eq!((o[3], o[4], o[5]), (1.0, 0.0, 20.5)); // changed, slot 0, new value at Q + 4
                                                      // The next step the pulse is gone and the slot is -1.
    assert_eq!((io.outputs[3], io.outputs[4]), (0.0, -1.0));
    // The title bar (inverted row) is lit across the top page.
    assert!(d.fb.page(0)[1..120].iter().all(|&b| b != 0));
}
