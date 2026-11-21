//! Commands for the electrical panel (`specs/electrical-schematics`, task 10): read the panel,
//! add, change and remove devices, bind them to the inputs and outputs of the project, build the
//! documentation. The same commands serve the chat and the MCP server.

use crab_core::{check_panel, panel_tags, Device, DeviceKind, Library, Project, Severity, Supply};
use crab_schematic::{build_document_in, Format, Lang};
use serde_json::{json, Map, Value};

use crate::{Context, Tool};

/// Names of the panel commands.
pub const NAMES: [&str; 6] = [
    "get_panel",
    "add_panel_device",
    "set_panel_device",
    "remove_panel_device",
    "set_panel_supply",
    "build_panel",
];

pub fn tools() -> Vec<Tool> {
    let kinds = [
        "feed",
        "breaker",
        "rcd",
        "psu",
        "contactor",
        "relay",
        "motor",
        "load",
        "terminal",
        "sensor",
        "button",
    ];
    let text = |d: &str| json!({"type": "string", "description": d});
    let fields = json!({
        "rating": text("номинал для чертежа и спецификации, например «C16» или «24 В, 5 А»"),
        "part": text("артикул, вводится вручную"),
        "bind": text("вход или выход проекта: имя тега или «блок.порт»; у контактора и реле — выход, у датчика и кнопки — вход"),
        "load": text("обозначение устройства, которое это коммутирует или питает (у контактора K1 — M1)"),
    });
    let with = |extra: Value| {
        let mut props = fields.as_object().cloned().unwrap_or_default();
        props.extend(extra.as_object().cloned().unwrap_or_default());
        Value::Object(props)
    };
    vec![
        Tool {
            name: "get_panel",
            description: "Прочитать электрический щит: питание, устройства, номера проводов, свободные входы и выходы проекта, к которым можно привязать устройства, и результат проверки щита. Проверка щита отдельная от проверки схемы и сборку не блокирует.",
            input_schema: json!({"type": "object", "properties": {}}),
        },
        Tool {
            name: "add_panel_device",
            description: "Добавить устройство в щит. Без id обозначение даётся само по виду (K1, Q1, M1…). Привязка bind — только для контактора, реле, датчика и кнопки; load — что коммутирует или питает устройство.",
            input_schema: json!({"type": "object", "properties": with(json!({
                "kind": {"type": "string", "enum": kinds, "description": "вид устройства"},
                "id": text("обозначение без тире: K1, Q1; буквы, цифры, _ и -"),
            })), "required": ["kind"]}),
        },
        Tool {
            name: "set_panel_device",
            description: "Изменить устройство щита. Указанные поля заменяются; пустая строка или null очищает номинал, артикул, привязку и load. new_id переименовывает устройство, ссылки load и номера проводов следуют за ним.",
            input_schema: json!({"type": "object", "properties": with(json!({
                "id": text("обозначение устройства"),
                "new_id": text("новое обозначение"),
                "kind": {"type": "string", "enum": kinds},
            })), "required": ["id"]}),
        },
        Tool {
            name: "remove_panel_device",
            description: "Удалить устройство из щита; ссылки load на него очищаются.",
            input_schema: json!({"type": "object", "properties": {"id": text("обозначение")}, "required": ["id"]}),
        },
        Tool {
            name: "set_panel_supply",
            description: "Питание щита: dc24 (24 В постоянного), ac230 (230 В, одна фаза) или ac400 (400 В, три фазы).",
            input_schema: json!({"type": "object", "properties": {
                "supply": {"type": "string", "enum": ["dc24", "ac230", "ac400"]}
            }, "required": ["supply"]}),
        },
        Tool {
            name: "build_panel",
            description: "Собрать документацию щита в PDF: титульный лист, силовая часть, цепи управления, перечень клемм, перечень проводов, спецификация. Номера проводов запоминаются в проекте. Файл по умолчанию `<имя проекта>_panel.pdf` рядом с проектом. Ошибки щита перечислены в ответе, но сборку не останавливают.",
            input_schema: json!({"type": "object", "properties": {
                "lang": {"type": "string", "enum": ["ru", "en"], "description": "язык подписей, по умолчанию ru"},
                "format": {"type": "string", "enum": ["a3", "a4"], "description": "формат листов, по умолчанию a3"},
                "out": {"type": "string", "description": "путь файла PDF"}
            }}),
        },
    ]
}

