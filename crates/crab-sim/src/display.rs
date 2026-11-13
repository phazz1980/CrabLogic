//! Display chips on the stub I²C bus (`specs/display`): the driver of an `oled` / `lcd` block
//! writes the same bytes it would send to the chip, and these models keep the picture so the
//! editor can show it. Supported: SSD1306 (128x64, 128x32), SH1106 (128x64, RAM 132 wide) and
//! HD44780 behind a PCF8574 backpack (4-bit protocol, P0..P7 = RS, RW, E, BL, D4..D7).
//! Unknown commands are ignored, as the chips mostly do.

/// Width of the SH1106 RAM; the visible 128 columns start at [`SH1106_OFFSET`].
const RAM_COLS: usize = 132;
const SH1106_OFFSET: usize = 2;
const PAGES: usize = 8;

/// Chip family of a display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayKind {
    Ssd1306,
    Sh1106,
    Lcd,
}

/// What the editor shows of a display.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayView {
    /// Id of the display block.
    pub id: String,
    pub kind: DisplayKind,
    /// OLED: pixels; LCD: character cells.
    pub width: usize,
    pub height: usize,
    /// The panel is switched on (OLED display on, LCD display on).
    pub on: bool,
    /// OLED: row-major pixels, `true` is lit (inversion and remap already applied; empty for LCD).
    pub pixels: Vec<bool>,
    /// OLED contrast register (0..=255); 0 for LCD.
    pub contrast: u8,
    /// LCD: the text of every row (codes 0..7 and unprintable ones show as a space).
    pub rows: Vec<String>,
    /// LCD: character codes of every row.
    pub codes: Vec<Vec<u8>>,
    /// LCD: the 8 custom characters, 5x8 (bit 4 is the leftmost column).
    pub cgram: [[u8; 8]; 8],
    /// LCD: the backlight bit of the backpack.
    pub backlight: bool,
}

/// A display chip at an I²C address.
#[derive(Debug, Clone)]
pub struct DisplayDevice {
    pub id: String,
    pub address: u8,
    model: Model,
}

#[derive(Debug, Clone)]
enum Model {
    Oled(Box<Oled>),
    Lcd(Box<Lcd>),
}

impl DisplayDevice {
    /// A display for a block of type `oled` (`model` = `ssd1306_128x64`, `ssd1306_128x32`,
    /// `sh1106_128x64`) or `lcd` (`1602`, `2004`); `None` for an unknown block type.
    pub fn for_block(block_type: &str, model: &str, id: &str, address: i64) -> Option<Self> {
        let address = u8::try_from(address & 0x7F).ok()?;
        let model = match block_type {
            "oled" => {
                let sh1106 = model.starts_with("sh1106");
                let height = if model.ends_with("x32") { 32 } else { 64 };
                Model::Oled(Box::new(Oled::new(sh1106, height)))
            }
            "lcd" => Model::Lcd(Box::new(Lcd::new(if model == "2004" {
                (20, 4)
            } else {
                (16, 2)
            }))),
            _ => return None,
        };
        Some(Self {
            id: id.to_owned(),
            address,
            model,
        })
    }

    pub fn ssd1306(id: &str, address: u8, height: usize) -> Self {
        Self {
            id: id.to_owned(),
            address,
            model: Model::Oled(Box::new(Oled::new(false, height))),
        }
    }

    pub fn sh1106(id: &str, address: u8) -> Self {
        Self {
            id: id.to_owned(),
            address,
            model: Model::Oled(Box::new(Oled::new(true, 64))),
        }
    }

    pub fn lcd(id: &str, address: u8, cols: usize, rows: usize) -> Self {
        Self {
            id: id.to_owned(),
            address,
            model: Model::Lcd(Box::new(Lcd::new((cols, rows)))),
        }
    }

    /// One I²C write transaction (the bytes after the address byte).
    pub fn write(&mut self, bytes: &[u8]) {
        match &mut self.model {
            Model::Oled(o) => o.write(bytes),
            Model::Lcd(l) => l.write(bytes),
        }
    }

    /// Power-on state: the picture is cleared.
    pub fn reset(&mut self) {
        match &mut self.model {
            Model::Oled(o) => **o = Oled::new(o.sh1106, o.height),
            Model::Lcd(l) => **l = Lcd::new((l.cols, l.rows)),
        }
    }

    pub fn view(&self) -> DisplayView {
        match &self.model {
            Model::Oled(o) => o.view(&self.id),
            Model::Lcd(l) => l.view(&self.id),
        }
    }
}

