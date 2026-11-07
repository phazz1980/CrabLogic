use super::*;

/// Files that live next to `block.toml`: `lib/<Name>/...` and `example.crab`.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct Attach {
    pub(super) libs: Vec<AttachedLib>,
    pub(super) example: Example,
    /// Pictures of the docs.
    pub(super) pictures: Pictures,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct AttachedLib {
    pub(super) name: String,
    /// Folder to copy in on save; `None` - already in the block folder.
    pub(super) src: Option<PathBuf>,
    /// Header put into the block's includes when the library was attached.
    pub(super) header: Option<String>,
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
        let mut libs: Vec<AttachedLib> = fs::read_dir(dir.join(crab_core::blockfile::LIB_DIR))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| AttachedLib {
                name: e.file_name().to_string_lossy().into_owned(),
                src: None,
                header: None,
            })
            .collect();
        libs.sort_by(|a, b| a.name.cmp(&b.name));
        let example = if dir.join(crab_core::blockfile::EXAMPLE_FILE).is_file() {
            Example::Saved
        } else {
            Example::None
        };
        Self {
            libs,
            example,
            pictures: Pictures::in_dir(dir),
        }
    }

    /// The same attachments for a copy of the block: everything is copied from `dir`.
    pub(super) fn copied_from(dir: &Path) -> Self {
        let mut a = Self::from_dir(dir);
        for lib in &mut a.libs {
            lib.src = Some(dir.join(crab_core::blockfile::LIB_DIR).join(&lib.name));
        }
        if a.example == Example::Saved {
            a.example = Example::New(dir.join(crab_core::blockfile::EXAMPLE_FILE));
        }
        a.pictures = Pictures::copied_from(dir);
        a
    }

    /// Makes the block folder `dir` hold exactly these attachments and the pictures `docs` uses.
    pub(super) fn apply(&mut self, dir: &Path, docs: &str) -> Result<(), String> {
        let lib_root = dir.join(crab_core::blockfile::LIB_DIR);
        let err = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
        if let Ok(entries) = fs::read_dir(&lib_root) {
            for e in entries.flatten() {
                if !self.libs.iter().any(|l| e.file_name() == *l.name) {
                    let _ = fs::remove_dir_all(e.path());
                }
            }
        }
        for lib in &mut self.libs {
            let Some(src) = lib.src.take() else { continue };
            let dest = lib_root.join(&lib.name);
            let _ = fs::remove_dir_all(&dest);
            copy_dir(&src, &dest).map_err(|e| err(&src, e))?;
        }
        if self.libs.is_empty() {
            let _ = fs::remove_dir(&lib_root);
        }
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

pub(super) fn copy_dir(src: &Path, dest: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dest)?;
    for e in fs::read_dir(src)? {
        let e = e?;
        let to = dest.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &to)?;
        } else {
            fs::copy(e.path(), to)?;
        }
    }
    Ok(())
}

/// Main header of an Arduino library folder: `<name>.h` in `src/` (or the folder
/// itself for old layouts), else the only header there.
pub(super) fn library_header(dir: &Path, name: &str) -> Option<String> {
    [dir.join("src"), dir.to_path_buf()]
        .into_iter()
        .find_map(|d| {
            let mut headers: Vec<String> = fs::read_dir(d)
                .ok()?
                .flatten()
                .filter(|e| e.path().is_file())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|f| f.to_lowercase().ends_with(".h"))
                .collect();
            headers.sort();
            let named = headers
                .iter()
                .find(|h| h[..h.len() - 2].eq_ignore_ascii_case(name));
            match (named, headers.len()) {
                (Some(h), _) => Some(h.clone()),
                (None, 1) => headers.pop(),
                _ => None,
            }
        })
}

/// `includes` (comma-separated) with `header` added once.
pub(super) fn with_include(includes: &str, header: &str) -> String {
    let mut all = list(includes);
    if !all.iter().any(|h| h == header) {
        all.push(header.to_owned());
    }
    all.join(", ")
}

/// `includes` (comma-separated) without `header`.
pub(super) fn without_include(includes: &str, header: &str) -> String {
    let all: Vec<String> = list(includes).into_iter().filter(|h| h != header).collect();
    all.join(", ")
}

pub(super) fn files_tab(ui: &mut Ui, f: &mut Form) {
    let attach = &mut f.attach;
    ui.weak(crate::i18n::tr(
        "Библиотека Arduino и пример лежат в папке блока и едут вместе с ним \
         в наборе .crabblocks и в проекте. Сборка подхватывает библиотеки сама, \
         а главный заголовок прикреплённой библиотеки попадает в поле «Заголовки» \
         на вкладке «Код ESP32».",
    ));
    ui.add_space(6.0);
    ui.label(RichText::new(crate::i18n::tr("Библиотеки")).strong());
    let mut remove = None;
    for (i, lib) in attach.libs.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(&lib.name);
            if lib.src.is_some() {
                ui.weak(crate::i18n::tr("(будет скопирована при сохранении)"));
            }
            if ui.small_button(crate::i18n::tr("Убрать")).clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        let lib = attach.libs.remove(i);
        if let Some(header) = lib.header {
            f.includes = without_include(&f.includes, &header);
        }
    }
    if attach.libs.is_empty() {
        ui.weak(crate::i18n::tr("Нет прикреплённых библиотек"));
    }
    if ui
        .button(crate::i18n::tr("Прикрепить библиотеку..."))
        .on_hover_text(crate::i18n::tr(
            "Папка библиотеки Arduino (с library.properties или src/)",
        ))
        .clicked()
    {
        if let Some(dir) = rfd::FileDialog::new()
            .set_title(crate::i18n::tr("Папка библиотеки Arduino"))
            .pick_folder()
        {
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !name.is_empty() && !attach.libs.iter().any(|l| l.name == name) {
                let header = library_header(&dir, &name);
                if let Some(header) = &header {
                    f.includes = with_include(&f.includes, header);
                }
                attach.libs.push(AttachedLib {
                    name,
                    src: Some(dir),
                    header,
                });
            }
        }
    }
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
