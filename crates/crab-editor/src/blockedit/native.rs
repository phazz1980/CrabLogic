use super::*;

/// A starting point for `native.cpp` made from the ports and parameters of the form.
pub(super) fn native_skeleton(f: &Form) -> String {
    let names = |rows: &[PortRow]| {
        rows.iter()
            .map(|r| r.name.trim().to_owned())
            .collect::<Vec<_>>()
    };
    let mut text = String::from(
        "// crab:native\n// Библиотеки Arduino: libs=\"Имя библиотеки, Другая\"; ресурсы (пин, шина, адрес): i2c=0x76\n// Значения для симуляции на компьютере: // crab:sim <выход>=<значение>\n",
    );
    for name in names(&f.inputs) {
        text.push_str(&format!("// вход:    in.{name}\n"));
    }
    for name in names(&f.outputs) {
        text.push_str(&format!("// выход:   out.{name}\n"));
    }
    for p in &f.params {
        text.push_str(&format!(
            "// параметр: p.{}  (только в CRAB_INIT)\n",
            p.name.trim()
        ));
    }
    text.push_str("\nstruct State {\n  int32_t counter;\n};\n\n");
    if !f.params.is_empty() {
        text.push_str("CRAB_INIT(State& s, Params p) {\n  // Один раз и при смене параметров: настройка датчика.\n}\n\n");
    }
    text.push_str("CRAB_STEP(State& s, In in, Out& out) {\n  // Каждый цикл: прочитать датчик и записать выходы.\n");
    for name in names(&f.outputs) {
        text.push_str(&format!("  // out.{name} = ...;\n"));
    }
    text.push_str("}\n");
    text
}

/// Writes the template into the form: the missing parameters and outputs the template needs are
/// added as rows, the code goes into `native`.
pub(super) fn apply_template(
    f: &mut Form,
    template: crab_core::native::NativeTemplate,
    def: Option<&BlockDef>,
) {
    use crab_core::native::NativeTemplate;
    let Some(def) = def.filter(|_| template != NativeTemplate::Empty) else {
        f.native = native_skeleton(f);
        return;
    };
    let fill = crab_core::native::fill_template(template, def);
    for o in &fill.outputs {
        let mut row = PortRow::new(o.name);
        row.ty = o.ty;
        row.label = o.label.to_owned();
        f.outputs.push(row);
    }
    for p in &fill.params {
        let mut row = ParamRow::new(p.name);
        row.kind = p.kind;
        row.default = p.default.to_owned();
        row.label = p.label.to_owned();
        row.shared = p.shared;
        f.params.push(row);
    }
    f.native = fill.source;
}

/// Steps of the wizard for the native part. Returns `true` when the generated code (step 6 and
/// no mistakes) or the plain text editor (after "Править код") should be shown below.
pub(super) fn wizard_part(ui: &mut Ui, ed: &mut BlockEditor, pal: &Palette) -> bool {
    let editable = !ed.read_only;
    ui.horizontal(|ui| {
        ui.weak(crate::i18n::tr(
            "Код делается мастером из полей; руками его править не нужно.",
        ));
        if ui
            .add_enabled(editable, egui::Button::new(crate::i18n::tr("Править код")))
            .on_hover_text(crate::i18n::tr(
                "Перейти к тексту C++. Из текста вернуться в мастер нельзя",
            ))
            .clicked()
        {
            ed.form.wizard = None;
            ed.form.synced = None;
        }
    });
    let Some(w) = ed.form.wizard.as_mut() else {
        return true;
    };
    let mut changed = false;
    let step = &mut ed.wizard_step;
    ui.add_enabled_ui(editable, |ui| {
        changed = crate::nativewizard::wizard_ui(ui, w, step, pal);
    });
    if changed {
        sync_wizard(&mut ed.form);
    }
    if ed.wizard_step < 5 {
        return false;
    }
    let errors = ed
        .form
        .wizard
        .as_ref()
        .map(LibraryWizard::validate)
        .unwrap_or_default();
    for e in &errors {
        ui.colored_label(pal.error, e);
    }
    errors.is_empty()
}

