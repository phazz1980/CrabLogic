//! User blocks inside a project file: the archive carries a copy of every user block
//! the schema uses (`user_blocks/<group>/<type>/<file>`), so a project opens on another
//! computer without copying `user_library/` by hand.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::Path;

use zip::ZipArchive;

use crate::library::Library;
use crate::logic::Logic;

/// File name and contents of a packed block file.
type PackedFile = (String, Vec<u8>);

/// Archive folder with the packed user blocks.
pub const PACKED_DIR: &str = "user_blocks";

/// Archive entries (name, contents) for the user blocks that `logic` uses.
pub fn pack_user_blocks(logic: &Logic, lib: &Library) -> Vec<(String, Vec<u8>)> {
    let mut kinds: Vec<&str> = logic.blocks.iter().map(|b| b.kind.as_str()).collect();
    kinds.sort_unstable();
    kinds.dedup();
    let mut out = Vec::new();
    for kind in kinds.into_iter().filter(|k| lib.is_user(k)) {
        let Some(block) = lib.get(kind) else { continue };
        let group = block.def.info.group.trim_matches('/');
        let prefix = if group.is_empty() {
            format!("{PACKED_DIR}/{kind}")
        } else {
            format!("{PACKED_DIR}/{group}/{kind}")
        };
        // Attached libraries and the example travel with the block.
        let Ok(files) = crate::blockfile::read_files(&block.dir) else {
            continue;
        };
        for (name, data) in files {
            out.push((format!("{prefix}/{name}"), data));
        }
        // The pack version and the revisions of the block, to tell later which copy is
        // newer or whether the two were changed independently.
        crate::pack::record(&block.dir);
        if let Some((dir, info)) = crate::pack::find(&block.dir) {
            let stamp = PackStamp {
                pack: dir
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                version: info.version,
                history: info.history.get(kind).cloned().unwrap_or_default(),
            };
            if let Ok(text) = toml::to_string(&stamp) {
                out.push((format!("{PACKED_PACKS}/{kind}.toml"), text.into_bytes()));
            }
        }
    }
    out
}

/// Archive folder with the pack versions of the packed user blocks (`<type>.toml`).
pub const PACKED_PACKS: &str = "user_block_packs";

/// Pack folder and version a block had when the project was saved.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct PackStamp {
    pack: String,
    version: u32,
    /// Revisions of the block up to the saved one (oldest first); empty in projects
    /// saved before the history existed.
    #[serde(default)]
    history: Vec<String>,
}

/// Pack stamps of the blocks packed into the project file `project`: type → stamp.
fn read_pack_stamps(project: &Path) -> BTreeMap<String, PackStamp> {
    let mut stamps = BTreeMap::new();
    let Ok(file) = fs::File::open(project) else {
        return stamps;
    };
    let Ok(mut zip) = ZipArchive::new(std::io::BufReader::new(file)) else {
        return stamps;
    };
    for i in 0..zip.len() {
        let Ok(mut entry) = zip.by_index(i) else {
            continue;
        };
        let name = entry.name().replace('\\', "/");
        let Some(kind) = name
            .strip_prefix(&format!("{PACKED_PACKS}/"))
            .and_then(|r| r.strip_suffix(".toml"))
            .filter(|k| safe_part(k))
        else {
            continue;
        };
        let mut text = String::new();
        if entry.read_to_string(&mut text).is_ok() {
            if let Ok(stamp) = toml::from_str::<PackStamp>(&text) {
                stamps.insert(kind.to_owned(), stamp);
            }
        }
    }
    stamps
}

/// Archive folder with the user board of the project.
pub const PACKED_BOARDS: &str = "user_boards";

/// Archive entries (name, contents) for the user board the project is made for: its
/// `board.toml`, text and pictures. Empty for a built-in board or no board.
pub fn pack_user_board(model: Option<&str>, lib: &Library) -> Vec<(String, Vec<u8>)> {
    let Some(board) = model.and_then(|id| lib.boards().get(id)) else {
        return Vec::new();
    };
    let Ok(files) = crate::blockfile::read_files(&board.dir) else {
        return Vec::new();
    };
    let id = &board.def.info.id;
    files
        .into_iter()
        .map(|(name, data)| (format!("{PACKED_BOARDS}/{id}/{name}"), data))
        .collect()
}

