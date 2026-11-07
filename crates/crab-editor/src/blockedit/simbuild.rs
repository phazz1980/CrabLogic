use super::*;

/// Trial generation for one state of the form.
pub(super) struct Generated {
    pub(super) def: BlockDef,
    pub(super) template: String,
    pub(super) result: Result<String, String>,
}

/// Background build of the simulation module made from the block code.
#[derive(Default)]
pub(super) struct SimBuild {
    pub(super) rx: Option<std::sync::mpsc::Receiver<Result<crab_build::wasm::ModuleBuild, String>>>,
    /// What to tell on the trial-run tab; empty when there is nothing to tell.
    pub(super) status: String,
}

impl SimBuild {
    /// Starts the build after a save; nothing happens while the option is off or zig is
    /// not found.
    pub(super) fn start(&mut self, dir: &Path, def: &BlockDef, template: &str) {
        self.rx = None;
        self.status.clear();
        let Some(zig) = crate::theme::sim_compiler() else {
            return;
        };
        match crab_codegen::sim_source(def, template) {
            Err(e) => self.status = e.to_string(),
            Ok(source) => {
                let (tx, rx) = std::sync::mpsc::channel();
                let dir = dir.to_owned();
                std::thread::spawn(move || {
                    let _ = tx.send(crab_build::wasm::build_module(&zig, &dir, &source));
                });
                self.rx = Some(rx);
                self.status = crate::i18n::tr("Симуляция собирается...").to_owned();
            }
        }
    }

    /// Takes the result of a finished build; true while a build is running.
    pub(super) fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        match rx.try_recv() {
            Ok(result) => {
                self.rx = None;
                self.status = match result {
                    Ok(crab_build::wasm::ModuleBuild::Built) => {
                        crate::i18n::tr("Симуляция собрана из кода блока").to_owned()
                    }
                    Ok(crab_build::wasm::ModuleBuild::KeptManual) => {
                        crate::i18n::tr("Есть свой block.wasm без метки — он не заменён").to_owned()
                    }
                    Err(e) => crate::i18n::fmt("Симуляция не собрана: {0}", &[&e]),
                };
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.rx = None;
                false
            }
        }
    }
}

/// Runs the block's `block.wasm` with hand-typed inputs and parameters.
pub(super) fn trial_tab(ui: &mut Ui, ed: &mut BlockEditor, dir: Option<&Path>, pal: &Palette) {
    let start = |t: &str| parse_float(t.trim()).unwrap_or(0.0);
    let shape = crate::trialrun::Shape {
        inputs: ed
            .form
            .inputs
            .iter()
            .map(|p| (p.name.as_str(), start(&p.default)))
            .collect(),
        params: ed
            .form
            .params
            .iter()
            .map(|p| (p.name.as_str(), start(&p.default)))
            .collect(),
        outputs: ed.form.outputs.iter().map(|p| p.name.as_str()).collect(),
    };
    if ed.sim_build.poll() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(200));
    }
    if !ed.sim_build.status.is_empty() {
        ui.weak(&ed.sim_build.status);
    }
    crate::trialrun::show(ui, &mut ed.trial, dir, &shape, pal);
}

pub(super) fn sketch_tab(ui: &mut Ui, ed: &BlockEditor, pal: &Palette) {
    match &ed.generated {
        None => {
            ui.weak(crate::i18n::tr(
                "Исправьте ошибки описания блока — тогда здесь появится код.",
            ));
        }
        Some(g) => match &g.result {
            Ok(code) => {
                ui.weak(crate::i18n::fmt(
                    "Скетч с одним блоком «{0}»: неподключённые входы — нули, обязательные \
                     параметры — пробные значения.",
                    &[&crab_codegen::preview_id(&g.def)],
                ));
                let mut text = code.as_str();
                let mut layouter = crate::cpphl::layouter(false);
                ui.add(
                    TextEdit::multiline(&mut text)
                        .code_editor()
                        .layouter(&mut layouter)
                        .desired_width(f32::INFINITY),
                );
            }
            Err(e) => {
                ui.colored_label(pal.error, e);
            }
        },
    }
}
