//! The kernel of one board: modules by [`Handle`], the bus between them and the leases they hold.
//!
//! Life cycle: [`Kernel::add`] (a module the engine has loaded) → [`Kernel::run_quantum`] →
//! [`Kernel::swap`] (the new instance is ready → the leases pass to it → the old one goes) →
//! [`Kernel::unload`]. A trap ends every lease of the module (its outputs go to their safe
//! state); the module stays and takes its outputs again when it next writes them. Leases that
//! passed to a new version at a swap end after its first full step unless it wrote those pins.
//!
//! A module that runs out of work units calls `crab.yield`: its `step` is suspended and the
//! next quantum resumes it instead of starting a new one, so the other modules keep running.

use alloc::string::String;
use alloc::vec::Vec;
use core::marker::PhantomData;

use crate::bus::{Bus, TopicDecl};
use crate::engine::{Instance, Run};
use crate::handle::{Handle, SlotMap};
use crate::host::{Devices, ModuleHost};
use crate::lease::Leases;

/// A module taken out of memory for a while (see [`Kernel::evict`]): its place in the scan, its
/// mailbox and its output leases stay with the kernel's sheet; the outputs keep their values.
///
/// Finish it with [`Kernel::restore`] or [`Kernel::abandon`]. Dropping it without either ends
/// the leases (the outputs go to their safe state); the place in the scan is then left empty.
#[must_use = "restore or abandon the evicted module"]
pub struct Evicted<D: Devices> {
    handle: Handle,
    leases: Leases<D::Hal>,
}

/// One quantum of a module: time passes for its devices, then a suspended `step` resumes or a
/// new one starts (with the time it missed while suspended).
pub fn step_module<I, D>(inst: &mut I, dt_ms: u32) -> Result<Run, String>
where
    I: Instance<ModuleHost<D>>,
    D: Devices,
{
    inst.host_mut().devices.tick(dt_ms);
    if inst.suspended() {
        let host = inst.host_mut();
        host.missed_ms = host.missed_ms.wrapping_add(dt_ms);
        return inst.resume();
    }
    let dt = dt_ms.wrapping_add(core::mem::take(&mut inst.host_mut().missed_ms));
    inst.call_step(dt)
}

/// One whole step of a module alone (the simulator of one sheet): yields are resumed at once,
/// at most `max_yields` times, after which the module counts as hung.
pub fn step_to_end<I, D>(inst: &mut I, dt_ms: u32, max_yields: u32) -> Result<(), String>
where
    I: Instance<ModuleHost<D>>,
    D: Devices,
{
    let mut run = step_module(inst, dt_ms)?;
    let mut yields = 0;
    while run == Run::Yielded {
        yields += 1;
        if yields > max_yields {
            return Err(String::from(
                "модуль выполняется слишком долго (возможно, бесконечный цикл)",
            ));
        }
        run = inst.resume()?;
    }
    Ok(())
}

/// Hands a message to a module if it subscribed to `topic`; returns whether it was delivered.
pub fn deliver_message<I, D>(inst: &mut I, topic: &str, data: &[u8]) -> Result<bool, String>
where
    I: Instance<ModuleHost<D>>,
    D: Devices,
{
    if !inst.host().subscriptions.iter().any(|t| t == topic) {
        return Ok(false);
    }
    inst.call_on_message(topic, data)?;
    Ok(true)
}

pub struct Kernel<I, D> {
    modules: SlotMap<I>,
    /// Scan order: the order modules were added; a replacement keeps the place.
    order: Vec<Handle>,
    bus: Bus,
    _devices: PhantomData<D>,
}

impl<I, D> Default for Kernel<I, D> {
    fn default() -> Self {
        Self {
            modules: SlotMap::new(),
            order: Vec::new(),
            bus: Bus::new(),
            _devices: PhantomData,
        }
    }
}