/// Reads the packed boards of the project file `project`: id → files. A project folder
/// (the old format, used by `examples/`) keeps them in `user_boards/<id>/` as they are.
fn read_packed_boards(project: &Path) -> BTreeMap<String, Vec<PackedFile>> {
    let mut boards = if project.is_dir() {
        read_folder_boards(project)
    } else {
        read_archive_boards(project)
    };
    boards.retain(|id, files| {
        crate::boards::valid_id(id) && files.iter().any(|(n, _)| n == crate::boards::BOARD_FILE)
    });
    boards
}

fn read_folder_boards(project: &Path) -> BTreeMap<String, Vec<PackedFile>> {
    let mut boards = BTreeMap::new();
    let entries = fs::read_dir(project.join(PACKED_BOARDS));
    for entry in entries.into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Ok(files) = crate::blockfile::read_files(&entry.path()) {
            boards.insert(name, files);
        }
    }
    boards
}

fn read_archive_boards(project: &Path) -> BTreeMap<String, Vec<PackedFile>> {
    let mut boards: BTreeMap<String, Vec<PackedFile>> = BTreeMap::new();
    let Ok(file) = fs::File::open(project) else {
        return boards;
    };
    let Ok(mut zip) = ZipArchive::new(std::io::BufReader::new(file)) else {
        return boards;
    };
    for i in 0..zip.len() {
        let Ok(mut entry) = zip.by_index(i) else {
            continue;
        };
        let name = entry.name().replace('\\', "/");
        let Some(rest) = name.strip_prefix(&format!("{PACKED_BOARDS}/")) else {
            continue;
        };
        let parts: Vec<&str> = rest.split('/').collect();
        if entry.is_dir() || parts.len() < 2 || !parts.iter().all(|p| safe_part(p)) {
            continue;
        }
        let mut data = Vec::new();
        if entry.read_to_end(&mut data).is_ok() {
            boards
                .entry(parts[0].to_owned())
                .or_default()
                .push((parts[1..].join("/"), data));
        }
    }
    boards
}

/// Restores the user board packed into the project file `project` into
/// `<user_dir>/boards/` if `lib` does not know it. A folder that already exists is not
/// touched, even a broken one, and a packed board that does not pass the check is
/// not written. Returns the ids written; the caller reloads the library when it is not
/// empty.
pub fn unpack_missing_boards(project: &Path, user_dir: &Path, lib: &Library) -> Vec<String> {
    let boards_dir = crate::boards::boards_dir(user_dir);
    let mut written = Vec::new();
    for (id, files) in read_packed_boards(project) {
        let dir = boards_dir.join(&id);
        if lib.boards().get(&id).is_some() || dir.exists() {
            continue;
        }
        let ok = crate::blockfile::write_files(&dir, &files).is_ok()
            && crate::boards::UserBoard::load(&dir).is_ok();
        if ok {
            written.push(id);
        } else {
            let _ = fs::remove_dir_all(&dir);
        }
    }
    written
}

/// Packed copies of user blocks: (group folder, type) → files.
type PackedBlocks = BTreeMap<(String, String), Vec<PackedFile>>;

