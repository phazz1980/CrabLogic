use super::*;

/// Checks ids, types and parameters. Returns the definition of each block
/// (`None` for unknown types and duplicates).
pub(super) fn check_blocks<'a>(
    logic: &Logic,
    lib: &'a Library,
    report: &mut Report,
) -> Vec<Option<&'a BlockDef>> {
    let mut seen = HashMap::new();
    let mut singles: HashMap<&str, &str> = HashMap::new();
    let mut defs = Vec::with_capacity(logic.blocks.len());
    for (i, block) in logic.blocks.iter().enumerate() {
        let id = block.id.as_str();
        if !is_valid_ident(id) {
            report.block_error(DiagKind::BadId,
                id,
                format!(
                    "недопустимый id блока «{id}»: только буквы (можно русские), цифры, «_» и «-» (имя может состоять и из одних цифр), без пробелов и точек"
                ),
            );
        }
        if seen.insert(id, i).is_some() {
            report.block_error(
                DiagKind::DuplicateId,
                id,
                format!("id «{id}» встречается дважды"),
            );
            defs.push(None);
            continue;
        }
        let Some(lib_block) = lib.resolve(logic, block) else {
            report.block_error(
                DiagKind::UnknownType,
                id,
                format!("блок «{id}»: неизвестный тип «{}»", block.kind),
            );
            defs.push(None);
            continue;
        };
        let def = &lib_block.def;
        for (name, value) in &block.params {
            match def.param(name) {
                Some(p) => {
                    check_param(id, p, value, report);
                    if block.kind == "wifi" {
                        check_wifi_text(id, &p.name, value, report);
                    }
                }
                None => report.block_error(
                    DiagKind::UnknownParam,
                    id,
                    format!(
                        "блок «{id}»: у типа «{}» нет параметра «{name}»",
                        block.kind
                    ),
                ),
            }
        }
        check_bindings(logic, block, def, report);
        for (port, value) in &block.consts {
            match def.input(port) {
                Some(p) if !p.ty.accepts(value) => report.block_error(
                    DiagKind::ConstantType,
                    id,
                    format!(
                        "блок «{id}»: константа на входе «{port}» не подходит к типу {}",
                        type_name(p.ty)
                    ),
                ),
                Some(_) => {}
                None => report.block_error(
                    DiagKind::ConstantType,
                    id,
                    format!("блок «{id}»: константа на несуществующем входе «{port}»"),
                ),
            }
        }
        for port in &block.inverted {
            match def.output(port) {
                Some(p) if p.ty != SignalType::Bool => report.block_error(
                    DiagKind::InvertType,
                    id,
                    format!(
                        "блок «{id}»: инвертировать можно только логический выход, «{port}» — нет"
                    ),
                ),
                Some(_) => {}
                None => report.block_error(
                    DiagKind::InvertType,
                    id,
                    format!("блок «{id}»: инверсия несуществующего выхода «{port}»"),
                ),
            }
        }
        for port in &block.inverted_inputs {
            match def.input(port) {
                Some(p) if p.ty != SignalType::Bool => report.block_error(
                    DiagKind::InvertType,
                    id,
                    format!(
                        "блок «{id}»: инвертировать можно только логический вход, «{port}» — нет"
                    ),
                ),
                Some(_) => {}
                None => report.block_error(
                    DiagKind::InvertType,
                    id,
                    format!("блок «{id}»: инверсия несуществующего входа «{port}»"),
                ),
            }
        }
        // Conditions that fail to evaluate (e.g. comparing a string with a number).
        let conditions = def.inputs.iter().chain(&def.outputs).map(|p| &p.when);
        for when in conditions
            .chain(def.params.iter().map(|p| &p.when))
            .filter(|_| block.tag.is_none())
        {
            if let Err(e) = def.enabled(when.as_deref(), &block.params) {
                report.block_error(DiagKind::Condition, id, format!("блок «{id}»: {e}"));
            }
        }
        // A copy's parameters are its tag's: they are checked on the tag.
        for p in def.params.iter().filter(|_| block.tag.is_none()) {
            if p.default.is_none()
                && !block.params.contains_key(&p.name)
                && !block.bound.contains_key(&p.name)
                && def
                    .enabled(p.when.as_deref(), &block.params)
                    .unwrap_or(true)
            {
                report.block_error(
                    DiagKind::MissingParam,
                    id,
                    format!("блок «{id}»: не задан параметр «{}»", p.name),
                );
            }
        }
        if def.info.single && block.tag.is_none() {
            if let Some(first) = singles.insert(&block.kind, id) {
                singles.insert(&block.kind, first);
                report.block_error(
                    DiagKind::Singleton,
                    id,
                    format!(
                        "блок «{id}»: «{}» может быть в проекте только один, уже есть «{first}»",
                        def.info.name
                    ),
                );
            }
        }
        defs.push(Some(def));
    }
    defs
}

