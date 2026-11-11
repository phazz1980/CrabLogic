use super::*;

pub(super) fn show_label(show: ParamShow) -> &'static str {
    match show {
        ParamShow::Named => crate::i18n::tr("имя=значение"),
        ParamShow::Value => crate::i18n::tr("значение"),
        ParamShow::Secret => crate::i18n::tr("пароль (p***1)"),
        ParamShow::Hidden => crate::i18n::tr("не показывать"),
    }
}

pub(super) fn kind_label(kind: ParamKind) -> &'static str {
    match kind {
        ParamKind::Bool => crate::i18n::tr("bool — да/нет"),
        ParamKind::Int => crate::i18n::tr("int — целое"),
        ParamKind::Float => crate::i18n::tr("float — дробное"),
        ParamKind::Text => crate::i18n::tr("string — строка"),
        ParamKind::Duration => crate::i18n::tr("duration — время"),
        ParamKind::Pin => crate::i18n::tr("pin — вывод"),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PortRow {
    pub(super) name: String,
    /// Caption in Russian; empty: the name is shown.
    pub(super) label: String,
    pub(super) ty: SignalType,
    /// Empty: no default (inputs only).
    pub(super) default: String,
    pub(super) when: String,
    pub(super) description: String,
    /// Value in the simulation on the PC (outputs of drivers); empty: none.
    pub(super) sim: String,
}

impl PortRow {
    pub(super) fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            label: String::new(),
            ty: SignalType::Bool,
            default: String::new(),
            when: String::new(),
            description: String::new(),
            sim: String::new(),
        }
    }

    pub(super) fn from_def(p: &PortDef) -> Self {
        Self {
            name: p.name.clone(),
            label: p.label.clone().unwrap_or_default(),
            ty: p.ty,
            default: p.default.as_ref().map(value_text).unwrap_or_default(),
            when: p.when.clone().unwrap_or_default(),
            description: p.description.clone(),
            sim: p.sim.as_ref().map(value_text).unwrap_or_default(),
        }
    }

    pub(super) fn to_def(&self) -> Result<PortDef, String> {
        Ok(PortDef {
            name: self.name.trim().to_owned(),
            label: optional(&self.label),
            ty: self.ty,
            default: parse_signal(self.ty, &self.default)
                .map_err(|e| format!("«{}»: {e}", self.name))?,
            when: optional(&self.when),
            description: self.description.trim().to_owned(),
            sim: parse_signal(self.ty, &self.sim)
                .map_err(|e| format!("«{}», значение в симуляции: {e}", self.name))?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ParamRow {
    pub(super) name: String,
    pub(super) label: String,
    pub(super) kind: ParamKind,
    pub(super) default: String,
    pub(super) min: String,
    pub(super) max: String,
    pub(super) when: String,
    pub(super) description: String,
    pub(super) show: ParamShow,
    /// `local` and `[[param.choice]]` from `block.toml`, kept as is (no column for them).
    pub(super) local: Option<String>,
    pub(super) choices: Vec<Choice>,
    /// For pins: direction and sharing, edited next to the type.
    pub(super) mode: PinMode,
    pub(super) shared: bool,
    /// The value is needed while generating code: no input can replace it.
    pub(super) static_only: bool,
    /// Bus of a pin, kept as it was (set in `block.toml`).
    pub(super) bus: String,
}

impl ParamRow {
    pub(super) fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            label: String::new(),
            kind: ParamKind::Int,
            default: String::new(),
            min: String::new(),
            max: String::new(),
            when: String::new(),
            description: String::new(),
            show: ParamShow::Named,
            local: None,
            choices: Vec::new(),
            mode: PinMode::Io,
            shared: false,
            static_only: false,
            bus: String::new(),
        }
    }

    pub(super) fn from_def(p: &ParamDef) -> Self {
        let num = |v: Option<f64>| v.map(|v| format!("{v}")).unwrap_or_default();
        Self {
            name: p.name.clone(),
            label: p.label.clone().unwrap_or_default(),
            kind: p.kind,
            default: p.default.as_ref().map(value_text).unwrap_or_default(),
            min: num(p.min),
            max: num(p.max),
            when: p.when.clone().unwrap_or_default(),
            description: p.description.clone(),
            show: p.show,
            local: p.local.clone(),
            choices: p.choices.clone(),
            mode: p.mode,
            shared: p.shared,
            static_only: p.static_only,
            bus: p.bus.clone(),
        }
    }

    pub(super) fn to_def(&self) -> Result<ParamDef, String> {
        let err = |e: String| format!("«{}»: {e}", self.name);
        Ok(ParamDef {
            name: self.name.trim().to_owned(),
            label: optional(&self.label),
            kind: self.kind,
            default: parse_param(self.kind, &self.default).map_err(err)?,
            min: parse_number(&self.min).map_err(err)?,
            max: parse_number(&self.max).map_err(err)?,
            when: optional(&self.when),
            description: self.description.trim().to_owned(),
            show: self.show,
            local: self.local.clone(),
            mode: if self.kind == ParamKind::Pin {
                self.mode
            } else {
                PinMode::Io
            },
            shared: self.kind == ParamKind::Pin && self.shared,
            bus: if self.kind == ParamKind::Pin {
                self.bus.clone()
            } else {
                String::new()
            },
            fixed: false,
            runtime: false,
            static_only: self.static_only,
            choices: self.choices.clone(),
        })
    }
}

/// Everything the window edits; compared with the saved copy to detect changes.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Form {
    /// Type of the block: made from the name for a new block, fixed afterwards
    /// (schemas refer to it).
    pub(super) kind: String,
    pub(super) name: String,
    pub(super) symbol: String,
    /// Section and folders in the user library (`Раздел/Папка`), empty in the root.
    pub(super) group: String,
    pub(super) description: String,
    pub(super) memory: bool,
    pub(super) single: bool,
    /// The block type may stand on an expander pin; kept from the copied block.
    pub(super) expander: bool,
    /// `[block.uart]`: kept as it is, the form has no field for it yet.
    pub(super) uart: Option<crab_core::UartUse>,
    /// Comma-separated.
    pub(super) includes: String,
    pub(super) libraries: String,
    pub(super) inputs: Vec<PortRow>,
    pub(super) outputs: Vec<PortRow>,
    pub(super) params: Vec<ParamRow>,
    pub(super) template: String,
    /// `native.cpp`: C++ built into the master firmware; empty when the block has none.
    pub(super) native: String,
    /// Wizard fields when `native` was made by the wizard and not edited by hand.
    pub(super) wizard: Option<LibraryWizard>,
    /// The wizard as last applied to the rows of the form.
    pub(super) synced: Option<LibraryWizard>,
    pub(super) docs: String,
    /// Arduino libraries and the example attached to the block.
    pub(super) attach: Attach,
}

