//! Network services in the left panel: Wi-Fi, Ethernet, the web interface and MQTT.
//! A service is a hidden block of the schema: the check box adds or removes it,
//! the settings are its parameters, and every port or text/number parameter can
//! be taken out to the board as a variable (a binding stored in the block).

use crab_core::project::MODBUS_KINDS;
use crab_core::{ParamValue, SignalType, Var, VarType};
use eframe::egui::{self, Id, Ui};

use crate::app::{EditorApp, LogLevel};
use crate::canvas::DragItem;
use crate::dialogs::Dialog;
use crate::radiopanel::RadioAction;

const NETWORKS: [(&str, &str); 2] = [("wifi", "Wi-Fi"), ("ethernet", "Ethernet")];
const WEB_UI: &str = "web_ui";
const MQTT: &str = "mqtt";
/// MQTT topic blocks of the left panel: (kind, list title, add button).
const TOPICS: [(&str, &str, &str); 2] = [
    ("mqtt_publish", "Публикации", "+ публикация"),
    ("mqtt_subscribe", "Подписки", "+ подписка"),
];

pub enum NetAction {
    Toggle {
        kind: &'static str,
        on: bool,
    },
    SetParam {
        id: String,
        name: String,
        value: ParamValue,
    },
    Settings(String),
    /// Adds a hidden MQTT publication or subscription block.
    AddTopic(&'static str),
    Remove(String),
    Designer,
    Focus(String),
    /// Virtual bench in simulation: the network link is lost or back.
    BenchOffline(bool),
    /// Virtual bench in simulation: the MQTT broker stops answering or comes back.
    BenchBrokerDown(bool),
}

/// The «Сеть» folder: Wi-Fi, Ethernet, the web interface, MQTT and the «Modbus» folder (whose
/// actions go to `modbus`, they are the radio ones).
pub fn show(app: &EditorApp, ui: &mut Ui, modbus: &mut Vec<RadioAction>) -> Vec<NetAction> {
    let mut actions = Vec::new();
    let Some(doc) = &app.doc else {
        return actions;
    };
    let logic = &doc.project.logic;
    let has_net = NETWORKS
        .iter()
        .any(|(kind, _)| logic.blocks.iter().any(|b| b.kind == *kind));
    let used = has_net
        || logic
            .blocks
            .iter()
            .any(|b| MODBUS_KINDS.contains(&b.kind.as_str()));
    ui.add_space(4.0);
    egui::CollapsingHeader::new(egui::RichText::new(crate::i18n::tr("Сеть")).strong())
        .id_salt("network_folder")
        .default_open(used)
        .show(ui, |ui| {
            for (kind, title) in NETWORKS {
                service(app, ui, kind, title, true, &mut actions);
            }
            // The web interface and MQTT ride on a network service: their sections appear with one.
            if has_net {
                service(
                    app,
                    ui,
                    WEB_UI,
                    crate::i18n::tr("Веб-интерфейс"),
                    true,
                    &mut actions,
                );
                service(app, ui, MQTT, "MQTT", true, &mut actions);
                bench_section(app, ui, &mut actions);
            }
            crate::modbuspanel::show(app, ui, modbus);
        });
    ui.separator();
    actions
}

/// Faults of the virtual bench for the hidden network blocks, which have no ports on a sheet.
fn bench_section(app: &EditorApp, ui: &mut Ui, actions: &mut Vec<NetAction>) {
    let (Some(sim), Some(doc)) = (&app.sim, &app.doc) else {
        return;
    };
    ui.add_space(4.0);
    ui.label(egui::RichText::new(crate::i18n::tr("Виртуальный стенд")).strong());
    let mut offline = sim.offline();
    if ui
        .checkbox(&mut offline, crate::i18n::tr("Нет связи с сетью"))
        .changed()
    {
        actions.push(NetAction::BenchOffline(offline));
    }
    if doc.project.logic.blocks.iter().any(|b| b.kind == MQTT) {
        let mut down = sim.broker_down();
        if ui
            .checkbox(&mut down, crate::i18n::tr("Брокер не отвечает"))
            .changed()
        {
            actions.push(NetAction::BenchBrokerDown(down));
        }
    }
    ui.separator();
}

fn service(
    app: &EditorApp,
    ui: &mut Ui,
    kind: &'static str,
    title: &str,
    available: bool,
    actions: &mut Vec<NetAction>,
) {
    let Some(doc) = &app.doc else { return };
    let logic = &doc.project.logic;
    let block = logic.blocks.iter().find(|b| b.kind == kind);
    ui.add_space(4.0);
    // The web page is switched by a parameter: its settings and widgets stay.
    let web_switch = kind == WEB_UI && block.is_some();
    let mut on = match block {
        Some(b) if web_switch => !matches!(b.params.get("enabled"), Some(ParamValue::Bool(false))),
        _ => block.is_some(),
    };
    let toggle = |on: bool, actions: &mut Vec<NetAction>| match block {
        Some(b) if web_switch => actions.push(NetAction::SetParam {
            id: b.id.clone(),
            name: "enabled".into(),
            value: ParamValue::Bool(on),
        }),
        _ => actions.push(NetAction::Toggle { kind, on }),
    };
    let pullable = block.is_some_and(|b| {
        app.lib
            .resolve(logic, b)
            .is_some_and(|l| l.def.info.pullable)
    });
    // A flat row like a variable: the title is dragged onto a sheet with the whole block,
    // a double click opens the settings, the switch turns the service on and off.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let title_text = egui::RichText::new(title).strong();
        let resp = match block {
            Some(b) if pullable => {
                let placed = doc.project.layout.blocks.contains_key(&b.id);
                let selected = app.selection.as_ref().is_some_and(|s| s.has_block(&b.id));
                crate::panels::drag_row(
                    ui,
                    Id::unique(("service", &b.id)),
                    DragItem::Topic(b.id.clone()),
                    selected,
                    title_text,
                )
                .on_hover_text(if placed {
                    crate::i18n::tr(
                        "Двойной щелчок — настройки; перетащите, чтобы переставить блок на плате",
                    )
                } else {
                    crate::i18n::tr("Двойной щелчок — настройки; перетащите весь блок на плату")
                })
            }
            _ => ui.selectable_label(false, title_text),
        };
        if let Some(b) = block {
            if resp.double_clicked() {
                actions.push(NetAction::Settings(b.id.clone()));
            } else if resp.clicked() && doc.project.layout.blocks.contains_key(&b.id) {
                actions.push(NetAction::Focus(b.id.clone()));
            }
        }
        resp.context_menu(|ui| {
            if let Some(b) = block {
                if ui.button(crate::i18n::tr("Настройки...")).clicked() {
                    actions.push(NetAction::Settings(b.id.clone()));
                    ui.close();
                }
            }
            let mut menu_on = on;
            if ui
                .add_enabled(
                    available || on,
                    egui::Checkbox::new(&mut menu_on, crate::i18n::tr("Включить")),
                )
                .clicked()
            {
                toggle(menu_on, actions);
                ui.close();
            }
            if block.is_some() && ui.button(crate::i18n::tr("Блок целиком удалить")).clicked()
            {
                actions.push(NetAction::Toggle { kind, on: false });
                ui.close();
            }
        });
        let switch = ui
            .add_enabled_ui(available || on, |ui| {
                crate::dialogs::toggle_switch(ui, &mut on)
            })
            .inner;
        if !available {
            switch.on_hover_text(crate::i18n::tr("Сначала включите Wi-Fi или Ethernet"));
        } else if switch.changed() {
            toggle(on, actions);
        }
        if kind == WEB_UI
            && crate::iconbtn::icon_button(
                ui,
                true,
                crate::iconbtn::Icon::Designer,
                crate::i18n::tr("Конструктор страницы..."),
            )
            .clicked()
        {
            actions.push(NetAction::Designer);
        }
    });
    if block.is_some() && (kind == WEB_UI || kind == MQTT) {
        ui.indent(("net_sub", kind), |ui| {
            if kind == WEB_UI {
                pages_section(app, ui, actions);
            } else {
                topics_section(app, ui, actions);
            }
        });
    }
    ui.separator();
}

