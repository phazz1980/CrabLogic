//! What a project will cost the board, estimated from its blocks without building the modules
//! (`specs/module-ram`): heap per module and in all, the step of each, and how many more blocks
//! fit. The numbers come from `crab_core::cost`, measured by `crablogic bench`; the linear memory
//! of text blocks (`crab.mem`) is not counted. The modules are those of the current groups
//! ([`Program::auto_group_for_board`] first, as the build does).

use crab_core::array_blocks::{array_name, DEST_PARAM};
use crab_core::cost::{costs_for, heap_for, needs_network};
use crab_core::pins::Chip;
use crab_core::SheetSpeed;

use crate::program::Program;

/// One module of the estimate.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleEstimate {
    /// Number of the first sheet of the module.
    pub unit: u32,
    pub name: String,
    pub sheets: Vec<u32>,
    pub blocks: usize,
    /// Heap on the board, bytes.
    pub heap: u32,
    /// Step of the module, microseconds.
    pub step_us: f32,
    /// How often the module runs, ms.
    pub period_ms: u32,
}

/// The estimate of a whole project.
#[derive(Debug, Clone, PartialEq)]
pub struct Estimate {
    pub chip: Option<Chip>,
    /// The project has blocks that need the network stack.
    pub network: bool,
    pub modules: Vec<ModuleEstimate>,
    /// Heap of all the modules, bytes.
    pub heap: u32,
    /// Heap the modules may take, bytes.
    pub budget: u32,
    /// Blocks of the average mix (50% logic, 30% math, 20% the rest) that still fit into the
    /// modules already there and the heap left.
    pub more_blocks: u32,
}

impl Estimate {
    /// Heap left for modules; negative when the project does not fit.
    pub fn free(&self) -> i64 {
        i64::from(self.budget) - i64::from(self.heap)
    }

    pub fn fits(&self) -> bool {
        self.free() >= 0
    }

    /// Sum of the steps of the modules that run every period, as a share of the period, percent:
    /// the largest one over all modules (they run one after another in a period).
    pub fn busiest_percent(&self) -> f32 {
        let mut per_period: std::collections::BTreeMap<u32, f32> = Default::default();
        for m in &self.modules {
            *per_period.entry(m.period_ms).or_default() += m.step_us;
        }
        per_period
            .into_iter()
            .map(|(ms, us)| us / (ms as f32 * 10.0))
            .fold(0.0, f32::max)
    }
}