/// Makes the wizard's parameters, inputs and outputs rows of the form and writes `native`.
/// Does nothing while the wizard has mistakes: the rows stay as of the last good state.
pub(super) fn sync_wizard(f: &mut Form) {
    let Some(new) = f.wizard.clone() else { return };
    if !new.validate().is_empty() {
        return;
    }
    let old = f.synced.take();
    let names = |w: Option<&LibraryWizard>| -> [Vec<String>; 3] {
        let Some(w) = w else {
            return Default::default();
        };
        let mut inputs: Vec<String> = Vec::new();
        for a in &w.actions {
            let n = a.input.trim().to_owned();
            if !inputs.contains(&n) {
                inputs.push(n);
            }
        }
        [
            w.params.iter().map(|p| p.name.trim().to_owned()).collect(),
            inputs,
            w.outputs.iter().map(|o| o.name.trim().to_owned()).collect(),
        ]
    };
    let [old_params, old_inputs, old_outputs] = names(old.as_ref());
    let [new_params, new_inputs, new_outputs] = names(Some(&new));
    let kind = |t: WizType| match t {
        WizType::Pin => ParamKind::Pin,
        WizType::Int => ParamKind::Int,
        WizType::Float => ParamKind::Float,
        WizType::Bool => ParamKind::Bool,
    };
    sync_rows(
        &mut f.params,
        &old_params,
        &new_params,
        |r| r.name.trim().to_owned(),
        |r, n| r.name = n.to_owned(),
        |r, i| r.kind = kind(new.params[i].ty),
        ParamRow::new,
    );
    sync_rows(
        &mut f.inputs,
        &old_inputs,
        &new_inputs,
        |r| r.name.trim().to_owned(),
        |r, n| r.name = n.to_owned(),
        |r, _| r.ty = SignalType::Bool,
        |n| {
            let mut r = PortRow::new(n);
            r.default = "false".to_owned();
            r
        },
    );
    sync_rows(
        &mut f.outputs,
        &old_outputs,
        &new_outputs,
        |r| r.name.trim().to_owned(),
        |r, n| r.name = n.to_owned(),
        |r, i| {
            r.ty = match new.outputs[i].ty {
                WizType::Bool => SignalType::Bool,
                WizType::Float => SignalType::Float,
                _ => SignalType::Int,
            }
        },
        PortRow::new,
    );
    f.native = new.generate();
    f.synced = Some(new);
}

/// Brings `rows` to the names in `new`: the row that had `old[i]` is renamed to `new[i]`
/// (keeping its caption and the rest), a missing one is made, one that is gone is removed.
pub(super) fn sync_rows<R>(
    rows: &mut Vec<R>,
    old: &[String],
    new: &[String],
    name: impl Fn(&R) -> String,
    set_name: impl Fn(&mut R, &str),
    update: impl Fn(&mut R, usize),
    make: impl Fn(&str) -> R,
) {
    for (i, n) in new.iter().enumerate() {
        let found = old
            .get(i)
            .and_then(|o| rows.iter().position(|r| name(r) == *o))
            .or_else(|| rows.iter().position(|r| name(r) == *n));
        let at = match found {
            Some(at) => at,
            None => {
                rows.push(make(n));
                rows.len() - 1
            }
        };
        set_name(&mut rows[at], n);
        update(&mut rows[at], i);
    }
    for o in old.iter().skip(new.len()) {
        if !new.contains(o) {
            rows.retain(|r| name(r) != *o);
        }
    }
}

/// Background build of the master with the native part of the block being edited.
#[derive(Default)]
pub(super) struct NativeBuild {
    pub(super) rx: Option<std::sync::mpsc::Receiver<Result<(), Vec<String>>>>,
    /// What to tell under the button; empty before the first build.
    pub(super) status: String,
    /// Errors of the last failed build.
    pub(super) errors: Vec<String>,
    pub(super) failed: bool,
}

impl NativeBuild {
    pub(super) fn running(&self) -> bool {
        self.rx.is_some()
    }