/// Wi-Fi limits: SSID is 1..=32 bytes; a password is empty (open network), 8..=63 printable
/// ASCII characters, or exactly 64 hex digits (a raw PSK).
pub fn wifi_text_error(name: &str, text: &str) -> Option<String> {
    match name {
        "ssid" | "ap_ssid" => {
            if text.len() > 32 {
                return Some("имя сети — не более 32 байт".to_owned());
            }
            if text.chars().any(|c| c.is_control()) {
                return Some("имя сети содержит управляющие символы".to_owned());
            }
            None
        }
        "password" | "ap_password" => {
            if text.is_empty() {
                return None;
            }
            if let Some(c) = text.chars().find(|c| !(' '..='~').contains(c)) {
                return Some(format!(
                    "в пароле недопустимый символ «{c}» (разрешены латиница, цифры и знаки ASCII)"
                ));
            }
            let hex64 = text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit());
            if !(8..=63).contains(&text.len()) && !hex64 {
                return Some(format!(
                    "длина пароля {} — нужно 8–63 символа (или 64 шестнадцатеричных)",
                    text.len()
                ));
            }
            None
        }
        _ => None,
    }
}

pub(super) fn check_wifi_text(id: &str, name: &str, value: &ParamValue, report: &mut Report) {
    if let ParamValue::Text(s) = value {
        if let Some(e) = wifi_text_error(name, s) {
            report.block_error(
                DiagKind::ParamValue,
                id,
                format!("блок «{id}»: параметр «{name}» — {e}"),
            );
        }
    }
}

pub(crate) fn check_param(id: &str, p: &ParamDef, value: &ParamValue, report: &mut Report) {
    if !p.kind.accepts(value) {
        let expected = match p.kind {
            ParamKind::Bool => "true или false",
            ParamKind::Int => "целое число",
            ParamKind::Float => "число",
            ParamKind::Text => "строка в кавычках",
            ParamKind::Duration => "время вида \"500ms\", \"5s\", \"2m\", \"1h\"",
            ParamKind::Pin => "номер вывода",
        };
        report.block_error(
            DiagKind::ParamValue,
            id,
            format!("блок «{id}»: параметр «{}» — ожидается {expected}", p.name),
        );
        return;
    }
    if !p.allows(value) {
        let list: Vec<String> = p
            .choices
            .iter()
            .map(|c| format!("{} ({})", c.value, c.label))
            .collect();
        report.block_error(
            DiagKind::ParamValue,
            id,
            format!(
                "блок «{id}»: параметр «{}» — допустимо только {}",
                p.name,
                list.join(", ")
            ),
        );
        return;
    }
    let number = match value {
        ParamValue::Int(v) => Some(*v as f64),
        ParamValue::Float(v) => Some(*v),
        ParamValue::Text(s) if p.kind == ParamKind::Duration => {
            parse_duration_ms(s).map(|ms| ms as f64)
        }
        _ => None,
    };
    let Some(number) = number else { return };
    // A pin parameter allowed below zero (`min = -1`) uses it for "not used".
    if p.kind == ParamKind::Pin && number < 0.0 && p.min.is_none_or(|min| min >= 0.0) {
        report.block_error(
            DiagKind::ParamValue,
            id,
            format!(
                "блок «{id}»: номер вывода «{}» не может быть отрицательным",
                p.name
            ),
        );
    }
    if let Some(min) = p.min.filter(|&min| number < min) {
        report.block_error(
            DiagKind::ParamValue,
            id,
            format!(
                "блок «{id}»: параметр «{}» меньше допустимого {min}",
                p.name
            ),
        );
    }
    if let Some(max) = p.max.filter(|&max| number > max) {
        report.block_error(
            DiagKind::ParamValue,
            id,
            format!(
                "блок «{id}»: параметр «{}» больше допустимого {max}",
                p.name
            ),
        );
    }
}