/// Reads every packed block of the project file `project`.
fn read_packed(project: &Path) -> PackedBlocks {
    let mut blocks = PackedBlocks::new();
    let Ok(file) = fs::File::open(project) else {
        return blocks;
    };
    let Ok(mut zip) = ZipArchive::new(std::io::BufReader::new(file)) else {
        return blocks;
    };
    let mut entries: Vec<(Vec<String>, Vec<u8>)> = Vec::new();
    for i in 0..zip.len() {
        let Ok(mut entry) = zip.by_index(i) else {
            continue;
        };
        let name = entry.name().replace('\\', "/");
        let Some(rest) = name.strip_prefix(&format!("{PACKED_DIR}/")) else {
            continue;
        };
        let parts: Vec<String> = rest.split('/').map(str::to_owned).collect();
        if entry.is_dir() || parts.len() < 2 || !parts.iter().all(|p| safe_part(p)) {
            continue;
        }
        let mut data = Vec::new();
        if entry.read_to_end(&mut data).is_ok() {
            entries.push((parts, data));
        }
    }
    // A block folder is where its `block.toml` is; deeper files (`lib/...`) belong to it.
    let dirs: Vec<Vec<String>> = entries
        .iter()
        .filter(|(p, _)| p.last().is_some_and(|n| n == "block.toml") && p.len() >= 2)
        .map(|(p, _)| p[..p.len() - 1].to_vec())
        .collect();
    for (parts, data) in entries {
        let Some(dir) = dirs
            .iter()
            .filter(|d| parts.len() > d.len() && parts.starts_with(d))
            .max_by_key(|d| d.len())
        else {
            continue;
        };
        let (kind, group) = dir.split_last().expect("block dir is not empty");
        blocks
            .entry((group.join("/"), kind.clone()))
            .or_default()
            .push((parts[dir.len()..].join("/"), data));
    }
    blocks
}

/// Writes a packed block into `user_dir`, replacing the files of an older copy.
fn write_block(user_dir: &Path, group: &str, kind: &str, files: &[PackedFile]) -> bool {
    let dir = user_dir.join(group).join(kind);
    let _ = fs::remove_dir_all(&dir);
    crate::blockfile::write_files(&dir, files).is_ok()
}

/// Archive entries for every block folder under `dir` (an example's own `library/`, e.g.
/// `examples/wasm_driver/library/bmp280`): packed as user blocks, so opening the example
/// installs them like the blocks of any project file.
pub fn pack_block_folders(dir: &Path) -> Vec<PackedFile> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut folders: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join(crate::library::BLOCK_FILE).is_file())
        .collect();
    folders.sort();
    let mut out = Vec::new();
    for folder in folders {
        let Ok(block) = crate::library::LibraryBlock::load(&folder) else {
            continue;
        };
        let kind = &block.def.info.kind;
        let group = block.def.info.group.trim_matches('/');
        let prefix = if group.is_empty() {
            format!("{PACKED_DIR}/{kind}")
        } else {
            format!("{PACKED_DIR}/{group}/{kind}")
        };
        if let Ok(files) = crate::blockfile::read_files(&folder) {
            out.extend(files.into_iter().map(|(n, d)| (format!("{prefix}/{n}"), d)));
        }
    }
    out
}

/// Restores the packed user blocks of the project file `project` that are missing in
/// `lib` into `user_dir` (the `user_library/` folder). Blocks that already exist stay as
/// they are. Returns the types written; the caller reloads the library when it is not empty.
pub fn unpack_missing_blocks(project: &Path, user_dir: &Path, lib: &Library) -> Vec<String> {
    read_packed(project)
        .into_iter()
        .filter(|((_, kind), _)| lib.get(kind).is_none())
        .filter(|((group, kind), files)| write_block(user_dir, group, kind, files))
        .map(|((_, kind), _)| kind)
        .collect()
}

/// A user block whose copy in the project differs from the one in the program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionDiff {
    pub kind: String,
    /// Which copy is newer.
    pub newer: Newer,
}

/// The relation of the two copies of a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Newer {
    /// The copy in the project file descends from the other one.
    Project,
    /// The copy in the program's user library descends from the other one.
    Program,
    /// Both were changed independently of each other (neither is in the other's history).
    Parallel,
    /// Cannot be told: no history, equal versions and times.
    Unknown,
}

/// Latest modification time of the files under `dir`.
fn latest_modified(dir: &Path) -> Option<std::time::SystemTime> {
    let mut latest = None;
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        let time = if path.is_dir() {
            latest_modified(&path)
        } else {
            entry.metadata().and_then(|m| m.modified()).ok()
        };
        latest = latest.max(time);
    }
    latest
}