impl Program<'_> {
    /// Bytes of the arrays the blocks of `sheets` use: each array once, its elements live in the
    /// linear memory of the module.
    fn array_bytes(&self, sheets: &[u32]) -> u32 {
        let mut names: Vec<&str> = Vec::new();
        for b in self
            .blocks
            .iter()
            .filter(|b| sheets.contains(&self.sheet_of(&b.block.id)))
        {
            let dest = match b.block.params.get(DEST_PARAM) {
                Some(crab_core::ParamValue::Text(name)) => Some(name.as_str()),
                _ => None,
            };
            for name in array_name(b.block).into_iter().chain(dest) {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        names
            .iter()
            .filter_map(|n| self.project.logic.array(n))
            .fold(0u32, |sum, a| sum.saturating_add(a.bytes()))
    }

    /// Estimates the project for the board of its config, with the modules as they are grouped now.
    pub fn estimate(&self) -> Estimate {
        let chip = Chip::of_board(&self.project.config.board);
        let heap = heap_for(chip);
        let costs = costs_for(chip);
        let layout = &self.project.layout;
        let mut network = false;
        let modules: Vec<ModuleEstimate> = self
            .sheets()
            .into_iter()
            .map(|unit| {
                let sheets = self.sheets_of_module(unit);
                let kinds: Vec<&str> = self
                    .blocks
                    .iter()
                    .filter(|b| sheets.contains(&self.sheet_of(&b.block.id)))
                    .map(|b| b.block.kind.as_str())
                    .collect();
                network |= kinds.iter().any(|k| needs_network(k));
                let speed = self
                    .project
                    .logic
                    .sheets
                    .iter()
                    .find(|s| s.number == unit)
                    .map_or(SheetSpeed::Normal, |s| s.speed);
                let background = speed == SheetSpeed::Background || layout.is_background(unit);
                let array_bytes = self.array_bytes(&sheets);
                ModuleEstimate {
                    unit,
                    name: self.sheet_module(unit).map(|m| m.name).unwrap_or_default(),
                    sheets,
                    blocks: kinds.len(),
                    heap: heap.module_bytes(kinds.iter().copied(), array_bytes),
                    step_us: costs.sheet_us(kinds.iter().copied()),
                    period_ms: if background {
                        layout.background_period(unit)
                    } else {
                        self.project.period_ms()
                    },
                }
            })
            .collect();
        let total = heap.project_bytes(&modules.iter().map(|m| m.heap).collect::<Vec<_>>());
        let budget = heap.budget(network);
        let mix = (50 * heap.block_bytes("and")
            + 30 * heap.block_bytes("add")
            + 20 * heap.block_bytes("digital_out"))
            / 100;
        let more_blocks = budget.saturating_sub(total).checked_div(mix).unwrap_or(0);
        Estimate {
            chip,
            network,
            modules,
            heap: total,
            budget,
            more_blocks,
        }
    }
}

#[cfg(test)]
mod tests {
    use crab_core::{Library, Project};

    use super::*;

    fn lib() -> Library {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        Library::load(root.join("library")).unwrap().0
    }

    #[test]
    fn a_small_project_fits_with_room_to_spare() {
        let lib = lib();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let project = Project::load(root.join("examples/background")).unwrap();
        let mut program = Program::build(&project, &lib).unwrap();
        let apart = program.estimate();
        assert_eq!(apart.modules.len(), 2);
        assert!(apart.fits());
        assert!(!apart.network);
        assert_eq!(
            apart.heap,
            heap_for(apart.chip).first_load + apart.modules.iter().map(|m| m.heap).sum::<u32>()
        );
        // More blocks fit: (budget - heap) / the average block.
        assert!(apart.more_blocks > 100, "{}", apart.more_blocks);

        // Joined sheets are one module with the blocks of both and a module's load less.
        program.set_groups(vec![vec![1, 2]]).unwrap();
        let joined = program.estimate();
        assert_eq!(joined.modules.len(), 1);
        assert_eq!(joined.modules[0].sheets, vec![1, 2]);
        assert_eq!(
            joined.modules[0].blocks,
            apart.modules.iter().map(|m| m.blocks).sum::<usize>()
        );
        assert!(joined.heap < apart.heap);
    }

    #[test]
    fn the_arrays_of_a_module_are_part_of_its_memory() {
        let lib = lib();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut project = Project::load(root.join("examples/array_average")).unwrap();
        let before = Program::build(&project, &lib).unwrap().estimate();
        let array = project
            .logic
            .arrays
            .first_mut()
            .expect("the example has an array");
        let old = array.bytes();
        array.len += 100;
        let grown = array.bytes() - old;
        assert!(grown > 0);
        let after = Program::build(&project, &lib).unwrap().estimate();
        assert_eq!(after.heap - before.heap, grown);
    }

    #[test]
    fn network_blocks_take_the_stack_out_of_the_budget() {
        let lib = lib();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let project = Project::load(root.join("examples/mqtt_wasm")).unwrap();
        let program = Program::build(&project, &lib).unwrap();
        let est = program.estimate();
        assert!(est.network);
        assert_eq!(est.budget, heap_for(est.chip).budget(true));
    }

    #[test]
    fn the_busiest_period_counts_the_modules_that_share_it() {
        let est = Estimate {
            chip: None,
            network: false,
            modules: vec![
                ModuleEstimate {
                    unit: 1,
                    name: "a".into(),
                    sheets: vec![1],
                    blocks: 1,
                    heap: 0,
                    step_us: 1000.0,
                    period_ms: 10,
                },
                ModuleEstimate {
                    unit: 2,
                    name: "b".into(),
                    sheets: vec![2],
                    blocks: 1,
                    heap: 0,
                    step_us: 1000.0,
                    period_ms: 10,
                },
            ],
            heap: 0,
            budget: 0,
            more_blocks: 0,
        };
        // 2000 us of a 10 ms period.
        assert!((est.busiest_percent() - 20.0).abs() < 0.01);
    }
}