    pub(super) fn start(
        &mut self,
        block: crab_build::native::NativeBlock,
        board: crab_core::BoardConfig,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.rx = Some(rx);
        self.status = crate::i18n::tr("Мастер собирается, это займёт пару минут...").to_owned();
        self.errors.clear();
        self.failed = false;
        std::thread::spawn(move || {
            let result = (|| {
                let cli = crab_build::ArduinoCli::locate().map_err(|e| vec![e.to_string()])?;
                let work = std::env::temp_dir().join("crablogic_native_check");
                let features = crab_build::MasterFeatures {
                    natives: vec![block],
                    ..Default::default()
                };
                let mut errors = Vec::new();
                let result = cli.compile_master(&work, &board, &features, true, &mut |line| {
                    if let crab_build::OutputLine::Error(e) = crab_build::classify(line) {
                        errors.push(e);
                    }
                });
                result.map(|_| ()).map_err(|e| {
                    if errors.is_empty() {
                        errors.push(e.to_string());
                    }
                    errors
                })
            })();
            let _ = tx.send(result);
        });
    }

    pub(super) fn poll(&mut self) {
        let Some(rx) = &self.rx else { return };
        match rx.try_recv() {
            Ok(Ok(())) => {
                self.rx = None;
                self.status = crate::i18n::tr("Мастер с этим блоком собран без ошибок").to_owned();
            }
            Ok(Err(errors)) => {
                self.rx = None;
                self.failed = true;
                self.status = crate::i18n::tr("Мастер не собрался:").to_owned();
                self.errors = errors;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.rx = None;
                self.failed = true;
                self.status = crate::i18n::tr("Сборка прервалась").to_owned();
            }
        }
    }
}

