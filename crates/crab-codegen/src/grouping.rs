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
//!   for any budget is a module of its own.

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

impl Program<'_> {
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
        pack_sheets(&facts, overhead, MAX_GROUP_BLOCKS, outputs)
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
        let kept: Vec<Vec<u32>> = planned
            .into_iter()
            .filter(|g| self.wasm_module_with(g[0], watch).is_ok())
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
}