// ---------------------------------------------------------------- OLED

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Addressing {
    Horizontal,
    Vertical,
    Page,
}

#[derive(Debug, Clone)]
struct Oled {
    sh1106: bool,
    height: usize,
    ram: [[u8; RAM_COLS]; PAGES],
    on: bool,
    inverted: bool,
    all_on: bool,
    contrast: u8,
    seg_remap: bool,
    com_reverse: bool,
    mode: Addressing,
    col: usize,
    page: usize,
    col_range: (usize, usize),
    page_range: (usize, usize),
}

impl Oled {
    fn new(sh1106: bool, height: usize) -> Self {
        let cols = if sh1106 { RAM_COLS } else { 128 };
        Self {
            sh1106,
            height,
            ram: [[0; RAM_COLS]; PAGES],
            on: false,
            inverted: false,
            all_on: false,
            contrast: 0x7F,
            // Common modules are upright with segment remap + COM reverse (A1/C8, what Adafruit and
            // u8g2 send); A0/C0 turns the picture by 180 degrees. Raw writes before any init are
            // shown upright, so the model starts in the upright state.
            seg_remap: true,
            com_reverse: true,
            mode: Addressing::Page,
            col: 0,
            page: 0,
            col_range: (0, cols - 1),
            page_range: (0, PAGES - 1),
        }
    }

    fn cols(&self) -> usize {
        if self.sh1106 {
            RAM_COLS
        } else {
            128
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        // Control byte: bit 7 Co (one more control byte follows after one byte), bit 6 D/C
        // (data). Without Co the rest of the transaction is of that kind.
        let mut items: Vec<(bool, u8)> = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let control = bytes[i];
            let data = control & 0x40 != 0;
            i += 1;
            if control & 0x80 != 0 {
                if let Some(&b) = bytes.get(i) {
                    items.push((data, b));
                }
                i += 1;
            } else {
                items.extend(bytes[i..].iter().map(|&b| (data, b)));
                break;
            }
        }
        // The arguments of a command are the next bytes, whatever control byte carried them.
        let mut rest = items.into_iter().peekable();
        while let Some((data, b)) = rest.next() {
            self.byte(
                data,
                b,
                &mut std::iter::from_fn(|| rest.next().map(|x| x.1)),
            );
        }
    }

    /// One data byte, or one command; `rest` gives the argument bytes of a command.
    fn byte(&mut self, data: bool, b: u8, rest: &mut impl Iterator<Item = u8>) {
        if data {
            self.put(b);
            return;
        }
        let mut arg = || rest.next().unwrap_or(0);
        match b {
            0x00..=0x0F => self.col = (self.col & 0xF0) | usize::from(b & 0x0F),
            0x10..=0x1F => self.col = (self.col & 0x0F) | (usize::from(b & 0x0F) << 4),
            0x20 if !self.sh1106 => {
                self.mode = match arg() & 3 {
                    0 => Addressing::Horizontal,
                    1 => Addressing::Vertical,
                    _ => Addressing::Page,
                };
            }
            0x21 if !self.sh1106 => {
                let (a, z) = (usize::from(arg() & 0x7F), usize::from(arg() & 0x7F));
                self.col_range = (a, z);
                self.col = a;
            }
            0x22 if !self.sh1106 => {
                let (a, z) = (usize::from(arg() & 7), usize::from(arg() & 7));
                self.page_range = (a, z);
                self.page = a;
            }
            0x81 => self.contrast = arg(),
            0xA0 => self.seg_remap = false,
            0xA1 => self.seg_remap = true,
            0xA4 => self.all_on = false,
            0xA5 => self.all_on = true,
            0xA6 => self.inverted = false,
            0xA7 => self.inverted = true,
            0xAE => self.on = false,
            0xAF => self.on = true,
            0xB0..=0xB7 => self.page = usize::from(b & 7),
            0xC0 => self.com_reverse = false,
            0xC8 => self.com_reverse = true,
            // Commands with one argument byte that do not change the picture.
            0xA8 | 0xD3 | 0xD5 | 0xD9 | 0xDA | 0xDB | 0x8D | 0xAD => {
                arg();
            }
            _ => {}
        }
    }

