//! The internal bus of one board: several modules, topics with a kind, bounded queues
//! (`specs/pubsub-bus`). It behaves as the master firmware must.
//!
//! * A **state** topic keeps its last value; a module that subscribes gets it at once, and a
//!   pending older value of the same topic is replaced (no loss).
//! * An **event** topic is a FIFO queue per module; when it is full the oldest message goes and the
//!   module's loss counter grows.
//! * A message longer than the topic allows is refused and counted.
//! * Replacing a module keeps the topics and their last values.

use std::collections::{HashMap, VecDeque};

use crate::mcu::{Manifest, McuModule, TopicKind};

/// Default size of a module queue and longest message when the manifest says nothing.
pub const DEFAULT_QUEUE: usize = 8;
pub const DEFAULT_MAX_LEN: usize = 64;

struct Topic {
    kind: TopicKind,
    max_len: usize,
    last: Option<Vec<u8>>,
    refused: u32,
}

impl Topic {
    fn new(kind: TopicKind, max_len: usize) -> Self {
        Topic {
            kind,
            max_len,
            last: None,
            refused: 0,
        }
    }
}

struct Slot {
    module: McuModule,
    queue: VecDeque<(String, Vec<u8>)>,
    cap: usize,
    lost: u32,
    /// How many of the module subscriptions the bus has already looked at.
    seen_subs: usize,
}

impl Slot {
    fn new(module: McuModule, manifest: &Manifest) -> Self {
        Slot {
            module,
            queue: VecDeque::new(),
            cap: if manifest.queue == 0 {
                DEFAULT_QUEUE
            } else {
                manifest.queue
            },
            lost: 0,
            seen_subs: 0,
        }
    }

    fn push(&mut self, topic: &str, data: Vec<u8>, kind: TopicKind) {
        if kind == TopicKind::State {
            // only the newest value of a state topic is worth delivering
            self.queue.retain(|(t, _)| t != topic);
        }
        if self.queue.len() >= self.cap {
            self.queue.pop_front();
            self.lost += 1;
        }
        self.queue.push_back((topic.to_owned(), data));
    }
}

#[derive(Default)]
pub struct Bus {
    slots: Vec<Slot>,
    topics: HashMap<String, Topic>,
}

impl Bus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a module; returns its index. A topic declared twice with another kind is an error.
    pub fn add(&mut self, module: McuModule, manifest: &Manifest) -> Result<usize, String> {
        self.declare(manifest)?;
        self.slots.push(Slot::new(module, manifest));
        Ok(self.slots.len() - 1)
    }

    /// Puts a new module in the place of the old one; topics and their last values stay, the
    /// messages waiting for the old module are dropped.
    pub fn replace(
        &mut self,
        index: usize,
        module: McuModule,
        manifest: &Manifest,
    ) -> Result<(), String> {
        self.declare(manifest)?;
        let lost = self.slots.get(index).ok_or("нет такого модуля")?.lost;
        let mut slot = Slot::new(module, manifest);
        slot.lost = lost;
        self.slots[index] = slot;
        Ok(())
    }

    fn declare(&mut self, manifest: &Manifest) -> Result<(), String> {
        for decl in &manifest.topics {
            let max_len = if decl.max_len == 0 {
                DEFAULT_MAX_LEN
            } else {
                decl.max_len
            };
            match self.topics.get_mut(&decl.name) {
                Some(t) if t.kind != decl.kind => {
                    return Err(format!(
                        "топик «{}» объявлен с разным видом (state/event)",
                        decl.name
                    ));
                }
                Some(t) => t.max_len = t.max_len.max(max_len),
                None => {
                    self.topics
                        .insert(decl.name.clone(), Topic::new(decl.kind, max_len));
                }
            }
        }
        Ok(())
    }

    /// One scan of every module: deliver what waits, run `step`, route what was published.
    pub fn step(&mut self, dt_ms: u32) -> Result<(), String> {
        for i in 0..self.slots.len() {
            self.sync_subscriptions(i);
            let waiting: Vec<_> = self.slots[i].queue.drain(..).collect();
            for (topic, data) in waiting {
                self.slots[i].module.deliver(&topic, &data)?;
            }
            self.slots[i].module.step(dt_ms)?;
            let published = self.slots[i].module.take_published();
            for (topic, data) in published {
                self.route(&topic, data);
            }
        }
        Ok(())
    }

    /// A module that has just subscribed to a state topic gets its last value.
    fn sync_subscriptions(&mut self, i: usize) {
        let subs = self.slots[i].module.subscriptions().to_vec();
        for topic in &subs[self.slots[i].seen_subs.min(subs.len())..] {
            if let Some(last) = self
                .topics
                .get(topic)
                .filter(|t| t.kind == TopicKind::State)
                .and_then(|t| t.last.clone())
            {
                self.slots[i].push(topic, last, TopicKind::State);
            }
        }
        self.slots[i].seen_subs = subs.len();
    }

    /// Delivers a message from the host (the outside world) as if a module had published it.
    pub fn publish(&mut self, topic: &str, data: &[u8]) {
        self.route(topic, data.to_vec());
    }

    fn route(&mut self, topic: &str, data: Vec<u8>) {
        let entry = self
            .topics
            .entry(topic.to_owned())
            .or_insert_with(|| Topic::new(TopicKind::Event, DEFAULT_MAX_LEN));
        if data.len() > entry.max_len {
            entry.refused += 1;
            return;
        }
        let kind = entry.kind;
        if kind == TopicKind::State {
            entry.last = Some(data.clone());
        }
        for slot in &mut self.slots {
            if slot.module.subscriptions().iter().any(|t| t == topic) {
                slot.push(topic, data.clone(), kind);
            }
        }
    }

    /// Event messages the module lost because its queue was full.
    pub fn lost(&self, index: usize) -> u32 {
        self.slots.get(index).map_or(0, |s| s.lost)
    }

    /// Messages of a topic refused because they were longer than the topic allows.
    pub fn refused(&self, topic: &str) -> u32 {
        self.topics.get(topic).map_or(0, |t| t.refused)
    }

    /// The last value of a state topic.
    pub fn last(&self, topic: &str) -> Option<&[u8]> {
        self.topics.get(topic)?.last.as_deref()
    }

    pub fn module(&mut self, index: usize) -> Option<&mut McuModule> {
        self.slots.get_mut(index).map(|s| &mut s.module)
    }
}
