//! Which sheets the build joins into one module (`specs/module-ram`, stage 1: automatic, with no
//! setting for the user). A module costs the board about 13 KB of heap whatever it holds, so
//! many small sheets are joined while the joined step stays short.
//!
//! The rules, in the order they are applied to the sheets (ascending):
//! * a sheet with the `web_ui` block stays a module of its own;
//! * only sheets of one class are joined: the same speed (fast, normal, background) and, for the
//!   background, the same period;
//! * a group grows while its estimated step (`crab_core::cost`, the call of the module once and
//!   every block) stays within [`STEP_BUDGET_PERCENT`] of the period, and it holds at most
//!   [`MAX_GROUP_BLOCKS`] blocks;
//! * a sheet is never split: one that does not fit a group starts a new one, and a sheet too big
//!   for any budget is a module of its own;
//! * sheets whose blocks reach the same array or structure are one module whatever the budget
//!   (`specs/arrays-structs` task 5): the data lives in the memory of a module. They need one class
//!   and no `web_ui` sheet among them; otherwise they stay apart and the build says why.

use crab_core::cost::costs_for;
use crab_core::pins::Chip;
use crab_core::SheetSpeed;

use crate::program::Program;
use crate::sheet::GroupError;
use crate::wasm::MAX_WATCHED;

/// Share of the cycle period the step of one module may take, percent. A module that overruns
/// its period delays every other (`specs/fast-cycle`); the estimate of `crab_core::cost` is
/// about 40% below the bench steps, so the real share is higher than this.
pub const STEP_BUDGET_PERCENT: u32 = 25;

/// Most blocks of one joined module. A guess for what the master loads in one piece (the
/// biggest free block of an ESP32 is about 110 KB); the bench ran 500 blocks of one sheet.
pub const MAX_GROUP_BLOCKS: usize = 1000;

/// Sheets can be joined when their class is equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Class {
    /// 0 fast, 1 normal, 2 background.
    pub speed: u8,
    /// Period of a background sheet, ms; 0 for the others.
    pub period_ms: u32,
}

/// What the rules need to know about a sheet.
#[derive(Debug, Clone, PartialEq)]
pub struct SheetFacts {
    pub number: u32,
    pub class: Class,
    /// The sheet holds the `web_ui` block and is a module of its own.
    pub alone: bool,
    pub blocks: usize,
    /// Output ports of the blocks (the signals a debug build reports).
    pub outputs: usize,
    /// Estimated step of the blocks of the sheet, microseconds.
    pub step_us: f32,
    /// Step the sheet's module may take, microseconds.
    pub budget_us: f32,
}

/// Packs the sheets, given ascending, into groups; `overhead_us` is the cost of the call of a
/// module, paid once per group. Returns the groups of two sheets or more, each ascending and
/// all by their first sheet; a sheet in none of them is a module of its own.
pub fn pack_sheets(
    sheets: &[SheetFacts],
    overhead_us: f32,
    max_blocks: usize,
    max_outputs: usize,
) -> Vec<Vec<u32>> {
    struct Open {
        class: Class,
        sheets: Vec<u32>,
        blocks: usize,
        outputs: usize,
        step_us: f32,
    }
    let mut open: Vec<Open> = Vec::new();
    let mut closed: Vec<Vec<u32>> = Vec::new();
    for s in sheets {
        if s.alone {
            closed.push(vec![s.number]);
            continue;
        }
        let at = open.iter().position(|o| o.class == s.class);
        if let Some(i) = at {
            let o = &mut open[i];
            if o.blocks + s.blocks <= max_blocks
                && o.outputs + s.outputs <= max_outputs
                && overhead_us + o.step_us + s.step_us <= s.budget_us
            {
                o.sheets.push(s.number);
                o.blocks += s.blocks;
                o.outputs += s.outputs;
                o.step_us += s.step_us;
                continue;
            }
            // The next sheet does not fit: this group is done.
            closed.push(open.remove(i).sheets);
        }
        open.push(Open {
            class: s.class,
            sheets: vec![s.number],
            blocks: s.blocks,
            outputs: s.outputs,
            step_us: s.step_us,
        });
    }
    closed.extend(open.into_iter().map(|o| o.sheets));
    closed.retain(|g| g.len() > 1);
    closed.sort_unstable_by_key(|g| g[0]);
    closed
}