    fn put(&mut self, b: u8) {
        let cols = self.cols();
        if self.col < cols && self.page < PAGES {
            self.ram[self.page][self.col] = b;
        }
        match self.mode {
            Addressing::Page => self.col = (self.col + 1) % cols,
            Addressing::Horizontal => {
                if self.col >= self.col_range.1 {
                    self.col = self.col_range.0;
                    self.page = if self.page >= self.page_range.1 {
                        self.page_range.0
                    } else {
                        self.page + 1
                    };
                } else {
                    self.col += 1;
                }
            }
            Addressing::Vertical => {
                if self.page >= self.page_range.1 {
                    self.page = self.page_range.0;
                    self.col = if self.col >= self.col_range.1 {
                        self.col_range.0
                    } else {
                        self.col + 1
                    };
                } else {
                    self.page += 1;
                }
            }
        }
    }

    fn view(&self, id: &str) -> DisplayView {
        let (w, h) = (128, self.height);
        let offset = if self.sh1106 { SH1106_OFFSET } else { 0 };
        let mut pixels = vec![false; w * h];
        for y in 0..h {
            let src_y = if self.com_reverse { y } else { h - 1 - y };
            for x in 0..w {
                let src_x = if self.seg_remap { x } else { w - 1 - x };
                let byte = self.ram[src_y / 8][src_x + offset];
                let lit = byte >> (src_y % 8) & 1 != 0;
                pixels[y * w + x] = self.all_on || lit != self.inverted;
            }
        }
        DisplayView {
            id: id.to_owned(),
            kind: if self.sh1106 {
                DisplayKind::Sh1106
            } else {
                DisplayKind::Ssd1306
            },
            width: w,
            height: h,
            on: self.on,
            pixels,
            contrast: self.contrast,
            rows: Vec::new(),
            codes: Vec::new(),
            cgram: [[0; 8]; 8],
            backlight: false,
        }
    }
}

// ---------------------------------------------------------------- HD44780

const RS: u8 = 0x01;
const RW: u8 = 0x02;
const EN: u8 = 0x04;
const BL: u8 = 0x08;

#[derive(Debug, Clone)]
struct Lcd {
    cols: usize,
    rows: usize,
    ddram: [u8; 128],
    cgram: [u8; 64],
    /// The address counter points into CGRAM (after a 0x40 command) instead of DDRAM.
    in_cgram: bool,
    addr: usize,
    increment: bool,
    on: bool,
    four_bit: bool,
    /// The high nibble of a byte waits for its low half.
    high: Option<u8>,
    last: u8,
    backlight: bool,
}

impl Lcd {
    fn new((cols, rows): (usize, usize)) -> Self {
        Self {
            cols,
            rows,
            ddram: [b' '; 128],
            cgram: [0; 64],
            in_cgram: false,
            addr: 0,
            increment: true,
            on: false,
            four_bit: false,
            high: None,
            last: 0,
            backlight: false,
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.backlight = b & BL != 0;
            // The chip takes the nibble when E falls; reads (RW high) are not modelled.
            if self.last & EN != 0 && b & EN == 0 && self.last & RW == 0 {
                self.nibble(self.last & RS != 0, self.last >> 4);
            }
            self.last = b;
        }
    }

    fn nibble(&mut self, rs: bool, n: u8) {
        if !self.four_bit {
            // Still in the 8-bit mode of power-on: one nibble is the high half of a function
            // set; 0x2 selects the 4-bit interface.
            if !rs && n == 0x2 {
                self.four_bit = true;
                self.high = None;
            }
            return;
        }
        match self.high.take() {
            None => self.high = Some(n),
            Some(h) => self.byte(rs, h << 4 | n),
        }
    }

    fn byte(&mut self, data: bool, b: u8) {
        if data {
            self.put(b);
            return;
        }
        match b {
            0x01 => {
                self.ddram = [b' '; 128];
                self.in_cgram = false;
                self.addr = 0;
                self.increment = true;
            }
            0x02..=0x03 => {
                self.in_cgram = false;
                self.addr = 0;
            }
            0x04..=0x07 => self.increment = b & 2 != 0,
            0x08..=0x0F => self.on = b & 4 != 0,
            0x40..=0x7F => {
                self.in_cgram = true;
                self.addr = usize::from(b & 0x3F);
            }
            0x80..=0xFF => {
                self.in_cgram = false;
                self.addr = usize::from(b & 0x7F);
            }
            _ => {}
        }
    }

    fn put(&mut self, b: u8) {
        if self.in_cgram {
            self.cgram[self.addr & 0x3F] = b & 0x1F;
            self.addr = self.step(self.addr, 0x40);
        } else {
            self.ddram[self.addr & 0x7F] = b;
            self.addr = self.step(self.addr, 0x80);
        }
    }

