//! What a sheet shows, beyond its drawing: which operations belong to which device symbol or wire,
//! and where the wires join a symbol (`specs/panel-editor`, task 3).
//!
//! The drawing stays a flat list of operations (see [`crate::Op`]); an [`Item`] names the
//! operations of one symbol or one wire by their index, so that the editor can pick, move and
//! re-route them while the PDF is made from the very same operations.

/// Direction in which a wire leaves a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// A place where a wire joins a symbol. `name` is `in:<k>` / `out:<k>` for the pole `k` of a power
/// symbol, `pe` for its earth, `signal`, `plus` and `minus` for a device of the control sheet, and
/// the terminal (`IO26`) for the board.
#[derive(Debug, Clone, PartialEq)]
pub struct Port {
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub dir: Dir,
}

/// The end of a wire: a device (or the bus `EXT` / `PE`) and the port name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct End {
    pub device: String,
    pub port: String,
}

impl End {
    pub fn new(device: &str, port: impl Into<String>) -> Self {
        Self {
            device: device.to_owned(),
            port: port.into(),
        }
    }
}

/// What an item is.
#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    /// The symbol of a device: `view` says which of its drawings (`power`, `control`, `board`),
    /// `origin` is the point the layout placed it at.
    Symbol {
        view: &'static str,
        device: String,
        origin: (f32, f32),
        ports: Vec<Port>,
    },
    /// A wire: the key it is numbered by (`K1:2>M1:L1`) and its two ends.
    Wire { key: String, from: End, to: End },
    /// A text the user can move by hand: the key of its place in the layout.
    Label { key: String },
    /// A supply line of the power sheet number `index` (from 0), the `k`-th conductor: its end
    /// reference can be edited to carry the line on to another sheet.
    Bus { index: usize, k: usize },
}

/// A symbol or a wire and the operations it is drawn with.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub kind: ItemKind,
    /// Indices into the operations of the sheet, in drawing order.
    pub ops: Vec<usize>,
}

impl Item {
    /// The device of a symbol.
    pub fn device(&self) -> Option<&str> {
        match &self.kind {
            ItemKind::Symbol { device, .. } => Some(device),
            ItemKind::Wire { .. } | ItemKind::Label { .. } | ItemKind::Bus { .. } => None,
        }
    }
}