/// Lists of MQTT publications and subscriptions: a topic is a hidden block whose settings
/// are edited here and whose ports are taken out to the board like those of Wi-Fi.
fn topics_section(app: &EditorApp, ui: &mut Ui, actions: &mut Vec<NetAction>) {
    let Some(doc) = &app.doc else { return };
    let logic = &doc.project.logic;
    for (kind, title, add) in TOPICS {
        // A group like «Входы»: the title with a small «+», the rows folded under it.
        let pressed = crate::panels::add_group(
            ui,
            ("mqtt_group", kind),
            title,
            crate::i18n::tr(add),
            true,
            |ui| {
                for b in logic.blocks.iter().filter(|b| b.kind == kind) {
                    topic_row(app, ui, b, actions);
                }
            },
        );
        if pressed {
            actions.push(NetAction::AddTopic(kind));
        }
    }
}

/// One topic row, like the row of a variable: dragged onto a sheet whole, a click finds it on the
/// board, a double click opens the settings, the context menu has the rest.
fn topic_row(app: &EditorApp, ui: &mut Ui, b: &crab_core::Block, actions: &mut Vec<NetAction>) {
    let Some(doc) = &app.doc else { return };
    let logic = &doc.project.logic;
    let lib = app.lib.resolve(logic, b);
    let topic = match b.params.get("topic").or_else(|| {
        lib.as_ref()
            .and_then(|l| l.def.param("topic"))
            .and_then(|p| p.default.as_ref())
    }) {
        Some(ParamValue::Text(t)) if !t.is_empty() => t.as_str(),
        _ => crate::i18n::tr("без топика"),
    };
    let placed = doc.project.layout.blocks.contains_key(&b.id);
    let selected = app.selection.as_ref().is_some_and(|s| s.has_block(&b.id));
    let caption = format!("{topic} ({})", b.id);
    let text = crate::panels::row_label(
        ui,
        caption,
        None,
        crate::panels::off_sheet_cross(ui, !placed),
    );
    let resp = crate::panels::drag_row(
        ui,
        Id::unique(("topic", &b.id)),
        DragItem::Topic(b.id.clone()),
        selected,
        text,
    )
    .on_hover_text(if placed {
        crate::i18n::tr("Щелчок — найти на плате. Двойной щелчок — свойства; перетащите на плату, чтобы переставить.")
    } else {
        crate::i18n::tr("Перетащите на плату. Двойной щелчок — свойства.")
    });
    if resp.double_clicked() {
        actions.push(NetAction::Settings(b.id.clone()));
    } else if resp.clicked() && placed {
        actions.push(NetAction::Focus(b.id.clone()));
    }
    resp.context_menu(|ui| {
        if placed && ui.button(crate::i18n::tr("Найти на плате")).clicked() {
            actions.push(NetAction::Focus(b.id.clone()));
            ui.close();
        }
        if ui.button(crate::i18n::tr("Свойства...")).clicked() {
            actions.push(NetAction::Settings(b.id.clone()));
            ui.close();
        }
        ui.separator();
        if ui.button(crate::i18n::tr("Удалить из проекта")).clicked() {
            actions.push(NetAction::Remove(b.id.clone()));
            ui.close();
        }
    });
}