/// How the two copies relate. First by the revision histories: a copy found in the
/// other one's history is the older; when neither is, the copies are parallel. Without
/// a history (a project saved by an older program, a block outside any pack): by the
/// pack version recorded in the project and the pack's version in the library, and
/// with equal versions by file times (the project file is written when it is saved,
/// the program's block when it is edited).
fn newer_copy(
    project: &Path,
    block_dir: &Path,
    stamp: Option<&PackStamp>,
    files: &[PackedFile],
) -> Newer {
    if let (Some(stamp), Some(mut library)) = (stamp, crate::pack::history(block_dir)) {
        if !stamp.history.is_empty() {
            let theirs = crate::pack::hash_files(files);
            let ours = crate::pack::content_hash(block_dir).unwrap_or_default();
            // A copy edited by hand after the last record counts as the newest revision.
            crate::pack::push_revision(&mut library, &ours);
            let mut packed = stamp.history.clone();
            crate::pack::push_revision(&mut packed, &theirs);
            if library.contains(&theirs) {
                return Newer::Program;
            }
            if packed.contains(&ours) {
                return Newer::Project;
            }
            return Newer::Parallel;
        }
    }
    if let (Some(stamp), Some((dir, info))) = (stamp, crate::pack::find(block_dir)) {
        let same_pack = dir
            .file_name()
            .is_some_and(|n| n.to_string_lossy() == stamp.pack);
        match info.version.cmp(&stamp.version) {
            _ if !same_pack => {}
            std::cmp::Ordering::Greater => return Newer::Program,
            std::cmp::Ordering::Less => return Newer::Project,
            std::cmp::Ordering::Equal => {}
        }
    }
    let saved = fs::metadata(project).and_then(|m| m.modified()).ok();
    match (latest_modified(block_dir), saved) {
        (Some(block), Some(saved)) if block > saved => Newer::Program,
        (Some(block), Some(saved)) if block < saved => Newer::Project,
        _ => Newer::Unknown,
    }
}

/// Whether the packed files equal the files in `dir` (line endings are ignored).
fn same_files(files: &[PackedFile], dir: &Path) -> bool {
    let norm = |d: &[u8]| {
        d.iter()
            .copied()
            .filter(|b| *b != b'\r')
            .collect::<Vec<u8>>()
    };
    let Ok(read) = crate::blockfile::read_files(dir) else {
        return false;
    };
    let local: BTreeMap<String, Vec<u8>> = read.into_iter().map(|(n, d)| (n, norm(&d))).collect();
    let packed: BTreeMap<String, Vec<u8>> =
        files.iter().map(|(n, d)| (n.clone(), norm(d))).collect();
    local == packed
}

/// User blocks that exist both in the project file and in `lib` with different
/// contents, with how the copies relate (see [`Newer`]).
pub fn version_diffs(project: &Path, lib: &Library) -> Vec<VersionDiff> {
    let stamps = read_pack_stamps(project);
    read_packed(project)
        .into_iter()
        .filter_map(|((_, kind), files)| {
            let local = lib.get(&kind).filter(|_| lib.is_user(&kind))?;
            (!same_files(&files, &local.dir)).then(|| VersionDiff {
                newer: newer_copy(project, &local.dir, stamps.get(&kind), &files),
                kind,
            })
        })
        .collect()
}

/// User blocks packed in the project file `project`, to offer them for import.
pub fn project_blocks(project: &Path) -> Vec<crate::blockfile::Incoming> {
    read_packed(project)
        .into_values()
        .filter_map(|files| crate::blockfile::Incoming::from_files(files).ok())
        .collect()
}

/// Replaces the program's user blocks `kinds` with the copies packed in the project
/// (each at its place in `user_dir`). Returns the types written.
pub fn adopt_project_blocks(project: &Path, user_dir: &Path, kinds: &[String]) -> Vec<String> {
    read_packed(project)
        .into_iter()
        .filter(|((_, kind), _)| kinds.contains(kind))
        .filter(|((group, kind), files)| write_block(user_dir, group, kind, files))
        .map(|((_, kind), _)| kind)
        .collect()
}