/// Joins the facts of the sheets of every set of `shared` into one sheet (numbered by the smallest
/// member), packs, and expands the groups back. A set that cannot be one module (two classes, or a
/// `web_ui` sheet in it) is left as it is. A joined sheet that packs with no other still makes a group.
pub fn pack_sheets_shared(
    sheets: &[SheetFacts],
    shared: &[Vec<u32>],
    overhead_us: f32,
    max_blocks: usize,
    max_outputs: usize,
) -> Vec<Vec<u32>> {
    let mut facts: Vec<SheetFacts> = sheets.to_vec();
    let mut members: std::collections::BTreeMap<u32, Vec<u32>> = Default::default();
    for set in shared {
        let mut found: Vec<SheetFacts> = facts
            .iter()
            .filter(|f| set.contains(&f.number))
            .cloned()
            .collect();
        found.sort_unstable_by_key(|f| f.number);
        if found.len() < 2 || found.iter().any(|f| f.alone || f.class != found[0].class) {
            continue;
        }
        let mut one = found[0].clone();
        for f in &found[1..] {
            one.blocks += f.blocks;
            one.outputs += f.outputs;
            one.step_us += f.step_us;
            one.budget_us = one.budget_us.min(f.budget_us);
        }
        let all: Vec<u32> = found.iter().map(|f| f.number).collect();
        facts.retain(|f| !all.contains(&f.number));
        facts.push(one);
        members.insert(all[0], all);
    }
    facts.sort_unstable_by_key(|f| f.number);
    let packed = pack_sheets(&facts, overhead_us, max_blocks, max_outputs);
    let expand = |number: u32| {
        members
            .get(&number)
            .cloned()
            .unwrap_or_else(|| vec![number])
    };
    let mut groups: Vec<Vec<u32>> = packed
        .iter()
        .map(|g| {
            let mut all: Vec<u32> = g.iter().flat_map(|&n| expand(n)).collect();
            all.sort_unstable();
            all
        })
        .collect();
    // A joined sheet that no group took still is one module of several sheets.
    for (number, all) in &members {
        if !packed.iter().any(|g| g.contains(number)) {
            groups.push(all.clone());
        }
    }
    groups.sort_unstable_by_key(|g| g[0]);
    groups
}

