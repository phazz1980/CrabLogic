//! Open project with undo/redo and the latest check report.

use std::path::Path;

use crab_core::project::is_web_kind;
use crab_core::{BlockPos, Layout, Library, Logic, Panel, ParamKind, Project, Report};

/// State that undo/redo restores.
#[derive(Clone)]
struct Snapshot {
    logic: Logic,
    layout: Layout,
    panel: Panel,
}

pub struct Doc {
    pub project: Project,
    /// Unsaved changes.
    pub dirty: bool,
    /// Bumped on every change of the project; background precompilation compares it.
    pub revision: u64,
    /// Bumped on every change of the electrical panel (and by undo and redo): the preview of the
    /// sheets is made again when it or `revision` changes. The firmware does not depend on it.
    pub panel_revision: u64,
    /// Check result for the current schema, refreshed after every edit.
    pub report: Report,
    /// The schema is frozen (debugging or simulation is running): edits change nothing.
    pub locked: bool,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// Key of the last edit: consecutive edits with the same key (dragging a block,
    /// typing a value) form one undo step.
    last_key: Option<String>,
}

const UNDO_LIMIT: usize = 200;

/// Whether the edit left everything a build depends on as it was: only the coordinates of
/// blocks and the routes of wires may differ.
fn same_build(
    before: &Snapshot,
    now: &Project,
    config_before: &crab_core::project::ProjectConfig,
) -> bool {
    before.logic == now.logic
        && *config_before == now.config
        && before.layout.sheets == now.layout.sheets
        && before.layout.blocks.len() == now.layout.blocks.len()
        && before
            .layout
            .blocks
            .iter()
            .zip(&now.layout.blocks)
            .all(|(a, b)| a.0 == b.0 && a.1.sheet == b.1.sheet)
}

impl Doc {
    pub fn open(path: &Path, lib: &Library) -> crab_core::Result<Self> {
        let mut project = Project::load(path)?;
        place_unplaced(&mut project, lib);
        let report = crab_codegen::check_project(&project, lib);
        Ok(Self {
            project,
            dirty: false,
            revision: 0,
            panel_revision: 0,
            report,
            locked: false,
            undo: Vec::new(),
            redo: Vec::new(),
            last_key: None,
        })
    }

    pub fn recheck(&mut self, lib: &Library) {
        self.report = crab_codegen::check_project(&self.project, lib);
    }

    /// Applies an edit as one undo step. `key` merges consecutive edits of the same
    /// thing; `None` always starts a new step. A failed edit leaves no trace.
    pub fn edit<T, E>(
        &mut self,
        lib: &Library,
        key: Option<String>,
        f: impl FnOnce(&mut Project) -> Result<T, E>,
    ) -> Result<T, E> {
        if self.locked {
            return f(&mut self.project.clone());
        }
        let before = self.snapshot();
        let config_before = self.project.config.clone();
        let merge = key.is_some() && key == self.last_key;
        match f(&mut self.project) {
            Ok(value) => {
                self.project.logic.sync_tags();
                let moved_only = same_build(&before, &self.project, &config_before);
                if !merge {
                    self.undo.push(before);
                    if self.undo.len() > UNDO_LIMIT {
                        self.undo.remove(0);
                    }
                }
                self.redo.clear();
                self.last_key = key;
                self.dirty = true;
                // Dragging a block only moves it on the sheet: nothing is rebuilt (the check,
                // the simulation and the background modules wait for a real change).
                if !moved_only {
                    self.revision += 1;
                    self.recheck(lib);
                }
                Ok(value)
            }
            Err(e) => {
                self.restore(before);
                Err(e)
            }
        }
    }

    /// Applies an edit of the electrical panel as one undo step (`key` merges consecutive edits
    /// of the same thing, as in [`Doc::edit`]). The panel does not change the firmware: nothing is
    /// rebuilt, the project only becomes unsaved. A panel can be edited while the schema is
    /// frozen (debugging, simulation): it is not part of what runs.
    pub fn edit_panel<T, E>(
        &mut self,
        key: Option<String>,
        f: impl FnOnce(&mut Panel) -> Result<T, E>,
    ) -> Result<T, E> {
        let before = self.snapshot();
        let merge = key.is_some() && key == self.last_key;
        match f(&mut self.project.panel) {
            Ok(value) => {
                if !merge {
                    self.undo.push(before);
                    if self.undo.len() > UNDO_LIMIT {
                        self.undo.remove(0);
                    }
                }
                self.redo.clear();
                self.last_key = key;
                self.dirty = true;
                self.panel_revision += 1;
                Ok(value)
            }
            Err(e) => {
                self.restore(before);
                Err(e)
            }
        }
    }

