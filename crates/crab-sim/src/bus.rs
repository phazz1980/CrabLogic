//! The internal bus of one board: several modules, topics with a kind, bounded queues
//! (`specs/pubsub-bus`). The rules live in the kernel ([`crab_kernel::bus`]); this is the kernel
//! of the PC with modules on the stub hardware, addressed by the order they were added.

use crab_kernel::{Handle, Kernel};

use crate::mcu::{Manifest, McuModule, StubIo};

pub use crab_kernel::bus::{DEFAULT_MAX_LEN, DEFAULT_QUEUE};

#[derive(Default)]
pub struct Bus {
    kernel: Kernel<McuModule, StubIo>,
    /// Handle of each module by index; a replacement keeps the index.
    handles: Vec<Handle>,
}

impl Bus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a module; returns its index. A topic declared twice with another kind is an error.
    pub fn add(&mut self, module: McuModule, manifest: &Manifest) -> Result<usize, String> {
        let h = self.kernel.add(module, &manifest.topics, manifest.queue)?;
        self.handles.push(h);
        Ok(self.handles.len() - 1)
    }

    /// Puts a new module in the place of the old one; topics and their last values stay, the
    /// messages waiting for the old module are dropped.
    pub fn replace(
        &mut self,
        index: usize,
        module: McuModule,
        manifest: &Manifest,
    ) -> Result<(), String> {
        let old = *self.handles.get(index).ok_or("нет такого модуля")?;
        self.handles[index] = self
            .kernel
            .swap(old, module, &manifest.topics, manifest.queue)?;
        Ok(())
    }

    /// One scan of every module: deliver what waits, run `step`, route what was published.
    pub fn step(&mut self, dt_ms: u32) -> Result<(), String> {
        self.kernel.run_quantum(dt_ms)
    }

    /// Delivers a message from the host (the outside world) as if a module had published it.
    pub fn publish(&mut self, topic: &str, data: &[u8]) {
        self.kernel.publish(topic, data);
    }

    /// Event messages the module lost because its queue was full.
    pub fn lost(&self, index: usize) -> u32 {
        self.handles
            .get(index)
            .map_or(0, |&h| self.kernel.bus().lost(h))
    }

    /// Messages of a topic refused because they were longer than the topic allows.
    pub fn refused(&self, topic: &str) -> u32 {
        self.kernel.bus().refused(topic)
    }

    /// The last value of a state topic.
    pub fn last(&self, topic: &str) -> Option<&[u8]> {
        self.kernel.bus().last(topic)
    }

    pub fn module(&mut self, index: usize) -> Option<&mut McuModule> {
        let h = *self.handles.get(index)?;
        self.kernel.get_mut(h)
    }
}