/// The check of the panel in the form of the other reports: `errors` and `warnings`.
pub fn report(project: &Project, lib: &Library) -> Value {
    let r = check_panel(project, lib);
    let list = |sev: Severity| -> Vec<Value> {
        r.diagnostics
            .iter()
            .filter(|d| d.severity == sev)
            .map(|d| json!({"kind": d.kind.id(), "message": d.message}))
            .collect()
    };
    json!({"errors": list(Severity::Error), "warnings": list(Severity::Warning)})
}

fn read(project: &Project, lib: &Library) -> Value {
    let panel = &project.panel;
    let free: Vec<Value> = panel_tags(project, lib)
        .into_iter()
        .filter(|t| {
            !panel
                .devices
                .iter()
                .any(|d| d.bind.split('.').next() == Some(t.name.as_str()))
        })
        .map(|t| json!({"name": t.name, "direction": if t.input { "input" } else { "output" }, "digital": t.digital}))
        .collect();
    json!({
        "supply": panel.supply,
        "devices": panel.devices,
        "wire_numbers": panel.wire_numbers,
        "free_tags": free,
        "check": report(project, lib),
    })
}

/// A text argument: `None` if absent, `Some("")` for null (clear).
fn text(args: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(String::new())),
        Some(Value::String(s)) => Ok(Some(s.trim().to_owned())),
        Some(_) => Err(format!("аргумент «{key}» должен быть строкой")),
    }
}

fn kind(text: &str) -> Result<DeviceKind, String> {
    serde_json::from_value(json!(text)).map_err(|_| {
        format!(
            "нет вида устройства «{text}»: feed, breaker, rcd, psu, contactor, relay, motor, load, terminal, sensor, button"
        )
    })
}

/// Sets the fields the arguments name.
fn apply(device: &mut Device, args: &Map<String, Value>) -> Result<(), String> {
    if let Some(v) = text(args, "rating")? {
        device.rating = v;
    }
    if let Some(v) = text(args, "part")? {
        device.part = v;
    }
    if let Some(v) = text(args, "bind")? {
        device.bind = v;
    }
    if let Some(v) = text(args, "load")? {
        device.load = v;
    }
    Ok(())
}

/// Runs a panel command; `None` if `name` is not one.
pub fn call(ctx: &mut dyn Context, name: &str, args: &Value) -> Option<Result<Value, String>> {
    if !NAMES.contains(&name) {
        return None;
    }
    let empty = Map::new();
    let args = args.as_object().unwrap_or(&empty);
    Some(run(ctx, name, args))
}

