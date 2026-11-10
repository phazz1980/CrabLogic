//! The engine that runs module code: wasmi on the PC, WAMR on the board (stage 3).

use alloc::string::String;

use crate::host::Host;

/// Execution limits of one module.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Work allowed per call (`step`, `on_message`, `init`); exceeding it traps.
    pub fuel_per_call: u64,
    /// Largest linear memory in bytes.
    pub max_memory: usize,
    /// Work units (`step` calls and loop passes) a module gets from `crab.yield`.
    pub yield_budget: i32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            fuel_per_call: 5_000_000,
            max_memory: 16 * 1024 * 1024,
            yield_budget: 10_000,
        }
    }
}

/// Compiles and starts modules.
pub trait Engine {
    type Instance<H: Host + 'static>: Instance<H>;

    /// Checks the imports against [`crate::abi`] and `allow`, instantiates the module, checks
    /// `abi_version` and runs `init`. Errors are messages for the user.
    fn load<H: Host + 'static>(
        &self,
        wasm: &[u8],
        allow: &[String],
        limits: Limits,
        host: H,
    ) -> Result<Self::Instance<H>, String>;
}

/// How a call into the module ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Run {
    /// The function returned.
    Done,
    /// The module called `crab.yield` and waits for [`Instance::resume`].
    Yielded,
}

/// A running module.
pub trait Instance<H> {
    fn host(&self) -> &H;
    fn host_mut(&mut self) -> &mut H;

    /// Calls `step(dt_ms)`. A trap is an error message.
    fn call_step(&mut self, dt_ms: u32) -> Result<Run, String>;

    /// Continues the `step` that yielded, with a new work limit.
    fn resume(&mut self) -> Result<Run, String>;

    /// Whether a `step` waits for [`Instance::resume`].
    fn suspended(&self) -> bool;

    /// Puts `topic` and `data` into the module memory (`alloc`) and calls `on_message`; a
    /// `crab.yield` there does not suspend it.
    fn call_on_message(&mut self, topic: &str, data: &[u8]) -> Result<(), String>;
}
