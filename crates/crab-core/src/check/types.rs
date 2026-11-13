use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// Coarse, stable class of a [`Diagnostic`]. The id is what the help page
/// "Частые ошибки" and the AI refer to; messages may change, ids must not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagKind {
    BadId,
    DuplicateId,
    UnknownType,
    UnknownParam,
    MissingParam,
    ParamValue,
    Singleton,
    Condition,
    ConstantType,
    InvertType,
    LinkEnds,
    LinkType,
    InputBusy,
    InputEmpty,
    PinBusy,
    PinUnavailable,
    PinWarning,
    VariableName,
    VariableType,
    VariableLink,
    VariableWriters,
    VariableUnused,
    LayoutOrphan,
    Loop,
    Uart,
    BoardBlock,
    Expander,
    ExpanderPin,
    Tag,
    FloatCounter,
    ExternalTag,
    /// The module generator refuses the block: it does not run on the Rust master
    /// (`crab_codegen::rust_master_diagnostics`).
    RustMaster,
    /// A block with C++ code (`esp32.cpp.tmpl`, `native.cpp`): only `block.wasm` runs now.
    CppBlock,
    /// A retained variable is fed straight from an output that changes often.
    RetainFast,
    /// A screen widget has no display or does not fit the display screen.
    DisplayWidget,
    /// Two displays at one I²C address on one bus.
    DisplayAddress,
    /// More different Cyrillic letters on one LCD screen than the character generator holds.
    DisplayCyrillic,
    /// A `menu` block without a display, or with a block that is not a display.
    MenuDisplay,
    /// The variable of a menu item is missing or of a wrong type.
    MenuItemVar,
}

impl DiagKind {
    pub const ALL: &'static [DiagKind] = &[
        DiagKind::BadId,
        DiagKind::DuplicateId,
        DiagKind::UnknownType,
        DiagKind::UnknownParam,
        DiagKind::MissingParam,
        DiagKind::ParamValue,
        DiagKind::Singleton,
        DiagKind::Condition,
        DiagKind::ConstantType,
        DiagKind::InvertType,
        DiagKind::LinkEnds,
        DiagKind::LinkType,
        DiagKind::InputBusy,
        DiagKind::InputEmpty,
        DiagKind::PinBusy,
        DiagKind::PinUnavailable,
        DiagKind::PinWarning,
        DiagKind::VariableName,
        DiagKind::VariableType,
        DiagKind::VariableLink,
        DiagKind::VariableWriters,
        DiagKind::VariableUnused,
        DiagKind::LayoutOrphan,
        DiagKind::Loop,
        DiagKind::Uart,
        DiagKind::BoardBlock,
        DiagKind::Expander,
        DiagKind::ExpanderPin,
        DiagKind::Tag,
        DiagKind::FloatCounter,
        DiagKind::ExternalTag,
        DiagKind::RustMaster,
        DiagKind::CppBlock,
        DiagKind::RetainFast,
        DiagKind::DisplayWidget,
        DiagKind::DisplayAddress,
        DiagKind::DisplayCyrillic,
        DiagKind::MenuDisplay,
        DiagKind::MenuItemVar,
    ];

    /// Stable machine-readable id, kebab-case.
    pub fn id(self) -> &'static str {
        match self {
            DiagKind::BadId => "bad-id",
            DiagKind::DuplicateId => "duplicate-id",
            DiagKind::UnknownType => "unknown-type",
            DiagKind::UnknownParam => "unknown-param",
            DiagKind::MissingParam => "missing-param",
            DiagKind::ParamValue => "param-value",
            DiagKind::Singleton => "singleton",
            DiagKind::Condition => "condition",
            DiagKind::ConstantType => "constant-type",
            DiagKind::InvertType => "invert-type",
            DiagKind::LinkEnds => "link-ends",
            DiagKind::LinkType => "link-type",
            DiagKind::InputBusy => "input-busy",
            DiagKind::InputEmpty => "input-empty",
            DiagKind::PinBusy => "pin-busy",
            DiagKind::PinUnavailable => "pin-unavailable",
            DiagKind::PinWarning => "pin-warning",
            DiagKind::VariableName => "variable-name",
            DiagKind::VariableType => "variable-type",
            DiagKind::VariableLink => "variable-link",
            DiagKind::VariableWriters => "variable-writers",
            DiagKind::VariableUnused => "variable-unused",
            DiagKind::LayoutOrphan => "layout-orphan",
            DiagKind::Loop => "loop",
            DiagKind::Uart => "uart",
            DiagKind::BoardBlock => "board-block",
            DiagKind::Expander => "expander",
            DiagKind::ExpanderPin => "expander-pin",
            DiagKind::Tag => "tag",
            DiagKind::FloatCounter => "float-counter",
            DiagKind::ExternalTag => "external-tag",
            DiagKind::RustMaster => "rust-master",
            DiagKind::CppBlock => "cpp-block",
            DiagKind::RetainFast => "retain-fast",
            DiagKind::DisplayWidget => "display-widget",
            DiagKind::DisplayAddress => "display-address",
            DiagKind::DisplayCyrillic => "display-cyrillic",
            DiagKind::MenuDisplay => "menu-display",
            DiagKind::MenuItemVar => "menu-item-var",
        }
    }
}

/// One problem found in the schema. Messages are user-facing, in Russian.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub kind: DiagKind,
    pub severity: Severity,
    /// Block the problem belongs to, for highlighting in the editor.
    pub block: Option<String>,
    /// Index of the link in [`Logic::links`].
    pub link: Option<usize>,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level = match self.severity {
            Severity::Error => "ошибка",
            Severity::Warning => "предупреждение",
        };
        write!(f, "{level}: {}", self.message)
    }
}

/// Result of [`check`].
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub diagnostics: Vec<Diagnostic>,
    /// Execution order as indices into [`Logic::blocks`]. Complete only without errors.
    pub order: Vec<usize>,
    /// Links that close a feedback loop through a block with memory: their value
    /// comes from the previous scan. Indices into [`Logic::links`].
    pub feedback: Vec<usize>,
}

impl Report {
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }

    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
    }

    pub(super) fn block_error(&mut self, kind: DiagKind, block: &str, message: String) {
        self.push(kind, Severity::Error, Some(block), None, message);
    }

    pub(super) fn link_error(&mut self, kind: DiagKind, link: usize, message: String) {
        self.push(kind, Severity::Error, None, Some(link), message);
    }

    pub(crate) fn push(
        &mut self,
        kind: DiagKind,
        severity: Severity,
        block: Option<&str>,
        link: Option<usize>,
        message: String,
    ) {
        self.diagnostics.push(Diagnostic {
            kind,
            severity,
            block: block.map(str::to_owned),
            link,
            message,
        });
    }
}