fn widget_blocks(logic: &crab_core::Logic) -> impl Iterator<Item = &crab_core::Block> {
    logic
        .blocks
        .iter()
        .filter(|w| w.kind.starts_with("web_") && w.kind != WEB_UI && w.tag.is_none())
}

/// Names of the web interface pages (at least one).
pub fn page_names(logic: &crab_core::Logic) -> Vec<String> {
    let names: Vec<String> = logic
        .blocks
        .iter()
        .find(|b| b.kind == WEB_UI)
        .and_then(|b| match b.params.get("pages") {
            Some(ParamValue::Text(t)) => Some(t.split(';').map(|n| n.trim().to_owned()).collect()),
            _ => None,
        })
        .unwrap_or_default();
    if names.is_empty() {
        vec![DEFAULT_PAGE.to_owned()]
    } else {
        names
    }
}

const DEFAULT_PAGE: &str = "Главная";

/// 1-based page of a widget, clamped to the existing pages.
fn widget_page(w: &crab_core::Block, pages: usize) -> usize {
    match w.params.get("page") {
        Some(ParamValue::Int(i)) => (*i).clamp(1, pages as i64) as usize,
        _ => 1,
    }
}

/// Pages with their widgets (they are made in the web designer): the name of the page and
/// its widgets; a click on a widget finds it on the sheet, a double click opens its settings.
fn pages_section(app: &EditorApp, ui: &mut Ui, actions: &mut Vec<NetAction>) {
    let Some(doc) = &app.doc else { return };
    let logic = &doc.project.logic;
    let names = page_names(logic);
    ui.separator();
    for (i, name) in names.iter().enumerate() {
        ui.label(format!("{}. {name}", i + 1));
        ui.indent(("page", i), |ui| {
            for w in widget_blocks(logic).filter(|w| widget_page(w, names.len()) == i + 1) {
                let caption = match w.params.get("caption") {
                    Some(ParamValue::Text(c)) if !c.is_empty() => c.as_str(),
                    _ => crate::i18n::tr("без имени"),
                };
                let sel = app.selection.as_ref().is_some_and(|s| s.has_block(&w.id));
                // Like a variable: a widget that is on no sheet yet is highlighted.
                let placed = std::iter::once(w.id.as_str())
                    .chain(logic.tag_copies(&w.id))
                    .any(|id| doc.project.layout.blocks.contains_key(id));
                let text = egui::RichText::new(format!("{caption} ({})", w.id));
                let text = if placed {
                    text
                } else {
                    text.color(ui.visuals().warn_fg_color)
                };
                let text = if sel { text.strong() } else { text };
                let resp = crate::canvas::drag_source(
                    ui,
                    Id::unique(("widget", &w.id)),
                    DragItem::Topic(w.id.clone()),
                    |ui| ui.label(text),
                )
                .on_hover_text(crate::i18n::tr(
                    "Щелчок — найти на плате, двойной щелчок — параметры. Перетащите на плату: можно несколько раз, все изображения работают с одним виджетом. Не поставленный на плату виджет подсвечен.",
                ));
                if resp.double_clicked() {
                    actions.push(NetAction::Settings(w.id.clone()));
                } else if resp.clicked() {
                    actions.push(NetAction::Focus(w.id.clone()));
                }
            }
        });
    }
}