/// Tab "Код C++ для платы": the native part of the block (`native.cpp`, `specs/drivers`).
pub(super) fn native_tab(
    ui: &mut Ui,
    ed: &mut BlockEditor,
    def: &Result<BlockDef, String>,
    board: &crab_core::BoardConfig,
    pal: &Palette,
) {
    let editable = !ed.read_only;
    let mut wizard = ed.form.wizard.is_some();
    ui.weak(crate::i18n::tr(
        "Код на C++ для датчика, шины или библиотеки Arduino. Он входит в мастер-прошивку платы и \
         выдаёт данные в схему; логику строят блоки схемы. Менять этот код = заново прошить мастер.",
    ));
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        let empty = ed.form.native.trim().is_empty();
        ui.add_enabled_ui(editable && empty && !wizard, |ui| {
            ui.menu_button(crate::i18n::tr("Создать по шаблону"), |ui| {
                // The wizard replaces the bare library skeleton.
                for t in crab_core::native::NativeTemplate::ALL
                    .into_iter()
                    .filter(|t| *t != crab_core::native::NativeTemplate::ArduinoLib)
                {
                    let usable = t == crab_core::native::NativeTemplate::Empty || def.is_ok();
                    let hint = if usable {
                        ""
                    } else {
                        "Сначала исправьте ошибки на других вкладках"
                    };
                    if ui
                        .add_enabled(usable, egui::Button::new(crate::i18n::tr(t.title())))
                        .on_disabled_hover_text(crate::i18n::tr(hint))
                        .clicked()
                    {
                        apply_template(&mut ed.form, t, def.as_ref().ok());
                        ui.close();
                    }
                }
                if ui
                    .button(crate::i18n::tr("Библиотека Arduino (мастер)"))
                    .on_hover_text(crate::i18n::tr(
                        "Пошагово: библиотека, объект, выходы, действия; код соберётся сам",
                    ))
                    .clicked()
                {
                    ed.form.wizard = Some(LibraryWizard::default());
                    ed.form.synced = None;
                    ed.wizard_step = 0;
                    ui.close();
                }
            });
        })
        .response
        .on_hover_text(crate::i18n::tr(
            "Готовый код для I²C, 1-Wire или библиотеки Arduino; недостающие параметры и выходы добавятся в блок",
        ));
        if editable && !wizard {
            ui.label(crate::i18n::tr("Вставить:"));
        }
    });
    if wizard {
        let show_code = wizard_part(ui, ed, pal);
        wizard = ed.form.wizard.is_some();
        if wizard && !show_code {
            return;
        }
    }
    let id = Id::unique("block_native");
    let mut insert = None;
    if editable && !wizard {
        ui.horizontal_wrapped(|ui| {
            let names = ed
                .form
                .inputs
                .iter()
                .map(|r| ("in", r.name.trim().to_owned()))
                .chain(
                    ed.form
                        .outputs
                        .iter()
                        .map(|r| ("out", r.name.trim().to_owned())),
                )
                .chain(
                    ed.form
                        .params
                        .iter()
                        .map(|p| ("p", p.name.trim().to_owned())),
                );
            for (group, name) in names {
                if name.is_empty() {
                    continue;
                }
                let text = format!("{group}.{name}");
                if ui.small_button(RichText::new(&text).monospace()).clicked() {
                    insert = Some(text);
                }
            }
        });
    }
    let mut layouter = crate::cpphl::layouter(true);
    let mut read_only_text = ed.form.native.as_str();
    if editable && !wizard {
        TextEdit::multiline(&mut ed.form.native)
            .id(id)
            .code_editor()
            .layouter(&mut layouter)
            .desired_width(f32::INFINITY)
            .desired_rows(22)
            .show(ui);
    } else {
        ui.add(
            TextEdit::multiline(&mut read_only_text)
                .code_editor()
                .layouter(&mut layouter)
                .desired_width(f32::INFINITY)
                .desired_rows(22),
        );
    }
    if let Some(text) = insert {
        insert_at_cursor(ui.ctx(), id, &mut ed.form.native, &text);
    }
    ed.native_build.poll();
    if ed.native_build.running() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(300));
    }
    if ed.form.native.trim().is_empty() {
        ui.weak(crate::i18n::tr(
            "Нет нативной части: блок работает только через шаблон «Код ESP32»",
        ));
        return;
    }
    let Ok(def) = def else {
        ui.weak(crate::i18n::tr(
            "(исправьте ошибки описания, тогда файл проверится)",
        ));
        return;
    };
    match crab_core::native::parse_native(&ed.form.native, def) {
        Err(errors) => {
            for e in errors {
                ui.colored_label(pal.error, e.to_string());
            }
        }
        Ok(part) => {
            let mut parts = Vec::new();
            if !part.libs.is_empty() {
                parts.push(crate::i18n::fmt(
                    "библиотеки: {0}",
                    &[&part.libs.join(", ")],
                ));
            }
            for (k, v) in &part.resources {
                parts.push(format!("{k} = {v}"));
            }
            let tail = if parts.is_empty() {
                String::new()
            } else {
                format!(" ({})", parts.join("; "))
            };
            ui.colored_label(
                pal.ok,
                format!("{}{tail}", crate::i18n::tr("Файл разобран, ошибок нет")),
            );
            ui.horizontal(|ui| {
                let core_ok = crab_build::cores::fqbn_available(&board.fqbn);
                let can = editable_build(ed) && core_ok;
                if ui
                    .add_enabled(
                        can,
                        egui::Button::new(crate::i18n::tr("Собрать мастер для проверки")),
                    )
                    .on_hover_text(crate::i18n::fmt(
                        "Собирает мастер-прошивку с этим блоком (один экземпляр) для платы {0}, без заливки",
                        &[&board.fqbn],
                    ))
                    .clicked()
                {
                    match crab_build::native::NativeBlock::from_parts(def, &ed.form.native, 1) {
                        Ok(block) => ed.native_build.start(block, board.clone()),
                        Err(e) => {
                            ed.native_build.status = e;
                            ed.native_build.failed = true;
                        }
                    }
                }
                if !core_ok {
                    ui.weak(crate::i18n::tr("Нет ядра ESP32 для этой платы: «Файл → Ядра ESP32»"));
                }
            });
            if !ed.native_build.status.is_empty() {
                let color = if ed.native_build.failed {
                    pal.error
                } else {
                    pal.weak
                };
                ui.colored_label(color, &ed.native_build.status);
                for line in ed.native_build.errors.iter().take(12) {
                    ui.label(RichText::new(line).monospace().color(pal.error));
                }
            }
        }
    }
}

/// A build can start unless one is running already.
pub(super) fn editable_build(ed: &BlockEditor) -> bool {
    !ed.native_build.running()
}