impl Program<'_> {
    /// Sets of sheets (ascending, two or more each) whose blocks reach the same array or structure.
    /// Sets that share a sheet are merged.
    pub fn shared_sheet_sets(&self) -> Vec<Vec<u32>> {
        use crab_core::array_blocks::{array_name, ArrayOp, DEST_PARAM};
        use crab_core::struct_blocks::used_structs;
        use std::collections::{BTreeMap, BTreeSet};
        let mut users: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
        for b in &self.blocks {
            let sheet = self.sheet_of(&b.block.id);
            let mut names: Vec<&str> = Vec::new();
            names.extend(array_name(b.block));
            names.extend(used_structs(b.block));
            if ArrayOp::from_kind(&b.block.kind).is_some() {
                if let Some(crab_core::ParamValue::Text(d)) = b.block.params.get(DEST_PARAM) {
                    names.push(d);
                }
            }
            for name in names.into_iter().filter(|n| !n.is_empty()) {
                users.entry(name.to_owned()).or_default().insert(sheet);
            }
        }
        let mut sets: Vec<BTreeSet<u32>> = users.into_values().filter(|s| s.len() > 1).collect();
        // Merge sets that have a sheet in common until none do.
        let mut i = 0;
        while i < sets.len() {
            let mut j = i + 1;
            let mut merged = false;
            while j < sets.len() {
                if sets[i].intersection(&sets[j]).next().is_some() {
                    let other = sets.remove(j);
                    sets[i].extend(other);
                    merged = true;
                } else {
                    j += 1;
                }
            }
            if !merged {
                i += 1;
            }
        }
        sets.into_iter().map(|s| s.into_iter().collect()).collect()
    }

    /// The groups the build must make whatever the budget: the sheets that share an array or a
    /// structure, where they can be one module.
    pub fn shared_groups(&self, chip: Option<Chip>) -> Vec<Vec<u32>> {
        let shared = self.shared_sheet_sets();
        self.plan_groups(chip)
            .into_iter()
            .filter(|g| shared.iter().any(|set| set.iter().all(|s| g.contains(s))))
            .collect()
    }

    /// The facts about every sheet that has blocks, ascending.
    fn sheet_facts(&self, chip: Option<Chip>) -> (Vec<SheetFacts>, f32) {
        let costs = costs_for(chip);
        let layout = &self.project.layout;
        let period_us = |ms: u32| ms as f32 * 1000.0 * STEP_BUDGET_PERCENT as f32 / 100.0;
        let facts = self
            .physical_sheets()
            .into_iter()
            .map(|number| {
                let on_sheet: Vec<_> = self
                    .blocks
                    .iter()
                    .filter(|b| self.sheet_of(&b.block.id) == number)
                    .collect();
                let kinds: Vec<&str> = on_sheet.iter().map(|b| b.block.kind.as_str()).collect();
                let speed = self
                    .project
                    .logic
                    .sheets
                    .iter()
                    .find(|s| s.number == number)
                    .map_or(SheetSpeed::Normal, |s| s.speed);
                let background = speed == SheetSpeed::Background || layout.is_background(number);
                let (code, period_ms) = if background {
                    (2, layout.background_period(number))
                } else {
                    (if speed == SheetSpeed::Fast { 0 } else { 1 }, 0)
                };
                let cycle_ms = if background {
                    period_ms
                } else {
                    self.project.period_ms()
                };
                SheetFacts {
                    number,
                    class: Class {
                        speed: code,
                        period_ms,
                    },
                    alone: kinds.contains(&"web_ui"),
                    blocks: kinds.len(),
                    outputs: on_sheet.iter().map(|b| b.lib.def.outputs.len()).sum(),
                    step_us: kinds.iter().map(|k| costs.block_us(k)).sum(),
                    budget_us: period_us(cycle_ms),
                }
            })
            .collect();
        (facts, costs.overhead)
    }

    /// The groups the build makes of the sheets of this program for `chip` (`None`: an unknown
    /// board, estimated as an ESP32). Only groups of two sheets or more.
    pub fn plan_groups(&self, chip: Option<Chip>) -> Vec<Vec<u32>> {
        self.plan_groups_for(chip, false)
    }

    /// Like [`Program::plan_groups`]; a `watch` (debug) build also keeps a group within the
    /// signals the master reports per module.
    pub fn plan_groups_for(&self, chip: Option<Chip>, watch: bool) -> Vec<Vec<u32>> {
        let (facts, overhead) = self.sheet_facts(chip);
        let outputs = if watch { MAX_WATCHED } else { usize::MAX };
        pack_sheets_shared(
            &facts,
            &self.shared_sheet_sets(),
            overhead,
            MAX_GROUP_BLOCKS,
            outputs,
        )
    }

    /// Joins the sheets as [`Program::plan_groups`] plans.
    pub fn auto_group(&mut self, chip: Option<Chip>) -> Result<(), GroupError> {
        let groups = self.plan_groups(chip);
        self.set_groups(groups)
    }

    /// Joins the sheets for the board of the project (an unknown board: as an ESP32); `watch`
    /// for a debug build.
    ///
    /// Every joined module is built once as a trial; a group whose module does not build
    /// (for example, its texts overflow the module memory) falls back to separate sheets.
    pub fn auto_group_for_board(&mut self, watch: bool) -> Result<(), GroupError> {
        let chip = Chip::of_board(&self.project.config.board);
        let planned = self.plan_groups_for(chip, watch);
        self.set_groups(planned.clone())?;
        // A group of sheets that share data stays even if its module does not build: separated, the
        // sheets would only report that the data is shared, and the real reason is in the module.
        let shared = self.shared_sheet_sets();
        let kept: Vec<Vec<u32>> = planned
            .into_iter()
            .filter(|g| {
                shared.iter().any(|set| set.iter().all(|s| g.contains(s)))
                    || self.wasm_module_with(g[0], watch).is_ok()
            })
            .collect();
        self.set_groups(kept)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NORMAL: Class = Class {
        speed: 1,
        period_ms: 0,
    };

    fn sheet(number: u32, class: Class, blocks: usize, step_us: f32) -> SheetFacts {
        SheetFacts {
            number,
            class,
            alone: false,
            blocks,
            outputs: blocks,
            step_us,
            budget_us: 2500.0,
        }
    }

    #[test]
    fn small_sheets_of_one_class_join() {
        let sheets: Vec<_> = (1..=5).map(|n| sheet(n, NORMAL, 10, 40.0)).collect();
        assert_eq!(
            pack_sheets(&sheets, 30.0, 1000, usize::MAX),
            vec![vec![1, 2, 3, 4, 5]]
        );
    }

    #[test]
    fn a_lone_sheet_makes_no_group() {
        let sheets = [sheet(1, NORMAL, 10, 40.0)];
        assert!(pack_sheets(&sheets, 30.0, 1000, usize::MAX).is_empty());
    }

    #[test]
    fn the_step_budget_cuts_groups() {
        // 30 + 3 × 800 = 2430 fits, a fourth sheet makes 3230.
        let sheets: Vec<_> = (1..=6).map(|n| sheet(n, NORMAL, 10, 800.0)).collect();
        assert_eq!(
            pack_sheets(&sheets, 30.0, 1000, usize::MAX),
            vec![vec![1, 2, 3], vec![4, 5, 6]]
        );
    }

    #[test]
    fn the_block_limit_cuts_groups() {
        let sheets: Vec<_> = (1..=5).map(|n| sheet(n, NORMAL, 400, 1.0)).collect();
        assert_eq!(
            pack_sheets(&sheets, 30.0, 1000, usize::MAX),
            vec![vec![1, 2], vec![3, 4]]
        );
    }

    #[test]
    fn a_sheet_over_the_budget_stays_alone() {
        let sheets = [
            sheet(1, NORMAL, 10, 40.0),
            sheet(2, NORMAL, 900, 4000.0),
            sheet(3, NORMAL, 10, 40.0),
            sheet(4, NORMAL, 10, 40.0),
        ];
        assert_eq!(
            pack_sheets(&sheets, 30.0, 1000, usize::MAX),
            vec![vec![3, 4]]
        );
    }

    #[test]
    fn classes_do_not_mix_but_do_skip_over_each_other() {
        let fast = Class {
            speed: 0,
            period_ms: 0,
        };
        let background = Class {
            speed: 2,
            period_ms: 10,
        };
        let slower_background = Class {
            speed: 2,
            period_ms: 50,
        };
        let sheets = [
            sheet(1, NORMAL, 10, 40.0),
            sheet(2, fast, 10, 40.0),
            sheet(3, NORMAL, 10, 40.0),
            sheet(4, background, 10, 40.0),
            sheet(5, slower_background, 10, 40.0),
            sheet(6, background, 10, 40.0),
            sheet(7, fast, 10, 40.0),
        ];
        // Sheet 3 joins sheet 1 over the fast sheet 2; background sheets of different periods
        // stay apart.
        assert_eq!(
            pack_sheets(&sheets, 30.0, 1000, usize::MAX),
            vec![vec![1, 3], vec![2, 7], vec![4, 6]]
        );
    }

    #[test]
    fn the_watch_limit_cuts_groups() {
        let sheets: Vec<_> = (1..=4).map(|n| sheet(n, NORMAL, 10, 1.0)).collect();
        assert_eq!(
            pack_sheets(&sheets, 30.0, 1000, 20),
            vec![vec![1, 2], vec![3, 4]]
        );
    }

    #[test]
    fn the_web_page_sheet_stays_alone() {
        let mut web = sheet(2, NORMAL, 10, 40.0);
        web.alone = true;
        let sheets = [sheet(1, NORMAL, 10, 40.0), web, sheet(3, NORMAL, 10, 40.0)];
        assert_eq!(
            pack_sheets(&sheets, 30.0, 1000, usize::MAX),
            vec![vec![1, 3]]
        );
    }

    #[test]
    fn sheets_that_share_data_are_one_module_over_the_budget() {
        // Sheets 1 and 3 share an array: each is too big to join anything, together they are a group.
        let sheets = [
            sheet(1, NORMAL, 10, 2000.0),
            sheet(2, NORMAL, 10, 40.0),
            sheet(3, NORMAL, 10, 2000.0),
        ];
        assert_eq!(
            pack_sheets_shared(&sheets, &[vec![1, 3]], 30.0, 1000, usize::MAX),
            vec![vec![1, 3]]
        );
        // Without the shared set the budget keeps them apart.
        assert!(pack_sheets(&sheets, 30.0, 1000, usize::MAX)
            .iter()
            .all(|g| g != &vec![1, 3]));
    }

    #[test]
    fn a_shared_group_takes_small_sheets_with_it() {
        let sheets: Vec<_> = (1..=4).map(|n| sheet(n, NORMAL, 10, 40.0)).collect();
        assert_eq!(
            pack_sheets_shared(&sheets, &[vec![2, 4]], 30.0, 1000, usize::MAX),
            vec![vec![1, 2, 3, 4]]
        );
    }

    #[test]
    fn sheets_of_two_classes_or_a_web_sheet_cannot_be_joined_by_sharing() {
        let fast = Class {
            speed: 0,
            period_ms: 0,
        };
        let sheets = [sheet(1, NORMAL, 10, 2000.0), sheet(2, fast, 10, 2000.0)];
        assert!(pack_sheets_shared(&sheets, &[vec![1, 2]], 30.0, 1000, usize::MAX).is_empty());
        let mut web = sheet(2, NORMAL, 10, 2000.0);
        web.alone = true;
        let sheets = [sheet(1, NORMAL, 10, 2000.0), web];
        assert!(pack_sheets_shared(&sheets, &[vec![1, 2]], 30.0, 1000, usize::MAX).is_empty());
    }
}
