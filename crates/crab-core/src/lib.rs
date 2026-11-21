//! Core of CrabLogic: project model, TOML loading/saving and the block library.
//!
//! The core knows nothing about GUI, AI or the board tools.

pub mod array_blocks;
pub mod arrays;
pub mod blockfile;
pub mod boardfile;
pub mod boards;
pub mod check;
pub mod cost;
pub mod devices;
pub mod edit;
mod edit_arrays;
mod edit_structs;
pub mod error;
pub mod expander;
pub mod ident;
pub mod lan8720;
pub mod layout;
pub mod library;
pub mod local;
pub mod logic;
pub mod modbus_map;
pub mod modbus_tags;
pub mod pack;
pub mod packed;
pub mod panel;
pub mod pins;
pub mod project;
pub mod protect;
pub mod report;
pub mod struct_blocks;
pub mod tags;
pub mod uart;
pub mod value;
pub mod vars;

mod toml_io;

pub use arrays::{ArrayVar, ElemType, StructField, StructVar};
pub use boards::{BoardDef, Boards, UserBoard};
pub use check::{
    bindable_type, check, check_panel, check_project, panel_binding, panel_tags, port_types,
    DiagKind, Diagnostic, PanelTag, Report, Severity,
};
pub use edit::{EditError, Fragment};
pub use error::{Error, Result};
pub use expander::{Expander, ExpanderChip};
pub use layout::{BlockPos, Layout, SheetInfo, WireRoute};
pub use library::{
    param_input_type, save_block, user_library_dir, BlockDef, Choice, Library, LibraryBlock,
    ParamDef, ParamKind, ParamShow, PinMode, PortDef, SignalType,
};
pub use local::LocalValues;
pub use logic::{Block, Link, Logic, PortRef, PortRefError};
pub use logic::{SheetExecution, SheetSpeed};
pub use panel::{Device, DeviceKind, Panel, Supply, PANEL_FILE, PANEL_PDF};
pub use pins::PinFunction;
pub use project::{
    hash_hex, BoardConfig, Project, ProjectConfig, ProjectInfo, RetainConfig, RunConfig,
    DEFAULT_PERIOD_MS, PERIODS_MS,
};
pub use uart::{UartAccess, UartConfig, UartUse};
pub use value::ParamValue;
pub use vars::{Var, VarType};
