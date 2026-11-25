//! Commands for the electrical panel (`specs/electrical-schematics`, task 10): read the panel,
//! add, change and remove devices, bind them to the inputs and outputs of the project, build the
//! documentation. The same commands serve the chat and the MCP server.

use crab_core::{
    check_panel, link_bus, link_device, panel_tags, rebind_device, split_part, symbol_key,
    ContactForm, Device, DeviceKind, Library, Panel, Project, Severity, SheetId, Supply, WireLink,
    MAX_SHEETS,
};
use crab_schematic::{build_document_in, Format, Lang, BOARD};
use serde_json::{json, Map, Value};

use crate::{Context, Tool};

/// Names of the panel commands.
pub const NAMES: [&str; 10] = [
    "get_panel",
    "add_panel_device",
    "set_panel_device",
    "reconnect_panel_device",
    "move_panel_symbol",
    "link_panel_terminals",
    "place_panel_device",
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
        "aux",
    ];
    let text = |d: &str| json!({"type": "string", "description": d});
    let fields = json!({
        "rating": text("номинал для чертежа и спецификации, например «C16» или «24 В, 5 А»"),
        "part": text("артикул, вводится вручную"),
        "bind": text("вход или выход проекта: имя тега или «блок.порт»; у контактора и реле — выход, у датчика и кнопки — вход"),
        "load": text("обозначение устройства, которое это коммутирует или питает (у контактора K1 — M1)"),
        "of": text("для kind=aux: обозначение аппарата (автомат, УЗО, контактор, реле), которому принадлежит доп. контакт; для kind=sensor: обозначение двигателя, температуру которого датчик меряет (термистор)"),
        "form": json!({"type": "string", "enum": ["no", "nc", "changeover"], "description": "только для kind=aux: no — замыкающий (клеммы 13-14), nc — размыкающий (21-22), changeover — переключающий (11-12-14)"}),
        "series": text("только для kind=aux: обозначение контактора или реле, в цепи катушки которого контакт стоит последовательно; пусто — контакт идёт на вход платы (bind); тогда bind очищается"),
        "from": text("обозначение источника, от которого питается ещё одно устройство, если источник уже питает другое по полю load (ветвление линии; точка соединения рисуется на листе сама); пусто — питается как обычно"),
        "poles": json!({"type": ["integer", "null"], "description": "число полюсов автомата, УЗО, контактора или реле: 1 или 2 при 230 В, 3 или 4 при 400 В; null — все проводники питания"}),
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
            description: "Добавить устройство в щит. Без id обозначение даётся само по виду (K1, Q1, M1…). Привязка bind — только для контактора, реле, датчика, кнопки и доп. контакта; load — что коммутирует или питает устройство. Доп. контакт (kind=aux) — обязательно of; без id обозначение вида Q1_1.",
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
            name: "reconnect_panel_device",
            description: "Изменить настоящую связь устройства щита с проверкой (в отличие от set_panel_device, который записывает поля как есть): кто его питает (fed_by), ветка линии другого устройства (branch_of), вход или выход проекта, к которому оно привязано (bind). Пустая строка или null снимают связь. Все изменения одним шагом: при отказе не меняется ничего. Возвращается результат проверки щита.",
            input_schema: json!({"type": "object", "properties": {
                "id": text("обозначение устройства"),
                "fed_by": {"type": ["string", "null"], "description": "обозначение аппарата, который питает устройство; null — снять питание от аппарата"},
                "branch_of": {"type": ["string", "null"], "description": "обозначение устройства, линию которого устройство разветвляет; null — снять ветку"},
                "bind": {"type": ["string", "null"], "description": "вход или выход проекта: имя тега или «блок.порт»; null — снять привязку"},
            }, "required": ["id"]}),
        },
        Tool {
            name: "move_panel_symbol",
            description: "Сдвинуть условное изображение устройства на листах щита относительно его автоматического места; связи, зоны и перекрёстные ссылки следуют за ним. reset: true возвращает изображение на автоматическое место.",
            input_schema: json!({"type": "object", "properties": {
                "id": text("обозначение устройства"),
                "view": {"type": "string", "enum": ["power", "control"], "description": "power — силовая часть, control — цепи управления; по умолчанию power"},
                "dx_mm": {"type": "number", "description": "сдвиг вправо в мм (отрицательный — влево); задаётся абсолютно, а не прибавляется"},
                "dy_mm": {"type": "number", "description": "сдвиг вниз в мм (отрицательный — вверх); задаётся абсолютно, а не прибавляется"},
                "reset": {"type": "boolean", "description": "true — вернуть на автоматическое место, dx_mm и dy_mm не учитываются"},
            }, "required": ["id"]}),
        },
        Tool {
            name: "link_panel_terminals",
            description: "Провести провод вручную между двумя выводами щита: K1:A2, A1:IO5 (вывод платы), bus:L1 (линия питания). Провод заменяет автоматический на этих выводах. remove: true — удалить такой провод (порядок концов не важен). Возвращается результат проверки щита.",
            input_schema: json!({"type": "object", "properties": {
                "from": text("первый вывод: устройство:вывод"),
                "to": text("второй вывод: устройство:вывод или bus:линия"),
                "remove": {"type": "boolean", "description": "true — удалить провод; по умолчанию добавить"},
            }, "required": ["from", "to"]}),
        },
        Tool {
            name: "place_panel_device",
            description: "Поставить устройство щита на лист вручную (или часть устройства, например K1/coil) в мм от левого верхнего угла листа. remove: true — убрать с листа; устройство остаётся в щите. Лист: power:0, control:1 или free:2.",
            input_schema: json!({"type": "object", "properties": {
                "id": text("обозначение устройства или части, например K1 или K1/coil"),
                "sheet": text("лист: power:N, control:N или free:N"),
                "x_mm": {"type": "number", "description": "положение по горизонтали на листе, мм"},
                "y_mm": {"type": "number", "description": "положение по вертикали на листе, мм"},
                "remove": {"type": "boolean", "description": "true — убрать устройство с листа"},
            }, "required": ["id"]}),
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
    // Moved symbols; the bus and label offsets belong to the wires, not to the devices.
    let layout: Vec<Value> = project
        .panel_layout
        .symbols
        .iter()
        .filter(|(key, _)| !key.starts_with("bus:") && !key.starts_with("label:"))
        .map(|(key, s)| json!({"key": key, "dx_mm": s.dx / 4.0, "dy_mm": s.dy / 4.0}))
        .collect();
    json!({
        "supply": panel.supply,
        "devices": panel.devices,
        "wire_numbers": panel.wire_numbers,
        "free_tags": free,
        "links": panel.links,
        "layout": layout,
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

/// A connection argument that may be cleared: `None` if absent, `Some(None)` for null or an
/// empty string, `Some(Some(name))` otherwise.
fn opt_text(args: &Map<String, Value>, key: &str) -> Result<Option<Option<String>>, String> {
    Ok(text(args, key)?.map(|s| (!s.is_empty()).then_some(s)))
}

/// Largest offset of a symbol, in millimetres: more than the longest sheet (A3, 420 mm).
const MAX_SHIFT_MM: f64 = 1000.0;

/// A length in millimetres; absent or null counts as zero. Only numbers within
/// `±MAX_SHIFT_MM` are accepted.
fn millimetres(args: &Map<String, Value>, key: &str) -> Result<f32, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(0.0),
        Some(v) => v
            .as_f64()
            .filter(|x| x.abs() <= MAX_SHIFT_MM)
            .map(|x| x as f32)
            .ok_or_else(|| {
                format!("аргумент «{key}»: число миллиметров от −{MAX_SHIFT_MM} до {MAX_SHIFT_MM}")
            }),
    }
}

/// A flag argument; absent counts as `false`.
fn flag(args: &Map<String, Value>, key: &str) -> Result<bool, String> {
    match args.get(key) {
        None => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(format!("аргумент «{key}» должен быть true или false")),
    }
}

fn kind(text: &str) -> Result<DeviceKind, String> {
    serde_json::from_value(json!(text)).map_err(|_| {
        format!(
            "нет вида устройства «{text}»: feed, breaker, rcd, fuse, switch, psu, contactor, relay, motor, load, terminal, sensor, button, aux"
        )
    })
}

/// Checks one end of a hand wire: `device:terminal` of a device of the panel or of the board
/// (`A1:IO5`), or a supply line `bus:L1`.
fn check_end(panel: &Panel, end: &str) -> Result<(), String> {
    if let Some(line) = link_bus(end) {
        let mut lines: Vec<&str> = vec!["+24V", "0V"];
        lines.extend(panel.supply.lines());
        return if lines.contains(&line) {
            Ok(())
        } else {
            Err(format!(
                "в щите нет линии «{line}»; есть: {}",
                lines.join(", ")
            ))
        };
    }
    let well_formed = end
        .split_once(':')
        .is_some_and(|(device, terminal)| !device.is_empty() && !terminal.is_empty());
    if !well_formed {
        return Err(format!(
            "вывод «{end}»: нужен вид устройство:вывод, например K1:A2"
        ));
    }
    let device = link_device(end);
    if device != BOARD && panel.device(device).is_none() {
        return Err(format!("в щите нет устройства «{device}»"));
    }
    // A terminal of a device of the panel must be one the sheet draws; the board and its channels
    // are not checked here.
    if let (Some(d), Some((_, terminal))) = (panel.device(device), end.split_once(':')) {
        let terminals = crab_schematic::nets::terminals_of(panel, d);
        if !terminals.iter().any(|t| t == terminal) {
            return Err(format!(
                "у устройства «{device}» нет вывода «{terminal}»; есть: {}",
                terminals.join(", ")
            ));
        }
    }
    Ok(())
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
    if let Some(v) = text(args, "of")? {
        device.of = v;
    }
    if let Some(v) = text(args, "from")? {
        device.from = v;
    }
    if let Some(v) = text(args, "form")? {
        device.form = match v.as_str() {
            "" | "no" => ContactForm::No,
            "nc" => ContactForm::Nc,
            "changeover" | "co" => ContactForm::Changeover,
            _ => return Err(
                "форма контакта: no (замыкающий), nc (размыкающий) или changeover (переключающий)"
                    .to_owned(),
            ),
        };
    }
    if let Some(v) = text(args, "series")? {
        // The circuit is one thing: the coil or an input of the board.
        if !v.is_empty() && text(args, "bind")?.is_none() {
            device.bind.clear();
        }
        device.series = v;
    }
    match args.get("poles") {
        None => {}
        Some(Value::Null) => device.poles = None,
        Some(v) => {
            device.poles = Some(
                v.as_u64()
                    .and_then(|n| u8::try_from(n).ok())
                    .filter(|n| (1..=4).contains(n))
                    .ok_or("аргумент «poles» — целое число от 1 до 4")?,
            );
        }
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
            let of = text(args, "of")?.unwrap_or_default();
            let mut added = String::new();
            ctx.edit(&mut |p| {
                let panel = &mut p.panel;
                if kind == DeviceKind::Aux && of.is_empty() {
                    return Err(
                        "у доп. контакта нужен аргумент «of» — аппарат, которому он принадлежит"
                            .to_owned(),
                    );
                }
                let new = match (&id, kind) {
                    (Some(id), _) => id.clone(),
                    (None, DeviceKind::Aux) => panel.next_aux_id(&of),
                    (None, _) => panel.next_id(kind),
                };
                if !crab_core::panel::is_valid_designation(&new) {
                    return Err(format!(
                        "недопустимое обозначение «{new}» (буквы, цифры, «_», «-» и «.», без пробелов)"
                    ));
                }
                if panel.device(&new).is_some() {
                    return Err(format!("обозначение «{new}» уже занято"));
                }
                let mut device = Device {
                    id: new.clone(),
                    kind,
                    via: String::new(),
                    rating: String::new(),
                    part: String::new(),
                    bind: String::new(),
                    load: String::new(),
                    poles: None,
                    of: String::new(),
                    form: ContactForm::default(),
                    pins: String::new(),
                    from: String::new(),
                    free: false,
                    series: String::new(),
                    conductors: Vec::new(),
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
                // The manual layout of the sheets follows the new designation.
                if let Some(new) = &new_id {
                    crab_core::rename_device(p, &id, new)?;
                }
                Ok(())
            })?;
            Ok(json!({"ok": true, "check": report(ctx.project(), &lib)}))
        }
        "reconnect_panel_device" => {
            let id = need_id()?;
            let fed_by = opt_text(args, "fed_by")?;
            let branch_of = opt_text(args, "branch_of")?;
            let bind = opt_text(args, "bind")?;
            if fed_by.is_none() && branch_of.is_none() && bind.is_none() {
                return Err("укажите fed_by, branch_of или bind".to_owned());
            }
            // One edit: a refused step leaves the other links as they were.
            ctx.edit(&mut |p| {
                if p.panel.device(&id).is_none() {
                    return Err(format!("в щите нет устройства «{id}»"));
                }
                if let Some(parent) = &fed_by {
                    p.panel.reconnect_power(&id, parent.as_deref())?;
                }
                if let Some(source) = &branch_of {
                    p.panel.branch_power(&id, source.as_deref())?;
                }
                if let Some(tag) = &bind {
                    rebind_device(p, &lib, &id, tag.as_deref())?;
                }
                Ok(())
            })?;
            Ok(json!({"ok": true, "check": report(ctx.project(), &lib)}))
        }
        "move_panel_symbol" => {
            let id = need_id()?;
            let view = text(args, "view")?
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "power".to_owned());
            if view != "power" && view != "control" {
                return Err("вид: power или control".to_owned());
            }
            let reset = flag(args, "reset")?;
            // Offsets are stored in units of 1/4 mm.
            let dx = millimetres(args, "dx_mm")? * 4.0;
            let dy = millimetres(args, "dy_mm")? * 4.0;
            let key = symbol_key(&view, &id);
            ctx.edit(&mut |p| {
                if p.panel.device(&id).is_none() {
                    return Err(format!("в щите нет устройства «{id}»"));
                }
                if reset {
                    p.panel_layout.clear_symbol(&key);
                } else {
                    p.panel_layout.set_shift(&key, dx, dy);
                }
                Ok(())
            })?;
            // The offset as stored, after snapping to the grid.
            let (sx, sy) = ctx.project().panel_layout.shift(&key);
            Ok(json!({"id": id, "view": view, "dx_mm": sx / 4.0, "dy_mm": sy / 4.0}))
        }
        "link_panel_terminals" => {
            let arg = |key: &str| -> Result<String, String> {
                text(args, key)?
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| format!("нужен аргумент «{key}»"))
            };
            let from = arg("from")?;
            let to = arg("to")?;
            let remove = flag(args, "remove")?;
            let key = WireLink {
                from: from.clone(),
                to: to.clone(),
            }
            .key();
            ctx.edit(&mut |p| {
                let panel = &mut p.panel;
                // A wire has the same two ends in either order.
                let same =
                    |l: &WireLink| (l.from == from && l.to == to) || (l.from == to && l.to == from);
                if remove {
                    let before = panel.links.len();
                    panel.links.retain(|l| !same(l));
                    if panel.links.len() == before {
                        return Err(format!("провода «{key}» нет"));
                    }
                    return Ok(());
                }
                if from == to {
                    return Err("провод не может начинаться и кончаться на одном выводе".to_owned());
                }
                check_end(panel, &from)?;
                check_end(panel, &to)?;
                if link_bus(&from).is_some() && link_bus(&to).is_some() {
                    return Err("провод между двумя линиями не рисуется".to_owned());
                }
                if panel.links.iter().any(same) {
                    return Err(format!("провод «{key}» уже есть"));
                }
                panel.links.push(WireLink {
                    from: from.clone(),
                    to: to.clone(),
                });
                Ok(())
            })?;
            Ok(json!({"key": key, "check": report(ctx.project(), &lib)}))
        }
        "place_panel_device" => {
            let id = need_id()?;
            // A part key (`K1/coil`) belongs to its device.
            let device = split_part(&id).0.to_owned();
            let remove = flag(args, "remove")?;
            if remove {
                ctx.edit(&mut |p| {
                    if p.panel.device(&device).is_none() {
                        return Err(format!("в щите нет устройства «{device}»"));
                    }
                    p.panel_layout.unplace(&id);
                    Ok(())
                })?;
                return Ok(json!({"ok": true, "id": id}));
            }
            let sheet = text(args, "sheet")?
                .filter(|s| !s.is_empty())
                .ok_or("нужен аргумент «sheet»: power:N, control:N или free:N")?
                .parse::<SheetId>()?;
            if sheet.number() >= MAX_SHEETS {
                return Err(format!("лист «{sheet}»: номер листа меньше {MAX_SHEETS}"));
            }
            if !args.contains_key("x_mm") || !args.contains_key("y_mm") {
                return Err("нужны аргументы «x_mm» и «y_mm» — место на листе в мм".to_owned());
            }
            // Positions are stored in units of 1/4 mm.
            let x = millimetres(args, "x_mm")? * 4.0;
            let y = millimetres(args, "y_mm")? * 4.0;
            ctx.edit(&mut |p| {
                if p.panel.device(&device).is_none() {
                    return Err(format!("в щите нет устройства «{device}»"));
                }
                p.panel_layout.place(&id, sheet, x, y);
                Ok(())
            })?;
            // The place as stored, after snapping to the grid.
            let at = ctx.project().panel_layout.placed.get(&id).map(
                |p| json!({"sheet": p.sheet.to_string(), "x_mm": p.x / 4.0, "y_mm": p.y / 4.0}),
            );
            Ok(json!({"id": id, "placed": at}))
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