    /// An edit that only moves things on the sheets (block coordinates, wire bends) and keeps
    /// every block on its sheet: the fast path of dragging, run every frame. It copies nothing
    /// when it continues an edit of the same `key`, and it does not bump the revision or
    /// recheck: nothing a build depends on changes.
    pub fn edit_layout<T, E>(
        &mut self,
        key: String,
        f: impl FnOnce(&mut Project) -> Result<T, E>,
    ) -> Result<T, E> {
        if self.locked {
            return f(&mut self.project.clone());
        }
        let merge = self.last_key.as_deref() == Some(key.as_str());
        let before = (!merge).then(|| self.snapshot());
        let value = f(&mut self.project)?;
        if let Some(before) = before {
            self.undo.push(before);
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last_key = Some(key);
        self.dirty = true;
        Ok(value)
    }

    /// Takes the web page (the `web_*` blocks) from `other`, the project as another program
    /// (the web constructor) saved it, and keeps everything else as it is in the editor.
    pub fn adopt_web(&mut self, other: &Project, lib: &Library) {
        let _ = self.edit::<(), ()>(lib, None, |p| {
            p.logic.blocks.retain(|b| !is_web_kind(&b.kind));
            p.logic.blocks.extend(
                other
                    .logic
                    .blocks
                    .iter()
                    .filter(|b| is_web_kind(&b.kind))
                    .cloned(),
            );
            Ok(())
        });
    }

    /// Ends merging of consecutive edits (e.g. when a drag is released).
    pub fn end_edit(&mut self) {
        self.last_key = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.locked && !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.locked && !self.redo.is_empty()
    }

    pub fn undo(&mut self, lib: &Library) {
        if self.locked {
            return;
        }
        if let Some(s) = self.undo.pop() {
            self.redo.push(self.snapshot());
            self.restore(s);
            self.after_history(lib);
        }
    }

    pub fn redo(&mut self, lib: &Library) {
        if self.locked {
            return;
        }
        if let Some(s) = self.redo.pop() {
            self.undo.push(self.snapshot());
            self.restore(s);
            self.after_history(lib);
        }
    }

    /// Saves the project archive with the PDF of all sheets; returns a warning
    /// if the PDF could not be made.
    pub fn save(&mut self, lib: &Library) -> crab_core::Result<Option<String>> {
        let (panel_pdf, panel_warning) = self.panel_pdf(lib);
        let more: Vec<(&str, &[u8])> = panel_pdf
            .iter()
            .map(|p| (crab_core::PANEL_PDF, p.as_slice()))
            .collect();
        let warning = crab_pdf::save_project_with(&self.project, lib, &more)?;
        self.dirty = false;
        Ok(warning.or(panel_warning))
    }

    /// The PDF of the panel documentation for the archive (Russian, A3); none for an empty panel.
    /// The numbers of the wires are kept in the panel, so that they stay the same later.
    fn panel_pdf(&mut self, lib: &Library) -> (Option<Vec<u8>>, Option<String>) {
        use crab_schematic::{build_document_in, doc::typst_source, Format, Lang};
        if self.project.panel.devices.is_empty() {
            return (None, None);
        }
        let lang = Lang::Ru;
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            build_document_in(&self.project, lib, lang, Format::A3)
        }));
        match built {
            Ok(Ok((doc, nets))) => {
                if doc.sheets.is_empty() {
                    return (None, None);
                }
                if nets.wire_numbers() != self.project.panel.wire_numbers {
                    nets.store(&mut self.project.panel);
                }
                match crab_pdf::compile(typst_source(&doc, lang)) {
                    Ok(pdf) => (Some(pdf), None),
                    Err(e) => (None, Some(format!("panel.pdf: {e}"))),
                }
            }
            Ok(Err(e)) => (None, Some(format!("panel.pdf: {e}"))),
            Err(_) => (None, Some("panel.pdf: internal error".to_owned())),
        }
    }

    fn after_history(&mut self, lib: &Library) {
        self.last_key = None;
        self.dirty = true;
        self.revision += 1;
        self.panel_revision += 1;
        self.recheck(lib);
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            logic: self.project.logic.clone(),
            layout: self.project.layout.clone(),
            panel: self.project.panel.clone(),
        }
    }

    fn restore(&mut self, s: Snapshot) {
        self.project.logic = s.logic;
        self.project.layout = s.layout;
        self.project.panel = s.panel;
    }
}