fn run(ctx: &mut dyn Context, name: &str, args: &Map<String, Value>) -> Result<Value, String> {
    let lib = ctx.library().clone();
    let id = text(args, "id")?.filter(|s| !s.is_empty());
    let need_id = || id.clone().ok_or_else(|| "нужен аргумент «id»".to_owned());
    match name {
        "get_panel" => Ok(read(ctx.project(), &lib)),
        "add_panel_device" => {
            let kind = kind(&text(args, "kind")?.ok_or("нужен аргумент «kind»")?)?;
            let mut added = String::new();
            ctx.edit(&mut |p| {
                let panel = &mut p.panel;
                let new = id.clone().unwrap_or_else(|| panel.next_id(kind));
                if !crab_core::ident::is_valid_ident(&new) {
                    return Err(format!(
                        "недопустимое обозначение «{new}» (буквы, цифры, «_» и «-», без пробелов)"
                    ));
                }
                if panel.device(&new).is_some() {
                    return Err(format!("обозначение «{new}» уже занято"));
                }
                let mut device = Device {
                    id: new.clone(),
                    kind,
                    rating: String::new(),
                    part: String::new(),
                    bind: String::new(),
                    load: String::new(),
                };
                apply(&mut device, args)?;
                panel.devices.push(device);
                added = new;
                Ok(())
            })?;
            Ok(json!({"id": added, "check": report(ctx.project(), &lib)}))
        }
        "set_panel_device" => {
            let id = need_id()?;
            let new_kind = text(args, "kind")?.map(|k| kind(&k)).transpose()?;
            let new_id = text(args, "new_id")?.filter(|s| !s.is_empty());
            ctx.edit(&mut |p| {
                let panel = &mut p.panel;
                let device = panel
                    .devices
                    .iter_mut()
                    .find(|d| d.id == id)
                    .ok_or_else(|| format!("в щите нет устройства «{id}»"))?;
                if let Some(k) = new_kind {
                    device.kind = k;
                }
                apply(device, args)?;
                if let Some(new) = &new_id {
                    panel.rename(&id, new)?;
                }
                Ok(())
            })?;
            Ok(json!({"ok": true, "check": report(ctx.project(), &lib)}))
        }
        "remove_panel_device" => {
            let id = need_id()?;
            ctx.edit(&mut |p| {
                if p.panel.device(&id).is_none() {
                    return Err(format!("в щите нет устройства «{id}»"));
                }
                p.panel.remove(&id);
                Ok(())
            })?;
            Ok(json!({"ok": true, "check": report(ctx.project(), &lib)}))
        }
        "set_panel_supply" => {
            let text = text(args, "supply")?.ok_or("нужен аргумент «supply»")?;
            let supply: Supply = serde_json::from_value(json!(text))
                .map_err(|_| "питание: dc24, ac230 или ac400".to_owned())?;
            ctx.edit(&mut |p| {
                p.panel.supply = supply;
                Ok(())
            })?;
            Ok(json!({"ok": true, "check": report(ctx.project(), &lib)}))
        }
        "build_panel" => build(ctx, &lib, args),
        _ => unreachable!("checked by NAMES"),
    }
}

fn build(ctx: &mut dyn Context, lib: &Library, args: &Map<String, Value>) -> Result<Value, String> {
    let lang = match text(args, "lang")?.as_deref() {
        None | Some("") => Lang::Ru,
        Some(code) => Lang::from_code(code).ok_or("язык: ru или en")?,
    };
    let format = match text(args, "format")?.as_deref() {
        None | Some("") => Format::A3,
        Some(code) => Format::from_code(code).ok_or("формат: a3 или a4")?,
    };
    if ctx.project().panel.devices.is_empty() {
        return Err("щит пуст: сначала добавьте устройства (add_panel_device)".to_owned());
    }
    let (doc, nets) =
        build_document_in(ctx.project(), lib, lang, format).map_err(|e| e.to_string())?;
    // The numbers of the wires stay the same at the next build.
    if nets.wire_numbers() != ctx.project().panel.wire_numbers {
        ctx.edit(&mut |p| {
            nets.store(&mut p.panel);
            Ok(())
        })?;
    }
    let out = match text(args, "out")?.filter(|s| !s.is_empty()) {
        Some(path) => std::path::PathBuf::from(path),
        None => {
            let p = ctx.project();
            p.path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join(format!("{}_panel.pdf", p.display_name()))
        }
    };
    let pdf = crab_pdf::compile(crab_schematic::doc::typst_source(&doc, lang))
        .map_err(|e| e.to_string())?;
    std::fs::write(&out, pdf).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(json!({
        "file": out.display().to_string(),
        "sheets": doc.sheets.len(),
        "wires": nets.wires.len(),
        "check": report(ctx.project(), lib),
    }))
}