/// Checks the ports and parameters a block takes from variables.
pub(super) fn check_bindings(
    logic: &Logic,
    block: &crate::Block,
    def: &BlockDef,
    report: &mut Report,
) {
    let id = block.id.as_str();
    for name in &block.param_in {
        if def.input(name).is_none() {
            report.block_error(
                DiagKind::UnknownParam,
                id,
                format!("блок «{id}»: параметр «{name}» нельзя сделать входом"),
            );
        }
    }
    for (name, var) in &block.bound {
        let Some(v) = logic.var(var) else {
            report.block_error(
                DiagKind::VariableLink,
                id,
                format!("блок «{id}»: «{name}» связан с несуществующей переменной «{var}»"),
            );
            continue;
        };
        let want = if let Some(p) = def.input(name).or_else(|| def.output(name)) {
            Some(p.ty)
        } else {
            def.param(name)
                .and_then(|p| bindable_type(p.kind, !p.choices.is_empty(), p.runtime))
        };
        match want {
            None => report.block_error(
                DiagKind::VariableLink,
                id,
                format!("блок «{id}»: «{name}» нельзя связать с переменной"),
            ),
            Some(ty)
                if !matches!(ty, SignalType::Any | SignalType::Same)
                    && ty != v.ty.signal()
                    && !(ty == SignalType::Number && v.ty.signal().is_numeric()) =>
            {
                report.block_error(
                    DiagKind::VariableType,
                    id,
                    format!(
                        "блок «{id}»: «{name}» — {}, а переменная «{var}» — {}",
                        type_name(ty),
                        v.ty.name()
                    ),
                )
            }
            Some(_) => {}
        }
    }
}

/// Signal type a parameter takes from a variable; `None` if it cannot be bound
/// (the value is needed while generating: pins, durations, choices, flags).
pub fn bindable_type(kind: crate::ParamKind, choice: bool, runtime: bool) -> Option<SignalType> {
    use crate::ParamKind;
    if choice {
        return None;
    }
    match kind {
        ParamKind::Text => Some(SignalType::String),
        ParamKind::Int => Some(SignalType::Int),
        ParamKind::Float => Some(SignalType::Float),
        ParamKind::Bool if runtime => Some(SignalType::Bool),
        _ => None,
    }
}

pub(super) fn check_unconnected_inputs(
    logic: &Logic,
    defs: &[Option<&BlockDef>],
    report: &mut Report,
) {
    for (bi, block) in logic.blocks.iter().enumerate() {
        let Some(def) = defs[bi] else { continue };
        let required = def
            .active_inputs(&block.params)
            .filter(|i| i.default.is_none());
        for input in required {
            // Any link counts: an invalid one was already reported, don't pile up.
            let fed = logic
                .links
                .iter()
                .any(|l| l.to.block == block.id && l.to.port == input.name);
            if !fed
                && !block.consts.contains_key(&input.name)
                && !block.bound.contains_key(&input.name)
            {
                report.block_error(
                    DiagKind::InputEmpty,
                    &block.id,
                    format!(
                        "блок «{}»: на вход «{}» ничего не подано",
                        block.id, input.name
                    ),
                );
            }
        }
    }
}