/// Gives positions to blocks that have none (schemas written by hand or by AI):
/// a grid on sheet 1 below the placed blocks. Inputs and outputs (blocks with a pin)
/// stay in the project tree until they are dragged onto a sheet, like variables.
fn place_unplaced(project: &mut Project, lib: &Library) {
    let layout = &mut project.layout;
    let bottom = layout
        .blocks
        .values()
        .filter(|p| p.sheet == 1)
        .map(|p| p.y)
        .fold(0.0_f32, f32::max);
    let missing: Vec<String> = project
        .logic
        .blocks
        .iter()
        .filter(|b| !layout.blocks.contains_key(&b.id))
        // A web widget waits in the project tree; the page designer made it, not the sheet.
        .filter(|b| !crab_core::project::is_widget_kind(&b.kind))
        .filter(|b| {
            let tag = !crab_core::project::is_device_kind(&b.kind)
                && lib
                    .resolve(&project.logic, b)
                    .is_some_and(|r| r.def.params.iter().any(|p| p.kind == ParamKind::Pin));
            !tag
        })
        .map(|b| b.id.clone())
        .collect();
    for (i, id) in missing.into_iter().enumerate() {
        let (col, row) = (i % 5, i / 5);
        layout.blocks.insert(
            id,
            BlockPos {
                sheet: 1,
                x: 40.0 + 200.0 * col as f32,
                y: bottom + 160.0 + 140.0 * row as f32,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_and_outputs_stay_in_the_tree_until_dragged() {
        let (lib, _) =
            Library::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library")).unwrap();
        let mut project =
            Project::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/pump")).unwrap();
        let pins: Vec<String> = project
            .logic
            .blocks
            .iter()
            .filter(|b| {
                lib.resolve(&project.logic, b)
                    .is_some_and(|r| r.def.params.iter().any(|p| p.kind == ParamKind::Pin))
            })
            .map(|b| b.id.clone())
            .collect();
        assert!(!pins.is_empty());
        project.layout.blocks.clear();
        place_unplaced(&mut project, &lib);
        for b in &project.logic.blocks {
            assert_eq!(
                project.layout.blocks.contains_key(&b.id),
                !pins.contains(&b.id),
                "block {}",
                b.id
            );
        }
    }

    fn device(id: &str) -> crab_core::Device {
        toml::from_str(&format!(
            "id = \"{id}\"
kind = \"breaker\"
"
        ))
        .unwrap()
    }

    #[test]
    fn panel_edits_are_undone_and_do_not_rebuild_the_firmware() {
        let (lib, _) =
            Library::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library")).unwrap();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/pump");
        let mut doc = Doc::open(Path::new(path), &lib).unwrap();
        let (revision, panel) = (doc.revision, doc.project.panel.clone());
        assert!(!doc.dirty);
        doc.edit_panel::<(), ()>(None, |p| {
            p.devices.push(device("Q1"));
            Ok(())
        })
        .unwrap();
        assert!(doc.dirty);
        assert_eq!(doc.project.panel.devices.len(), panel.devices.len() + 1);
        // The firmware is not rebuilt, the preview of the sheets is.
        assert_eq!(doc.revision, revision);
        assert_eq!(doc.panel_revision, 1);
        assert!(doc.can_undo());
        doc.undo(&lib);
        assert_eq!(doc.project.panel, panel);
        doc.redo(&lib);
        assert_eq!(doc.project.panel.devices.len(), panel.devices.len() + 1);
        // An edit that fails leaves no trace.
        let before = doc.project.panel.clone();
        let failed = doc.edit_panel::<(), &str>(None, |p| {
            p.devices.clear();
            Err("no")
        });
        assert!(failed.is_err());
        assert_eq!(doc.project.panel, before);
    }

    #[test]
    fn saving_puts_the_panel_pdf_into_the_archive_only_for_a_panel_with_devices() {
        let (lib, _) =
            Library::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library")).unwrap();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/pump");
        let mut doc = Doc::open(Path::new(path), &lib).unwrap();
        let dir = std::env::temp_dir().join(format!("crab_panel_pdf_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        doc.project.path = dir.join("pump.crab");
        let names_in = |file: &Path| {
            let bytes = std::fs::read(file).unwrap();
            let has = |name: &str| bytes.windows(name.len()).any(|w| w == name.as_bytes());
            (has("project.toml"), has(crab_core::PANEL_PDF))
        };
        // No devices: nothing to document.
        doc.project.panel = crab_core::Panel::default();
        doc.save(&lib).unwrap();
        assert_eq!(names_in(&dir.join("pump.crab")), (true, false));
        // A panel with a feed, a breaker and a load. The PDF needs the `crablogic-pdf` helper; where
        // it is not built the save still succeeds and says why.
        doc.project.panel = toml::from_str(
            "supply = \"ac230\"\n[[device]]\nid = \"X1\"\nkind = \"feed\"\n\
             [[device]]\nid = \"Q1\"\nkind = \"breaker\"\nload = \"M1\"\n\
             [[device]]\nid = \"M1\"\nkind = \"motor\"\n",
        )
        .unwrap();
        let warning = doc.save(&lib).unwrap();
        assert!(!doc.dirty);
        let (_, has_pdf) = names_in(&dir.join("pump.crab"));
        assert!(has_pdf || warning.is_some(), "no panel.pdf and no warning");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_block_put_into_a_link_is_one_undo_step() {
        let (lib, _) =
            Library::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library")).unwrap();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/pump");
        let mut doc = Doc::open(Path::new(path), &lib).unwrap();
        let before = doc.project.logic.clone();
        let to: crab_core::PortRef = "delay.in".parse().unwrap();
        let pos = crab_core::BlockPos {
            sheet: 1,
            x: 100.0,
            y: 100.0,
        };
        let id = doc
            .edit(&lib, None, |p| {
                let id = p.add_block(&lib, "not", pos)?;
                p.insert_into_link(&lib, &id, &to)?;
                Ok::<_, crab_core::EditError>(id)
            })
            .unwrap();
        assert!(doc
            .project
            .logic
            .links
            .iter()
            .any(|l| l.from.block == id && l.to == to));
        doc.undo(&lib);
        assert_eq!(doc.project.logic, before);
    }

    #[test]
    fn consecutive_panel_edits_of_one_thing_are_one_undo_step() {
        let (lib, _) =
            Library::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library")).unwrap();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/pump");
        let mut doc = Doc::open(Path::new(path), &lib).unwrap();
        // The example has a panel of its own: start from an empty one.
        doc.project.panel = crab_core::Panel::default();
        doc.edit_panel::<(), ()>(None, |p| {
            p.devices.push(device("Q1"));
            Ok(())
        })
        .unwrap();
        for text in ["C", "C1", "C16"] {
            doc.edit_panel::<(), ()>(Some("rating:Q1".into()), |p| {
                p.devices[0].rating = text.into();
                Ok(())
            })
            .unwrap();
        }
        assert_eq!(doc.project.panel.devices[0].rating, "C16");
        doc.undo(&lib);
        assert_eq!(doc.project.panel.devices[0].rating, "");
        doc.undo(&lib);
        assert!(doc.project.panel.devices.is_empty());
    }

    #[test]
    fn a_locked_schema_ignores_edits_and_history() {
        let (lib, _) =
            Library::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library")).unwrap();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/pump");
        let mut doc = Doc::open(Path::new(path), &lib).unwrap();
        let _ = doc.edit::<(), ()>(&lib, None, |p| {
            p.logic.blocks.pop();
            Ok(())
        });
        assert!(doc.can_undo());
        doc.locked = true;
        assert!(!doc.can_undo());
        let before = doc.project.logic.clone();
        let layout = doc.project.layout.clone();
        let _ = doc.edit::<(), ()>(&lib, None, |p| {
            p.logic.blocks.pop();
            Ok(())
        });
        let _ = doc.edit_layout::<(), ()>("move:x".into(), |p| {
            p.layout.blocks.clear();
            Ok(())
        });
        doc.undo(&lib);
        assert_eq!(doc.project.logic, before);
        assert_eq!(doc.project.layout, layout);
        doc.locked = false;
        doc.undo(&lib);
        assert_ne!(doc.project.logic, before);
    }
}