/// Takes a setting out to the board: creates a service variable, binds it, puts its block
/// on the sheet and switches the port on. Returns the id of the block.
pub fn pull_out(
    p: &mut crab_core::Project,
    lib: &crab_core::Library,
    id: &str,
    name: &str,
    ty: SignalType,
    feed: bool,
    pos: crab_core::BlockPos,
) -> Result<String, crab_core::EditError> {
    let block = p.logic.block(id);
    let def = block.and_then(|b| lib.get(&b.kind)).map(|l| &l.def);
    // Already taken out: one more block of the same variable.
    if let Some(var) = block.and_then(|b| b.bound.get(name)).cloned() {
        return p.add_var_block(&var, feed, pos);
    }
    // The variable starts with the current value of the parameter.
    let init = def.filter(|d| d.output(name).is_none()).and_then(|d| {
        // The `mode` input continues the chosen mode.
        let source = if name == "mode" { "wifi_mode" } else { name };
        block
            .and_then(|b| b.params.get(source))
            .or_else(|| d.param(source).and_then(|q| q.default.as_ref()))
            .filter(|v| name != "mode" || matches!(v, ParamValue::Int(0..=2)))
            .cloned()
    });
    let base = format!("{id}_{name}");
    let var = std::iter::once(base.clone())
        .chain((2..).map(|n| format!("{base}_{n}")))
        .find(|n| p.logic.var(n).is_none())
        .expect("infinite range");
    p.add_var(Var {
        name: var.clone(),
        ty: var_type(ty),
        init,
        description: format!("{id}.{name}"),
        retain: false,
        service: true,
        settle_s: None,
        period_s: None,
    })?;
    p.bind(id, name, Some(&var))?;
    p.switch_port(lib, id, name, true)?;
    if name == "mode"
        && p.logic
            .block(id)
            .is_some_and(|b| b.params.contains_key("wifi_mode") || b.kind == "wifi")
    {
        p.set_param(lib, id, "wifi_mode", Some(ParamValue::Int(3)))?;
    }
    p.add_var_block(&var, feed, pos)
}