impl<I, D> Kernel<I, D>
where
    I: Instance<ModuleHost<D>>,
    D: Devices,
{
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a loaded module with the topics it declares and its queue size (0 → default).
    pub fn add(&mut self, inst: I, topics: &[TopicDecl], queue: usize) -> Result<Handle, String> {
        self.bus.declare(topics)?;
        let h = self.insert(inst)?;
        self.bus.attach(h, queue);
        self.note(h);
        self.order.push(h);
        Ok(h)
    }

    fn insert(&mut self, inst: I) -> Result<Handle, String> {
        self.modules
            .insert(inst)
            .ok_or_else(|| String::from("слишком много модулей"))
    }

    fn note(&mut self, h: Handle) {
        if let Some(m) = self.modules.get(h) {
            self.bus.note_subscriptions(h, &m.host().subscriptions);
        }
    }

    /// Puts a new instance in the place of `old`: the topics and their last values stay, the
    /// leases pass to the new instance (no output changes), the messages waiting for the old one
    /// are dropped. Returns the handle of the new module; `old` goes stale.
    pub fn swap(
        &mut self,
        old: Handle,
        inst: I,
        topics: &[TopicDecl],
        queue: usize,
    ) -> Result<Handle, String> {
        let pos = self
            .order
            .iter()
            .position(|&h| h == old)
            .ok_or_else(|| String::from("нет такого модуля"))?;
        self.bus.declare(topics)?;
        let new = self.insert(inst)?;
        let mut leaving = self.modules.remove(old).expect("module in order");
        if let Some(m) = self.modules.get_mut(new) {
            leaving
                .host_mut()
                .leases
                .hand_over(&mut m.host_mut().leases);
        }
        drop(leaving);
        self.bus.transfer(old, new, queue);
        self.note(new);
        self.order[pos] = new;
        Ok(new)
    }

    /// Frees the memory of a module but keeps its outputs: the instance is dropped, its leases
    /// move into the returned [`Evicted`] (no output changes), its place in the scan and its
    /// mailbox stay (messages wait). Until [`Kernel::restore`] / [`Kernel::abandon`] the scan
    /// skips the place.
    pub fn evict(&mut self, h: Handle) -> Result<Evicted<D>, String> {
        if !self.order.contains(&h) {
            return Err(String::from("нет такого модуля"));
        }
        let mut leaving = self
            .modules
            .remove(h)
            .ok_or_else(|| String::from("нет такого модуля"))?;
        let leases = leaving.host_mut().leases.take_all();
        drop(leaving);
        Ok(Evicted { handle: h, leases })
    }

    /// Puts a new instance in the place of an evicted module: the kept leases pass to it (no
    /// output changes), the waiting messages stay for it. Returns the new handle. If the
    /// instance cannot be inserted, the module is lost: its leases end and its place goes.
    pub fn restore(&mut self, evicted: Evicted<D>, inst: I) -> Result<Handle, String> {
        let Evicted {
            handle: old,
            mut leases,
        } = evicted;
        let pos = self
            .order
            .iter()
            .position(|&h| h == old)
            .ok_or_else(|| String::from("нет такого модуля"))?;
        let new = match self.insert(inst) {
            Ok(new) => new,
            Err(e) => {
                self.order.retain(|&x| x != old);
                self.bus.detach(old);
                return Err(e);
            }
        };
        if let Some(m) = self.modules.get_mut(new) {
            leases.hand_over(&mut m.host_mut().leases);
        }
        self.bus.rekey(old, new);
        self.note(new);
        self.order[pos] = new;
        Ok(new)
    }

    /// Gives up an evicted module: its leases end (outputs go to their safe state), its place in
    /// the scan and its mailbox go.
    pub fn abandon(&mut self, evicted: Evicted<D>) {
        let h = evicted.handle;
        self.order.retain(|&x| x != h);
        self.bus.detach(h);
        drop(evicted);
    }

    /// Removes a module: its leases end (outputs go to their safe state), its mailbox closes.
    pub fn unload(&mut self, h: Handle) -> Option<I> {
        let inst = self.modules.remove(h)?;
        self.order.retain(|&x| x != h);
        self.bus.detach(h);
        let mut inst = inst;
        inst.host_mut().leases.release();
        Some(inst)
    }

    /// One scan of every module: deliver what waits, run `step`, route what was published.
    /// The first error stops the scan; the failing module's leases end.
    pub fn run_quantum(&mut self, dt_ms: u32) -> Result<(), String> {
        for i in 0..self.order.len() {
            self.step(self.order[i], dt_ms)?;
        }
        Ok(())
    }

    /// One scan of one module, for a host that paces the modules itself (its own periods,
    /// pauses, statistics). On an error the module's leases end.
    pub fn step(&mut self, h: Handle, dt_ms: u32) -> Result<(), String> {
        let result = self.step_one(h, dt_ms);
        if let Some(m) = self.modules.get_mut(h) {
            match result {
                Err(_) => m.host_mut().leases.release(),
                // Only a step that ran to its end proves which pins the module drives.
                Ok(Run::Done) if m.host().leases.has_inherited() => {
                    m.host_mut().leases.release_inherited();
                }
                Ok(_) => {}
            }
        }
        result.map(|_| ())
    }

    fn step_one(&mut self, h: Handle, dt_ms: u32) -> Result<Run, String> {
        // An evicted place has no module: its messages wait.
        if !self.modules.contains(h) {
            return Ok(Run::Done);
        }
        // A module still in its last step gets the waiting messages before its next one.
        let suspended = self.modules.get(h).is_some_and(|m| m.suspended());
        let waiting = if suspended {
            Vec::new()
        } else {
            self.bus.take(h)
        };
        let Some(m) = self.modules.get_mut(h) else {
            return Ok(Run::Done);
        };
        for (topic, data) in waiting {
            deliver_message(m, &topic, &data)?;
        }
        let run = step_module(m, dt_ms)?;
        let published = core::mem::take(&mut m.host_mut().published);
        self.note(h);
        for (topic, data) in published {
            self.bus.publish(&topic, data);
        }
        Ok(run)
    }

    /// Delivers a message from the outside world as if a module had published it.
    pub fn publish(&mut self, topic: &str, data: &[u8]) {
        self.bus.publish(topic, data.to_vec());
    }

    pub fn get(&self, h: Handle) -> Option<&I> {
        self.modules.get(h)
    }

    pub fn get_mut(&mut self, h: Handle) -> Option<&mut I> {
        self.modules.get_mut(h)
    }

    /// Another module than `except` that holds a lease of `pin`, if any.
    pub fn holder_of(&self, pin: u8, except: Handle) -> Option<Handle> {
        self.order.iter().copied().find(|&h| {
            h != except
                && self
                    .modules
                    .get(h)
                    .is_some_and(|m| m.host().leases.holds(pin))
        })
    }

    /// Modules in scan order.
    pub fn handles(&self) -> &[Handle] {
        &self.order
    }

    pub fn bus(&self) -> &Bus {
        &self.bus
    }

    pub fn bus_mut(&mut self) -> &mut Bus {
        &mut self.bus
    }
}