impl Form {
    pub(super) fn new_block(kind: String, group: String) -> Self {
        Self {
            kind,
            name: crate::i18n::tr("Новый блок").into(),
            symbol: String::new(),
            group,
            description: String::new(),
            memory: false,
            single: false,
            expander: false,
            uart: None,
            includes: String::new(),
            libraries: String::new(),
            inputs: vec![PortRow::new("in")],
            outputs: vec![PortRow::new("out")],
            params: Vec::new(),
            template: NEW_TEMPLATE.into(),
            native: String::new(),
            wizard: None,
            synced: None,
            docs: String::new(),
            attach: Attach::default(),
        }
    }

    pub(super) fn from_block(block: &LibraryBlock) -> Self {
        let def = &block.def;
        let info = &def.info;
        let template = block
            .template(ESP32_CPP_TEMPLATE)
            .and_then(Result::ok)
            // Blocks written in Rust show their generated C++ as a starting point.
            .or_else(|| crab_codegen::builtin_cpp_preview(def))
            .unwrap_or_default();
        let native = block
            .native_path()
            .and_then(|p| fs::read_to_string(p).ok())
            .unwrap_or_default();
        let docs = block
            .docs_path()
            .and_then(|p| fs::read_to_string(p).ok())
            .unwrap_or_default();
        Self {
            kind: info.kind.clone(),
            name: info.name.clone(),
            symbol: info.symbol.clone().unwrap_or_default(),
            group: info.group.clone(),
            description: info.description.clone(),
            memory: info.memory,
            single: info.single,
            expander: info.expander,
            uart: info.uart.clone(),
            includes: info.includes.join(", "),
            libraries: info.libraries.join(", "),
            inputs: def.inputs.iter().map(PortRow::from_def).collect(),
            outputs: def.outputs.iter().map(PortRow::from_def).collect(),
            params: def.params.iter().map(ParamRow::from_def).collect(),
            template,
            wizard: LibraryWizard::parse(&native),
            synced: LibraryWizard::parse(&native),
            native,
            docs,
            attach: Attach::from_dir(&block.dir),
        }
    }

