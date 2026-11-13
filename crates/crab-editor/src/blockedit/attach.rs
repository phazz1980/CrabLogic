use super::*;

/// Files that live next to `block.toml`: `example.crab` and the pictures of the docs.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct Attach {
    pub(super) example: Example,
    /// Pictures of the docs.
    pub(super) pictures: Pictures,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(super) enum Example {
    #[default]
    None,
    /// Already in the block folder.
    Saved,
    /// Project file to copy in on save.
    New(PathBuf),
}

impl Attach {
    pub(super) fn from_dir(dir: &Path) -> Self {
        let example = if dir.join(crab_core::blockfile::EXAMPLE_FILE).is_file() {
            Example::Saved
        } else {
            Example::None
        };
        Self {
            example,
            pictures: Pictures::in_dir(dir),
        }
    }

    /// The same attachments for a copy of the block: everything is copied from `dir`.
    pub(super) fn copied_from(dir: &Path) -> Self {
        let mut a = Self::from_dir(dir);
        if a.example == Example::Saved {
            a.example = Example::New(dir.join(crab_core::blockfile::EXAMPLE_FILE));
        }
        a.pictures = Pictures::copied_from(dir);
        a
    }

    /// Makes the block folder `dir` hold exactly these attachments and the pictures `docs` uses.
    pub(super) fn apply(&mut self, dir: &Path, docs: &str) -> Result<(), String> {
        let err = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
        let example = dir.join(crab_core::blockfile::EXAMPLE_FILE);
        match &self.example {
            Example::None => {
                let _ = fs::remove_file(&example);
            }
            Example::Saved => {}
            Example::New(src) => {
                fs::copy(src, &example).map_err(|e| err(src, e))?;
                self.example = Example::Saved;
            }
        }
        self.pictures.apply(dir, docs)
    }
}

pub(super) fn files_tab(ui: &mut Ui, f: &mut Form, module: &ModuleInfo) {
    let attach = &mut f.attach;
    ui.label(RichText::new(crate::i18n::tr("Модуль WASM")).strong());
    match module.bytes {
        Some(n) => ui.label(crate::i18n::fmt("block.wasm: {0} байт", &[&n])),
        None => ui.weak(crate::i18n::tr(
            "block.wasm нет: логика блока — модуль WASM рядом с block.toml (инструкция — папка skills)",
        )),
    };
    ui.weak(crate::i18n::tr(
        "Модуль и пример лежат в папке блока и едут вместе с ним в наборе .crabblocks и в проекте.",
    ));
    ui.add_space(10.0);
    ui.label(RichText::new(crate::i18n::tr("Пример использования")).strong());
    match &attach.example {
        Example::None => {
            ui.weak(crate::i18n::tr("Пример не прикреплён"));
        }
        Example::Saved => {
            ui.label("example.crab");
        }
        Example::New(p) => {
            ui.label(crate::i18n::fmt(
                "{0} (будет скопирован при сохранении)",
                &[&p.display()],
            ));
        }
    }
    ui.horizontal(|ui| {
        if ui.button(crate::i18n::tr("Прикрепить пример...")).clicked() {
            if let Some(file) = rfd::FileDialog::new()
                .set_title(crate::i18n::tr("Проект-пример"))
                .add_filter(
                    crate::i18n::tr("Проект CrabLogic"),
                    &[
                        crab_core::project::PROJECT_EXT,
                        crab_core::project::LEGACY_PROJECT_EXT,
                    ],
                )
                .pick_file()
            {
                attach.example = Example::New(file);
            }
        }
        if ui
            .add_enabled(
                attach.example != Example::None,
                egui::Button::new(crate::i18n::tr("Убрать")),
            )
            .clicked()
        {
            attach.example = Example::None;
        }
    });
}