/// Puts the project's copy of the block `kind` into `user_dir` as a new block of type
/// `new_kind` (the name gets a mark), so that the two versions can be compared and merged
/// by hand. The caller picks a free `new_kind`. Returns whether it was written.
pub fn save_project_copy_aside(
    project: &Path,
    user_dir: &Path,
    kind: &str,
    new_kind: &str,
) -> bool {
    if !safe_part(new_kind) {
        return false;
    }
    let Some(((group, _), mut files)) = read_packed(project)
        .into_iter()
        .find(|((_, k), _)| k == kind)
    else {
        return false;
    };
    let Some(entry) = files.iter_mut().find(|(n, _)| n == "block.toml") else {
        return false;
    };
    let Ok(mut doc) = String::from_utf8_lossy(&entry.1).parse::<toml::Table>() else {
        return false;
    };
    let Some(block) = doc.get_mut("block").and_then(toml::Value::as_table_mut) else {
        return false;
    };
    let name = block
        .get("name")
        .and_then(toml::Value::as_str)
        .unwrap_or(kind)
        .to_owned();
    block.insert("type".into(), new_kind.into());
    block.insert("name".into(), format!("{name} (из проекта)").into());
    let Ok(text) = toml::to_string_pretty(&doc) else {
        return false;
    };
    entry.1 = text.into_bytes();
    write_block(user_dir, &group, new_kind, &files)
}