fn var_type(ty: SignalType) -> VarType {
    match ty {
        SignalType::Bool => VarType::Bool,
        SignalType::Float => VarType::Float,
        SignalType::String => VarType::String,
        SignalType::Int | SignalType::Any | SignalType::Number | SignalType::Same => VarType::Int,
    }
}

pub fn apply(app: &mut EditorApp, actions: Vec<NetAction>) {
    for action in actions {
        let lib = app.lib.clone();
        let pos = crate::panels::free_spot(app);
        match action {
            NetAction::Focus(id) => app.focus_block(&id),
            NetAction::BenchOffline(on) => {
                if let Some(s) = &mut app.sim {
                    s.set_offline(on);
                }
            }
            NetAction::BenchBrokerDown(on) => {
                if let Some(s) = &mut app.sim {
                    s.set_broker_down(on);
                }
            }
            NetAction::Designer => app.open_web_designer("/designer"),
            NetAction::Settings(id) => app.dialog = Dialog::for_block(app, &id),
            NetAction::Toggle { kind, on } => {
                let Some(doc) = &mut app.doc else { return };
                let result = doc.edit(&lib, None, |p| {
                    if on {
                        let id = p.add_block(&lib, kind, pos)?;
                        // The block waits in the panel until it is dragged onto a sheet.
                        p.layout.blocks.remove(&id);
                        Ok(())
                    } else {
                        let ids: Vec<String> = p
                            .logic
                            .blocks
                            .iter()
                            .filter(|b| b.kind == kind)
                            .map(|b| b.id.clone())
                            .collect();
                        ids.iter().try_for_each(|id| p.remove_block(id))
                    }
                });
                if let Err(e) = result {
                    app.info(LogLevel::Error, e.to_string());
                }
            }
            NetAction::AddTopic(kind) => {
                let Some(doc) = &mut app.doc else { return };
                // The topic stays in the panel until it is dragged onto a sheet.
                let added = doc.edit(&lib, None, |p| {
                    let id = p.add_block(&lib, kind, pos)?;
                    p.layout.blocks.remove(&id);
                    Ok::<_, crab_core::EditError>(())
                });
                if let Err(e) = added {
                    app.info(LogLevel::Error, e.to_string());
                }
            }
            NetAction::Remove(id) => {
                let Some(doc) = &mut app.doc else { return };
                if let Err(e) = doc.edit(&lib, None, |p| p.remove_block(&id)) {
                    app.info(LogLevel::Error, e.to_string());
                }
            }
            NetAction::SetParam { id, name, value } => {
                let Some(doc) = &mut app.doc else { return };
                let key = Some(format!("net:{id}.{name}"));
                if let Err(e) = doc.edit(&lib, key, |p| p.set_param(&lib, &id, &name, Some(value)))
                {
                    app.info(LogLevel::Error, e.to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crab_core::{BlockPos, Library, Project};

    const POS: BlockPos = BlockPos {
        sheet: 1,
        x: 0.0,
        y: 0.0,
    };

    #[test]
    fn pulled_output_reads_and_input_writes() {
        let (lib, _) =
            Library::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library")).unwrap();
        let mut p = Project::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/wifi_bound"
        ))
        .unwrap();
        let id = "wifi".to_owned();
        let out = pull_out(&mut p, &lib, &id, "address", SignalType::String, false, POS).unwrap();
        assert_eq!(p.logic.block(&out).unwrap().kind, crab_core::vars::VAR_GET);
        let inp = pull_out(&mut p, &lib, &id, "en", SignalType::Bool, true, POS).unwrap();
        assert_eq!(p.logic.block(&inp).unwrap().kind, crab_core::vars::VAR_SET);
    }

    #[test]
    fn pulled_param_is_an_ordinary_variable() {
        let (lib, _) =
            Library::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library")).unwrap();
        let mut p = Project::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/wifi_bound"
        ))
        .unwrap();
        let first = pull_out(
            &mut p,
            &lib,
            "wifi",
            "subnet",
            SignalType::String,
            true,
            POS,
        )
        .unwrap();
        assert_eq!(
            p.logic.block(&first).unwrap().kind,
            crab_core::vars::VAR_SET
        );
        let again = pull_out(
            &mut p,
            &lib,
            "wifi",
            "subnet",
            SignalType::String,
            true,
            POS,
        )
        .unwrap();
        assert_eq!(
            p.logic.block(&again).unwrap().kind,
            crab_core::vars::VAR_SET
        );
        let var = p
            .logic
            .block(&first)
            .unwrap()
            .var_name()
            .unwrap()
            .to_owned();
        assert!(p.logic.var(&var).unwrap().init.is_some());
    }

    #[test]
    fn pulled_mode_switches_to_input() {
        let (lib, _) =
            Library::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library")).unwrap();
        let mut p = Project::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/wifi_bound"
        ))
        .unwrap();
        pull_out(&mut p, &lib, "wifi", "mode", SignalType::Int, true, POS).unwrap();
        assert_eq!(
            p.logic.block("wifi").unwrap().params.get("wifi_mode"),
            Some(&ParamValue::Int(3))
        );
    }

    #[test]
    fn removing_last_block_releases_the_binding() {
        let (lib, _) =
            Library::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library")).unwrap();
        let mut p = Project::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/wifi_bound"
        ))
        .unwrap();
        let before = p.logic.block("wifi").unwrap().clone();
        let first = pull_out(&mut p, &lib, "wifi", "mode", SignalType::Int, true, POS).unwrap();
        let second = pull_out(&mut p, &lib, "wifi", "mode", SignalType::Int, true, POS).unwrap();
        // One block left: the binding stays.
        p.remove_block(&first).unwrap();
        p.release_unused_services(&lib).unwrap();
        assert!(p.logic.block("wifi").unwrap().bound.contains_key("mode"));
        // The last one gone: everything is back.
        p.remove_block(&second).unwrap();
        p.release_unused_services(&lib).unwrap();
        let after = p.logic.block("wifi").unwrap();
        assert!(!after.bound.contains_key("mode"));
        // The mode is back (explicitly or by default).
        let mode = |b: &crab_core::Block| b.params.get("wifi_mode").cloned();
        assert_eq!(
            mode(after).unwrap_or(ParamValue::Int(0)),
            mode(&before).unwrap_or(ParamValue::Int(0))
        );
        assert!(p.logic.vars.iter().all(|v| !v.service));
    }
}
