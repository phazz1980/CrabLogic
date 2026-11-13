use super::*;

/// What the block folder holds for the module: `block.wasm` size, if there is one.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct ModuleInfo {
    pub(super) bytes: Option<u64>,
}

impl ModuleInfo {
    pub(super) fn of_dir(dir: Option<&Path>) -> Self {
        let bytes = dir
            .and_then(|d| fs::metadata(d.join(crab_core::library::WASM_FILE)).ok())
            .map(|m| m.len());
        Self { bytes }
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
    crate::trialrun::show(ui, &mut ed.trial, dir, &shape, pal);
}