fn safe_part(part: &str) -> bool {
    !part.is_empty() && part != "." && part != ".." && !part.contains([':', '\\'])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{save_block, user_library_dir, BlockDef};
    use crate::logic::Block;
    use crate::project::Project;
    use std::path::PathBuf;

    #[test]
    fn safe_parts() {
        assert!(safe_part("Насосы"));
        assert!(!safe_part(".."));
        assert!(!safe_part(""));
        assert!(!safe_part("C:"));
    }

    #[test]
    fn example_blocks_travel_in_the_example_copy() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let src = repo.join("examples/wasm_driver");
        let entries = pack_block_folders(&src.join("library"));
        assert!(entries
            .iter()
            .any(|(n, _)| n == "user_blocks/Датчики/bmp280/block.wasm"));

        let root = std::env::temp_dir().join(format!("crab_example_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let mut project = Project::load(&src).unwrap();
        project.path = root.join("wasm_driver.crab");
        let extra: Vec<(&str, &[u8])> = entries
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect();
        project.save_with(&extra).unwrap();

        let (lib, _) = Library::load(repo.join("library")).unwrap();
        assert!(lib.get("bmp280").is_none());
        let user = root.join("user_library");
        let written = unpack_missing_blocks(&project.path, &user, &lib);
        assert_eq!(written, ["bmp280"]);
        assert!(user.join("Датчики/bmp280/block.wasm").is_file());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn user_blocks_travel_in_the_project() {
        let root = std::env::temp_dir().join(format!("crab_packed_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let builtin = root.join("library");
        fs::create_dir_all(&builtin).unwrap();
        let user = user_library_dir(&builtin);
        let def: BlockDef =
            toml::from_str("[block]\ntype = \"mine\"\nname = \"N\"\ngroup = \"Раздел/Папка\"\n")
                .unwrap();
        let bdir = save_block(&user.join("Раздел").join("Папка"), &def, "// code", "docs").unwrap();
        fs::create_dir_all(bdir.join("lib/Fancy")).unwrap();
        fs::write(bdir.join("lib/Fancy/Fancy.h"), "// lib").unwrap();
        let (lib, _) = Library::load_with_user(&builtin).unwrap();
        assert!(lib.is_user("mine"));

        let mut logic = Logic::default();
        logic.blocks.push(Block {
            id: "a".into(),
            kind: "mine".into(),
            ..Default::default()
        });
        let entries = pack_user_blocks(&logic, &lib);
        assert!(entries
            .iter()
            .any(|(n, _)| n == "user_blocks/Раздел/Папка/mine/block.toml"));

        let project = Project {
            path: root.join("p.crab"),
            config: toml::from_str(
                "[project]
name = \"t\"
[board]
fqbn = \"x:y:z\"
",
            )
            .unwrap(),
            logic,
            layout: Default::default(),
        };
        let extra: Vec<(&str, &[u8])> = entries
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect();
        project.save_with(&extra).unwrap();

        let offered = project_blocks(&project.path);
        assert_eq!(offered.len(), 1);
        assert_eq!(offered[0].libraries(), ["Fancy"]);

        // Another computer: no user library at all.
        fs::remove_dir_all(&user).unwrap();
        let (empty, _) = Library::load_with_user(&builtin).unwrap();
        let written = unpack_missing_blocks(&project.path, &user, &empty);
        assert_eq!(written, ["mine"]);
        let (lib, _) = Library::load_with_user(&builtin).unwrap();
        assert!(lib.is_user("mine"));
        assert_eq!(lib.get("mine").unwrap().def.info.group, "Раздел/Папка");
        // Already present: nothing is overwritten.
        assert!(unpack_missing_blocks(&project.path, &user, &lib).is_empty());
        assert!(version_diffs(&project.path, &lib).is_empty());

        // The program's block is edited: its contents differ from the packed copy.
        let mut def = lib.get("mine").unwrap().def.clone();
        def.info.description = "changed".into();
        let dir = lib.get("mine").unwrap().dir.parent().unwrap().to_owned();
        save_block(&dir, &def, "// code 2", "docs").unwrap();
        let (lib, _) = Library::load_with_user(&builtin).unwrap();
        let diffs = version_diffs(&project.path, &lib);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].kind, "mine");
        // The block file is dated after the project was saved, then before.
        let block_dir = lib.get("mine").unwrap().dir.clone();
        fn date_files(dir: &Path, t: std::time::SystemTime) {
            for entry in fs::read_dir(dir).unwrap().flatten() {
                if entry.path().is_dir() {
                    date_files(&entry.path(), t);
                } else {
                    let file = fs::File::options().write(true).open(entry.path()).unwrap();
                    file.set_modified(t).unwrap();
                }
            }
        }
        let set_time = |t| date_files(&block_dir, t);
        let hour = std::time::Duration::from_secs(3600);
        let now = std::time::SystemTime::now();
        set_time(now + hour);
        assert_eq!(version_diffs(&project.path, &lib)[0].newer, Newer::Program);
        set_time(now - hour);
        assert_eq!(version_diffs(&project.path, &lib)[0].newer, Newer::Project);
        // Taking the project's copy brings the old contents back.
        assert_eq!(
            adopt_project_blocks(&project.path, &user, &["mine".to_owned()]),
            ["mine"]
        );
        let (lib, _) = Library::load_with_user(&builtin).unwrap();
        assert!(version_diffs(&project.path, &lib).is_empty());
        // Edited by hand elsewhere: reported again.
        fs::write(
            lib.get("mine").unwrap().dir.join("esp32.cpp.tmpl"),
            "// other",
        )
        .unwrap();
        let diffs = version_diffs(&project.path, &lib);
        assert_eq!(diffs.len(), 1);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_pack_version_tells_which_copy_is_newer() {
        let root = std::env::temp_dir().join(format!("crab_packed_ver_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let pack = root.join("Мои");
        let block = pack.join("mine");
        fs::create_dir_all(&block).unwrap();
        let mut info = crate::pack::PackInfo::new("Мои");
        info.version = 3;
        info.write(&pack).unwrap();
        let project = root.join("p.crab");
        fs::write(&project, "x").unwrap();
        let stamp = |pack: &str, version| PackStamp {
            pack: pack.to_owned(),
            version,
            history: Vec::new(),
        };
        assert_eq!(
            newer_copy(&project, &block, Some(&stamp("Мои", 2)), &[]),
            Newer::Program
        );
        assert_eq!(
            newer_copy(&project, &block, Some(&stamp("Мои", 4)), &[]),
            Newer::Project
        );
        // Another pack or no stamp: the file times decide (the block is older than the
        // project file only if it was written first, so just check no panic and a verdict).
        let by_time = newer_copy(&project, &block, None, &[]);
        assert_eq!(
            newer_copy(&project, &block, Some(&stamp("Чужие", 9)), &[]),
            by_time
        );
        assert_eq!(
            newer_copy(&project, &block, Some(&stamp("Мои", 3)), &[]),
            by_time
        );
        fs::remove_dir_all(&root).unwrap();
    }

    fn copy_dir(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for e in fs::read_dir(from).unwrap().flatten() {
            let target = to.join(e.file_name());
            if e.path().is_dir() {
                copy_dir(&e.path(), &target);
            } else {
                fs::copy(e.path(), target).unwrap();
            }
        }
    }

    /// A computer: library folder, user library with a pack, one block `mine`.
    fn computer(root: &Path) -> (PathBuf, PathBuf) {
        let builtin = root.join("library");
        fs::create_dir_all(&builtin).unwrap();
        (builtin.clone(), user_library_dir(&builtin))
    }

    fn edit_block(user: &Path, note: &str) {
        let group = user.join("Мои");
        let (lib, _) =
            Library::load_with_user(user.parent().unwrap().join("library").as_path()).unwrap();
        let mut def = lib.get("mine").unwrap().def.clone();
        def.info.description = note.into();
        save_block(&group, &def, note, "docs").unwrap();
    }

    fn project_of(root: &Path, lib: &Library) -> Project {
        let mut logic = Logic::default();
        logic.blocks.push(Block {
            id: "a".into(),
            kind: "mine".into(),
            ..Default::default()
        });
        let project = Project {
            path: root.join("p.crab"),
            config: toml::from_str(
                "[project]
name = \"t\"
[board]
fqbn = \"x:y:z\"
",
            )
            .unwrap(),
            logic,
            layout: Default::default(),
        };
        let packed = pack_user_blocks(&project.logic, lib);
        let extra: Vec<(&str, &[u8])> = packed
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect();
        project.save_with(&extra).unwrap();
        project
    }

    #[test]
    fn parallel_versions_are_told_from_descendants() {
        let root = std::env::temp_dir().join(format!("crab_packed_hist_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (one_lib, one_user) = computer(&root.join("one"));
        crate::pack::PackInfo::new("Мои")
            .write(&one_user.join("Мои"))
            .unwrap();
        let def: BlockDef = toml::from_str(
            "[block]
type = \"mine\"
name = \"N\"
group = \"Мои\"
",
        )
        .unwrap();
        save_block(&one_user.join("Мои"), &def, "// a", "docs").unwrap();

        // The second computer starts from the same block.
        let (two_lib, two_user) = computer(&root.join("two"));
        copy_dir(&one_user, &two_user);

        // The second one changes the block and saves a project.
        edit_block(&two_user, "// p");
        let (lib2, _) = Library::load_with_user(&two_lib).unwrap();
        let project = project_of(&root, &lib2);

        // The first computer is still on the old block: the project descends from it.
        let (lib1, _) = Library::load_with_user(&one_lib).unwrap();
        let diffs = version_diffs(&project.path, &lib1);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].newer, Newer::Project);

        // It changes the block on its own: now neither descends from the other.
        edit_block(&one_user, "// q");
        let (lib1, _) = Library::load_with_user(&one_lib).unwrap();
        assert_eq!(
            version_diffs(&project.path, &lib1)[0].newer,
            Newer::Parallel
        );

        // The project's copy can be kept next to the program's one.
        assert!(save_project_copy_aside(
            &project.path,
            &one_user,
            "mine",
            "mine_project"
        ));
        let (lib1, _) = Library::load_with_user(&one_lib).unwrap();
        let aside = lib1.get("mine_project").unwrap();
        assert!(aside.def.info.name.contains("из проекта"));
        assert_eq!(lib1.get("mine").unwrap().def.info.description, "// q");
        fs::remove_dir_all(&root).unwrap();
    }

    const BOARD_TOML: &str = "[board]
id = \"mine\"
name = \"Моя плата\"
chip = \"esp32\"
fqbn = \"esp32:esp32:esp32\"

[[pin]]
gpio = 4
name = \"OUT1\"
";

    #[test]
    fn the_user_board_travels_in_the_project() {
        let root = std::env::temp_dir().join(format!("crab_packed_board_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let builtin = root.join("library");
        fs::create_dir_all(&builtin).unwrap();
        let user = user_library_dir(&builtin);
        let board_dir = crate::boards::boards_dir(&user).join("mine");
        fs::create_dir_all(board_dir.join("images")).unwrap();
        fs::write(board_dir.join("board.toml"), BOARD_TOML).unwrap();
        fs::write(board_dir.join("docs.md"), "Описание").unwrap();
        fs::write(board_dir.join("images/scheme.png"), [1u8, 2, 3]).unwrap();
        let (lib, errors) = Library::load_with_user(&builtin).unwrap();
        assert!(errors.is_empty(), "{errors:?}");

        let project = Project {
            path: root.join("p.crab"),
            config: toml::from_str(
                "[project]
name = \"t\"
[board]
fqbn = \"esp32:esp32:esp32\"
model = \"mine\"
",
            )
            .unwrap(),
            logic: Logic::default(),
            layout: Default::default(),
        };
        let entries = pack_user_board(project.config.board.model.as_deref(), &lib);
        let names: Vec<_> = entries.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "user_boards/mine/board.toml",
                "user_boards/mine/docs.md",
                "user_boards/mine/images/scheme.png"
            ]
        );
        // A built-in board or no board packs nothing.
        assert!(pack_user_board(Some("esp32"), &lib).is_empty());
        assert!(pack_user_board(None, &lib).is_empty());
        let extra: Vec<(&str, &[u8])> = entries
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect();
        project.save_with(&extra).unwrap();

        // Another computer: a clean library knows no board.
        let clean_user = root.join("other").join("user_library");
        let none = Library::default();
        let written = unpack_missing_boards(&project.path, &clean_user, &none);
        assert_eq!(written, ["mine"]);
        let restored = crate::boards::boards_dir(&clean_user).join("mine");
        assert_eq!(
            fs::read(restored.join("images/scheme.png")).unwrap(),
            [1, 2, 3]
        );
        assert_eq!(
            fs::read_to_string(restored.join("docs.md")).unwrap(),
            "Описание"
        );
        // Known, or already in a folder: left alone.
        assert!(unpack_missing_boards(&project.path, &clean_user, &none).is_empty());
        assert!(unpack_missing_boards(&project.path, &user, &lib).is_empty());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_project_folder_carries_its_board_in_user_boards() {
        let root = std::env::temp_dir().join(format!("crab_folder_board_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let project = root.join("project");
        let board = project.join("user_boards/mine");
        fs::create_dir_all(board.join("images")).unwrap();
        fs::write(board.join("board.toml"), BOARD_TOML).unwrap();
        fs::write(board.join("images/scheme.png"), [1u8, 2, 3]).unwrap();
        // A folder without `board.toml` and one with a bad id are not boards.
        fs::create_dir_all(project.join("user_boards/empty")).unwrap();
        fs::create_dir_all(project.join("user_boards/Bad Id")).unwrap();
        fs::write(project.join("user_boards/Bad Id/board.toml"), BOARD_TOML).unwrap();

        let user = root.join("user_library");
        let written = unpack_missing_boards(&project, &user, &Library::default());
        assert_eq!(written, ["mine"]);
        let restored = crate::boards::boards_dir(&user).join("mine");
        assert_eq!(
            fs::read(restored.join("images/scheme.png")).unwrap(),
            [1, 2, 3]
        );
        assert!(unpack_missing_boards(&project, &user, &Library::default()).is_empty());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_broken_packed_board_is_not_written() {
        let root =
            std::env::temp_dir().join(format!("crab_packed_bad_board_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let project = Project {
            path: root.join("p.crab"),
            config: toml::from_str("[project]\nname = \"t\"\n[board]\nfqbn = \"x:y:z\"\n").unwrap(),
            logic: Logic::default(),
            layout: Default::default(),
        };
        let bad = BOARD_TOML.replace("chip = \"esp32\"", "chip = \"avr\"");
        let extra: Vec<(&str, &[u8])> = vec![
            ("user_boards/mine/board.toml", bad.as_bytes()),
            ("user_boards/mine/../escape.txt", b"x"),
            ("user_boards/Bad Id/board.toml", BOARD_TOML.as_bytes()),
        ];
        project.save_with(&extra).unwrap();
        let user = root.join("user_library");
        let written = unpack_missing_boards(&project.path, &user, &Library::default());
        assert!(written.is_empty(), "{written:?}");
        assert!(!crate::boards::boards_dir(&user).join("mine").exists());
        assert!(!root.join("escape.txt").exists());
        fs::remove_dir_all(&root).unwrap();
    }
}
