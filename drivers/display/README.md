# display: driver for OLED and character LCD blocks

One crate, two blocks: `library/oled/block.wasm` (`--features oled`) and `library/lcd/block.wasm`
(`--features lcd`). Build both with `drivers\build.ps1 display`. Config format:
`crates/crab-ui-format`; input/output index contract: `specs/display/plan.md`.

## Font

The font is original work drawn for this project (`src/font.rs`, glyphs written as ASCII art and
parsed at compile time). It is dedicated to the public domain together with the project code, so
no third-party licence applies. Glyphs: 5x7 in a 5x8 cell (row 7 for descenders): ASCII 0x20-0x7E,
Cyrillic А-я, Ёё, the degree sign, ‹ › ▲ ▼ (menu) and 8 icons (thermometer, drop, fan, lamp, alarm, wifi, check,
cross; codes `0xE000 + id`). The 8-pixel font advances 6 px (21 characters on 128 px); the 16-pixel
font draws the same glyphs doubled (12 px advance). The 8-row glyph is also the HD44780 CGRAM
format, so the LCD reuses it for custom characters.

## Behaviour notes

- Modules may not contain tables or `call_indirect` (crab-codegen check). This forbids `dyn`,
  function pointers, `core::fmt` and any panic path with a formatted message, so all indexing goes
  through `src/safe.rs` (never `[i]` or `[a..b]` in driver code) and `f64::clamp` is replaced by
  `text::clamp`. The crate type is `cdylib` only: with `rlib` as well, the linker adds a table.
- Parameter numbers follow block.toml: model (0, ignored: the kind comes from the config),
  address 1, sda 2, scl 3. The bus is opened at 400 kHz.
- Value widget: `format` 0 = number with `decimals` then `unit`; `format` 1 = `unit` is a template
  with `%v`.
- Menu: OK opens it; up/down move (wrapping); OK on a submenu enters, on a number/choice starts a
  local edit (up/down change, OK confirms, Back cancels), on a bool toggles at once, on an action
  fires. `item` output: table index of the highlighted item, -1 when closed.
- OLED menu is drawn into the pixel framebuffer (`src/menugfx.rs`), the LCD menu into the
  character grid. 128x64: title (8 px) with a line at y = 8, items from y = 10 in the 16 px font
  (3 items, pitch 15, the last one ends at y = 55 with its descender row) or the 8 px font (pitch 10, 4 items
  with the hint bar, 5 without), a 4x4 / 5x5 marker for the selected item, values right-aligned,
  `›` for submenus, a scrollbar when the items do not fit, a 1 px frame around the value being
  edited, and (menu param `hints`) four inverted cells at y = 56..63: back/up/down/enter, while
  editing cancel/+/-/apply. 128x32: always 8 px items, two on the screen, no hint bar. The picture
  is redrawn only when `Menu::signature` changes.