    /// Group (`input`, `output`, `param`) and trimmed name of every row, in table order.
    pub(super) fn row_names(&self) -> Vec<(&'static str, String)> {
        let rows = |group, names: Vec<&String>| {
            names
                .into_iter()
                .map(|n| (group, n.trim().to_owned()))
                .collect::<Vec<_>>()
        };
        let mut all = rows("input", self.inputs.iter().map(|p| &p.name).collect());
        all.extend(rows(
            "output",
            self.outputs.iter().map(|p| &p.name).collect(),
        ));
        all.extend(rows("param", self.params.iter().map(|p| &p.name).collect()));
        all
    }

    /// Block definition from the form, checked the same way the library checks it.
    pub(super) fn to_def(&self) -> Result<BlockDef, String> {
        let kind = self.kind.trim().to_owned();
        if self.name.trim().is_empty() {
            return Err(crate::i18n::tr("у блока должно быть название").into());
        }
        if kind == VAR_GET || kind == VAR_SET {
            return Err(crate::i18n::fmt(
                "тип «{0}» занят встроенным блоком переменной",
                &[&kind],
            ));
        }
        let def = BlockDef {
            info: crab_core::library::BlockInfo {
                kind: kind.clone(),
                name: self.name.trim().to_owned(),
                symbol: optional(&self.symbol),
                group: self.group.trim().to_owned(),
                shared: String::new(),
                description: self.description.trim().to_owned(),
                memory: self.memory,
                includes: list(&self.includes),
                libraries: list(&self.libraries),
                single: self.single,
                hidden: false,
                pullable: false,
                uart: self.uart.clone(),
                board: None,
                code_of: None,
                expander: self.expander,
            },
            inputs: self
                .inputs
                .iter()
                .map(PortRow::to_def)
                .collect::<Result<_, _>>()?,
            outputs: self
                .outputs
                .iter()
                .map(PortRow::to_def)
                .collect::<Result<_, _>>()?,
            params: self
                .params
                .iter()
                .map(ParamRow::to_def)
                .collect::<Result<_, _>>()?,
        };
        def.validate(&kind)?;
        Ok(def)
    }
}

pub(super) fn optional(text: &str) -> Option<String> {
    let t = text.trim();
    (!t.is_empty()).then(|| t.to_owned())
}

pub(super) fn list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(super) fn parse_bool(t: &str) -> Option<bool> {
    match t {
        "true" | "1" | "да" => Some(true),
        "false" | "0" | "нет" => Some(false),
        _ => None,
    }
}

pub(super) fn parse_float(t: &str) -> Option<f64> {
    t.replace(',', ".").parse().ok()
}

/// Default of a port; empty means "no default".
pub(super) fn parse_signal(ty: SignalType, text: &str) -> Result<Option<ParamValue>, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let value = match ty {
        SignalType::Bool => parse_bool(t).map(ParamValue::Bool),
        SignalType::Int => t.parse().ok().map(ParamValue::Int),
        SignalType::Float => parse_float(t).map(ParamValue::Float),
        SignalType::String => Some(ParamValue::Text(t.to_owned())),
        SignalType::Any | SignalType::Same => Some(SignalType::guess_value(t)),
        SignalType::Number => {
            Some(SignalType::guess_value(t)).filter(|v| SignalType::of_value(v).is_numeric())
        }
    };
    value
        .map(Some)
        .ok_or_else(|| crate::i18n::fmt("«{0}» не подходит к типу {1}", &[&t, &type_name(ty)]))
}

/// Default of a parameter; empty means "no default" (the parameter is required).
pub(super) fn parse_param(kind: ParamKind, text: &str) -> Result<Option<ParamValue>, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let value = match kind {
        ParamKind::Bool => parse_bool(t).map(ParamValue::Bool),
        ParamKind::Int | ParamKind::Pin => t.parse().ok().map(ParamValue::Int),
        ParamKind::Float => parse_float(t).map(ParamValue::Float),
        // Durations are checked by the definition itself.
        ParamKind::Text | ParamKind::Duration => Some(ParamValue::Text(t.to_owned())),
    };
    value
        .map(Some)
        .ok_or_else(|| crate::i18n::fmt("«{0}» не подходит к виду «{1}»", &[&t, &kind_label(kind)]))
}

pub(super) fn parse_number(text: &str) -> Result<Option<f64>, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    parse_float(t)
        .map(Some)
        .ok_or_else(|| crate::i18n::fmt("граница «{0}» — не число", &[&t]))
}