    fn step(&self, addr: usize, size: usize) -> usize {
        if self.increment {
            (addr + 1) % size
        } else {
            (addr + size - 1) % size
        }
    }

    /// DDRAM address of the first cell of a row.
    fn row_start(row: usize) -> usize {
        [0x00, 0x40, 0x14, 0x54][row & 3]
    }

    fn view(&self, id: &str) -> DisplayView {
        let codes: Vec<Vec<u8>> = (0..self.rows)
            .map(|r| {
                (0..self.cols)
                    .map(|c| self.ddram[(Self::row_start(r) + c) & 0x7F])
                    .collect()
            })
            .collect();
        let rows = codes
            .iter()
            .map(|row| {
                row.iter()
                    .map(|&b| {
                        if (0x20..0x7F).contains(&b) {
                            b as char
                        } else {
                            ' '
                        }
                    })
                    .collect()
            })
            .collect();
        let mut cgram = [[0u8; 8]; 8];
        for (i, line) in self.cgram.iter().enumerate() {
            cgram[i / 8][i % 8] = *line;
        }
        DisplayView {
            id: id.to_owned(),
            kind: DisplayKind::Lcd,
            width: self.cols,
            height: self.rows,
            on: self.on,
            pixels: Vec::new(),
            contrast: 0,
            rows,
            codes,
            cgram,
            backlight: self.backlight,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(v: &DisplayView, x: usize, y: usize) -> bool {
        v.pixels[y * v.width + x]
    }

    #[test]
    fn ssd1306_page_write_lights_pixels() {
        let mut d = DisplayDevice::ssd1306("o", 0x3C, 64);
        // Init: display off, charge pump, contrast, display on.
        d.write(&[0x00, 0xAE, 0x8D, 0x14, 0x81, 0xCF, 0xAF]);
        // Page 2, column 5, then data: 0x01 (top pixel) and 0x80 (bottom of the page).
        d.write(&[0x00, 0xB2, 0x05, 0x10]);
        d.write(&[0x40, 0x01, 0x80]);
        let v = d.view();
        assert!(v.on);
        assert_eq!(v.contrast, 0xCF);
        assert!(lit(&v, 5, 16));
        assert!(lit(&v, 6, 23));
        assert!(!lit(&v, 5, 17));
        assert_eq!(v.pixels.iter().filter(|p| **p).count(), 2);
    }

    #[test]
    fn ssd1306_horizontal_addressing_wraps_to_the_next_page() {
        let mut d = DisplayDevice::ssd1306("o", 0x3C, 32);
        d.write(&[0x00, 0x20, 0x00, 0x21, 0x00, 0x7F, 0x22, 0x00, 0x03]);
        let mut frame = vec![0x40];
        frame.extend((0..128 * 4).map(|i| if i == 128 { 0x01 } else { 0 }));
        d.write(&frame);
        let v = d.view();
        assert_eq!(v.height, 32);
        // Byte 128 is column 0 of page 1.
        assert!(lit(&v, 0, 8));
        assert_eq!(v.pixels.iter().filter(|p| **p).count(), 1);
    }

    #[test]
    fn ssd1306_single_command_control_bytes_and_invert() {
        let mut d = DisplayDevice::ssd1306("o", 0x3C, 64);
        d.write(&[0x80, 0xB1, 0x80, 0xA7, 0x80, 0xFF]); // Co=1 pairs; unknown command is ignored
        d.write(&[0xC0, 0x01]); // single data byte
        let v = d.view();
        assert!(!lit(&v, 0, 8)); // inverted: the written pixel is dark
        assert!(lit(&v, 1, 8));
    }

    #[test]
    fn ssd1306_default_remap_commands_leave_the_picture_upright() {
        let mut d = DisplayDevice::ssd1306("o", 0x3C, 64);
        d.write(&[0x00, 0xA1, 0xC8]);
        d.write(&[0x40, 0x01]);
        let v = d.view();
        assert!(lit(&v, 0, 0));
        assert_eq!(v.pixels.iter().filter(|p| **p).count(), 1);
    }

    #[test]
    fn ssd1306_without_remap_the_picture_is_turned_around() {
        let mut d = DisplayDevice::ssd1306("o", 0x3C, 64);
        d.write(&[0x00, 0xA0, 0xC0]);
        d.write(&[0x40, 0x01]); // page 0, column 0, bit 0
        let v = d.view();
        assert!(lit(&v, 127, 63));
        assert_eq!(v.pixels.iter().filter(|p| **p).count(), 1);
    }

    #[test]
    fn sh1106_skips_two_ram_columns() {
        let mut d = DisplayDevice::sh1106("o", 0x3C);
        // Column 2 is the first visible one: low nibble 2, high nibble 0.
        d.write(&[0x00, 0xB0, 0x02, 0x10]);
        d.write(&[0x40, 0x01, 0x01]);
        // Column 0 and 1 are outside the visible area.
        d.write(&[0x00, 0xB1, 0x00, 0x10]);
        d.write(&[0x40, 0xFF, 0xFF, 0x01]);
        let v = d.view();
        assert_eq!(v.kind, DisplayKind::Sh1106);
        assert!(lit(&v, 0, 0));
        assert!(lit(&v, 1, 0));
        assert!(!lit(&v, 2, 0));
        // Page 1: the third byte went to RAM column 2, i.e. x = 0.
        assert!(lit(&v, 0, 8));
        assert!(!lit(&v, 0, 9));
        assert_eq!(v.pixels.iter().filter(|p| **p).count(), 3);
    }

    /// The nibble pairs a PCF8574 backpack gets: each nibble goes out twice (E high, E low).
    fn pcf(rs: bool, byte: u8, backlight: bool) -> Vec<u8> {
        let base = u8::from(rs) | if backlight { BL } else { 0 };
        let mut out = Vec::new();
        for n in [byte >> 4, byte & 0x0F] {
            out.push(n << 4 | base | EN);
            out.push(n << 4 | base);
        }
        out
    }

    fn init_4bit(d: &mut DisplayDevice) {
        for n in [0x3u8, 0x3, 0x3, 0x2] {
            d.write(&[n << 4 | EN | BL, n << 4 | BL]);
        }
        for cmd in [0x28, 0x0C, 0x01, 0x06] {
            d.write(&pcf(false, cmd, true));
        }
    }

    #[test]
    fn hd44780_writes_text_and_custom_char() {
        let mut d = DisplayDevice::lcd("l", 0x27, 16, 2);
        init_4bit(&mut d);
        for b in *b"Hi" {
            d.write(&pcf(true, b, true));
        }
        // Row 2 starts at DDRAM 0x40; put custom char 0 there.
        d.write(&pcf(false, 0x80 | 0x40, true));
        d.write(&pcf(true, 0, true));
        // CGRAM char 0: a heart-like pattern.
        d.write(&pcf(false, 0x40, true));
        let glyph = [0x00, 0x0A, 0x1F, 0x1F, 0x0E, 0x04, 0x00, 0x00];
        for g in glyph {
            d.write(&pcf(true, g, true));
        }
        let v = d.view();
        assert!(v.on);
        assert!(v.backlight);
        assert_eq!(v.rows[0], "Hi              ");
        assert_eq!(v.codes[1][0], 0);
        assert_eq!(v.cgram[0], glyph);
        assert_eq!(v.rows.len(), 2);

        // Backlight off is the BL bit of the last byte; clear empties the grid.
        d.write(&pcf(false, 0x01, false));
        let v = d.view();
        assert!(!v.backlight);
        assert_eq!(v.rows[0].trim(), "");
    }

    #[test]
    fn hd44780_2004_rows_interleave() {
        let mut d = DisplayDevice::lcd("l", 0x27, 20, 4);
        init_4bit(&mut d);
        d.write(&pcf(false, 0x80 | 0x14, true));
        d.write(&pcf(true, b'C', true));
        d.write(&pcf(false, 0x80 | 0x54, true));
        d.write(&pcf(true, b'D', true));
        let v = d.view();
        assert!(v.rows[2].starts_with('C'));
        assert!(v.rows[3].starts_with('D'));
    }

    #[test]
    fn display_for_block_picks_the_model() {
        let d = DisplayDevice::for_block("oled", "ssd1306_128x32", "a", 0x3C).unwrap();
        assert_eq!((d.view().width, d.view().height), (128, 32));
        let d = DisplayDevice::for_block("oled", "sh1106_128x64", "a", 0x3C).unwrap();
        assert_eq!(d.view().kind, DisplayKind::Sh1106);
        let d = DisplayDevice::for_block("lcd", "2004", "a", 0x27).unwrap();
        assert_eq!((d.view().width, d.view().height), (20, 4));
        assert!(DisplayDevice::for_block("and", "", "a", 1).is_none());
    }
}
