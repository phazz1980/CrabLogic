//! What the editor remembers about the board from the last module upload: the master's
//! capabilities and the layout of its `crabmod` flash. Used to warn about sheets that would not
//! go accelerated, without the board at hand.

use std::path::Path;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::mcu_module::MasterInfo;
use crate::upload_plan::{CrabmodSpace, StoredArea};

/// File name inside the project work folder.
pub const FILE: &str = "board.toml";

/// One stored image, as in [`StoredArea`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoArea {
    pub name: String,
    pub at: usize,
    pub len: usize,
    pub gen: u32,
}

/// The board as it was at the last module upload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardMemo {
    pub caps: Vec<String>,
    pub free: usize,
    pub capacity: usize,
    pub areas: Vec<MemoArea>,
}

impl BoardMemo {
    pub fn new(info: &MasterInfo, space: &CrabmodSpace) -> Self {
        Self {
            caps: info.caps.clone(),
            free: space.free,
            capacity: space.capacity,
            areas: space
                .areas
                .iter()
                .map(|a| MemoArea {
                    name: a.name.clone(),
                    at: a.at,
                    len: a.len,
                    gen: a.gen,
                })
                .collect(),
        }
    }

    /// Master info with only the capabilities filled.
    pub fn info(&self) -> MasterInfo {
        MasterInfo {
            caps: self.caps.clone(),
            ..MasterInfo::default()
        }
    }

    pub fn space(&self) -> CrabmodSpace {
        CrabmodSpace {
            free: self.free,
            capacity: self.capacity,
            areas: self
                .areas
                .iter()
                .map(|a| StoredArea {
                    name: a.name.clone(),
                    at: a.at,
                    len: a.len,
                    gen: a.gen,
                })
                .collect(),
        }
    }
}

/// Writes the memo into `work_dir` (created if missing).
pub fn save(work_dir: &Path, memo: &BoardMemo) -> std::io::Result<()> {
    std::fs::create_dir_all(work_dir)?;
    let text = toml::to_string_pretty(memo)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    std::fs::write(work_dir.join(FILE), text)
}

/// The memo and the time it was written; `None` if missing or unreadable.
pub fn load(work_dir: &Path) -> Option<(BoardMemo, SystemTime)> {
    let path = work_dir.join(FILE);
    let text = std::fs::read_to_string(&path).ok()?;
    let memo = toml::from_str(&text).ok()?;
    let time = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
    Some((memo, time))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> BoardMemo {
        let info = MasterInfo {
            caps: ["flashmod", "aot", "rv32imc"].map(str::to_owned).to_vec(),
            ..MasterInfo::default()
        };
        let space = CrabmodSpace {
            free: 40960,
            capacity: 131072,
            areas: vec![StoredArea {
                name: "sheet1".into(),
                at: 0,
                len: 3000,
                gen: 2,
            }],
        };
        BoardMemo::new(&info, &space)
    }

    #[test]
    fn round_trip_keeps_everything() {
        let dir = std::env::temp_dir().join(format!("crab-board-memo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let memo = sample();
        save(&dir.join("nested"), &memo).unwrap();
        let (loaded, _time) = load(&dir.join("nested")).unwrap();
        assert_eq!(loaded, memo);
        assert!(load(&dir.join("missing")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn info_and_space_survive() {
        let memo = sample();
        assert!(memo.info().aot_target().is_some());
        assert_eq!(memo.space().free_for(&["sheet1"]), 40960 + 4096);
        assert_eq!(memo.space().free_for(&["x"]), 40960);
    }
}
