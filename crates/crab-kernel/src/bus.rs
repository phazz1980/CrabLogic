//! The internal bus of one board (`specs/pubsub-bus`): topics with a kind, a bounded mailbox per
//! module, loss and refusal counters. The bus does not own the modules: mailboxes are keyed by
//! the module [`Handle`].
//!
//! * A **state** topic keeps its last value; a module that subscribes gets it at its next scan, and
//!   a pending older value of the same topic is replaced (no loss).
//! * An **event** topic is a FIFO queue per module; when it is full the oldest message goes and the
//!   module's loss counter grows.
//! * A message longer than the topic allows is refused and counted.
//! * Replacing a module keeps the topics and their last values; its loss counter carries over and
//!   the messages waiting for the old module are dropped.

use alloc::borrow::ToOwned;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::handle::Handle;

/// Default size of a module queue and longest message when the manifest says nothing.
pub const DEFAULT_QUEUE: usize = 8;
pub const DEFAULT_MAX_LEN: usize = 64;

/// The kind of a topic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum TopicKind {
    /// The last value matters: kept for late subscribers, a newer one replaces a pending one.
    State,
    /// Every message matters: queued in order, the oldest goes when the queue is full.
    #[default]
    Event,
}

/// A topic a module declares (one entry of the manifest `topics`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
pub struct TopicDecl {
    pub name: String,
    #[cfg_attr(feature = "serde", serde(default))]
    pub kind: TopicKind,
    /// Longest message in bytes; 0 means [`DEFAULT_MAX_LEN`].
    #[cfg_attr(feature = "serde", serde(default))]
    pub max_len: usize,
}

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

struct Mailbox {
    queue: VecDeque<(String, Vec<u8>)>,
    cap: usize,
    lost: u32,
    /// Topics the module subscribed to, in order (they only grow).
    subs: Vec<String>,
    /// How many of `subs` already got the last state value.
    seen: usize,
}

impl Mailbox {
    fn new(cap: usize) -> Self {
        Mailbox {
            queue: VecDeque::new(),
            cap: if cap == 0 { DEFAULT_QUEUE } else { cap },
            lost: 0,
            subs: Vec::new(),
            seen: 0,
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
    boxes: BTreeMap<Handle, Mailbox>,
    topics: BTreeMap<String, Topic>,
}

impl Bus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares the topics of a module. A topic declared twice with another kind is an error; the
    /// longest message allowed is the largest of the declarations.
    pub fn declare(&mut self, topics: &[TopicDecl]) -> Result<(), String> {
        for decl in topics {
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

    /// Opens a mailbox of `cap` messages (0 → [`DEFAULT_QUEUE`]) for a module.
    pub fn attach(&mut self, h: Handle, cap: usize) {
        self.boxes.insert(h, Mailbox::new(cap));
    }

    /// Closes the mailbox of a module; what waited in it is dropped.
    pub fn detach(&mut self, h: Handle) {
        self.boxes.remove(&h);
    }

    /// Gives the mailbox of `old` to `new`: the loss counter stays, the queue is dropped and the
    /// subscriptions start empty (the new module subscribes itself).
    pub fn transfer(&mut self, old: Handle, new: Handle, cap: usize) {
        let lost = self.boxes.remove(&old).map_or(0, |b| b.lost);
        let mut mailbox = Mailbox::new(cap);
        mailbox.lost = lost;
        self.boxes.insert(new, mailbox);
    }

    /// Moves the mailbox of `old` to `new` keeping the waiting messages and the loss counter
    /// (a module restarted in place). The subscriptions start empty: the new module subscribes
    /// itself, and a message it does not subscribe to is skipped at delivery.
    pub fn rekey(&mut self, old: Handle, new: Handle) {
        let mailbox = match self.boxes.remove(&old) {
            Some(b) => Mailbox {
                subs: Vec::new(),
                seen: 0,
                ..b
            },
            None => Mailbox::new(0),
        };
        self.boxes.insert(new, mailbox);
    }

    /// Records the module subscriptions (`subs` only grows); messages routed from now on reach
    /// the new topics. The last state value comes at the next [`Bus::take`].
    pub fn note_subscriptions(&mut self, h: Handle, subs: &[String]) {
        if let Some(b) = self.boxes.get_mut(&h) {
            if subs.len() > b.subs.len() {
                b.subs.extend_from_slice(&subs[b.subs.len()..]);
            }
        }
    }

    /// The messages waiting for a module, oldest first. A topic it subscribed to since the last
    /// call first gets its last state value.
    pub fn take(&mut self, h: Handle) -> Vec<(String, Vec<u8>)> {
        let Some(b) = self.boxes.get_mut(&h) else {
            return Vec::new();
        };
        for i in b.seen..b.subs.len() {
            let last = self
                .topics
                .get(&b.subs[i])
                .filter(|t| t.kind == TopicKind::State)
                .and_then(|t| t.last.clone());
            if let Some(last) = last {
                let topic = b.subs[i].clone();
                b.push(&topic, last, TopicKind::State);
            }
        }
        b.seen = b.subs.len();
        b.queue.drain(..).collect()
    }

    /// Routes a message to every module subscribed to `topic`. An unknown topic becomes an event
    /// topic with the default length.
    pub fn publish(&mut self, topic: &str, data: Vec<u8>) {
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
        for b in self.boxes.values_mut() {
            if b.subs.iter().any(|t| t == topic) {
                b.push(topic, data.clone(), kind);
            }
        }
    }

    /// Event messages the module lost because its queue was full.
    pub fn lost(&self, h: Handle) -> u32 {
        self.boxes.get(&h).map_or(0, |b| b.lost)
    }

    /// Messages of a topic refused because they were longer than the topic allows.
    pub fn refused(&self, topic: &str) -> u32 {
        self.topics.get(topic).map_or(0, |t| t.refused)
    }

    /// The last value of a state topic.
    pub fn last(&self, topic: &str) -> Option<&[u8]> {
        self.topics.get(topic)?.last.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handle::SlotMap;
    use alloc::vec;

    fn handles<const N: usize>() -> [Handle; N] {
        let mut map = SlotMap::new();
        core::array::from_fn(|_| map.insert(()).unwrap())
    }

    fn sub(bus: &mut Bus, h: Handle, topic: &str) {
        bus.note_subscriptions(h, &[topic.to_owned()]);
    }

    #[test]
    fn state_reaches_late_subscriber_and_replaces_pending() {
        let [h] = handles();
        let mut bus = Bus::new();
        bus.declare(&[TopicDecl {
            name: "t".into(),
            kind: TopicKind::State,
            max_len: 0,
        }])
        .unwrap();
        bus.attach(h, 2);
        bus.publish("t", vec![1]);
        sub(&mut bus, h, "t");
        bus.publish("t", vec![2]);
        bus.publish("t", vec![3]);
        assert_eq!(bus.take(h), vec![("t".into(), vec![3])]);
        assert_eq!(bus.lost(h), 0);
        assert_eq!(bus.last("t"), Some(&[3u8][..]));
    }

    #[test]
    fn event_queue_drops_oldest_and_counts() {
        let [a, b] = handles();
        let mut bus = Bus::new();
        bus.attach(a, 2);
        bus.attach(b, 0);
        sub(&mut bus, a, "e");
        for i in 0..3 {
            bus.publish("e", vec![i]);
        }
        assert_eq!(
            bus.take(a),
            vec![("e".into(), vec![1]), ("e".into(), vec![2])]
        );
        assert_eq!(bus.lost(a), 1);
        assert!(bus.take(b).is_empty(), "not subscribed");
        bus.publish("e", vec![0; DEFAULT_MAX_LEN + 1]);
        assert_eq!(bus.refused("e"), 1);
    }

    #[test]
    fn transfer_keeps_loss_drops_queue() {
        let [old, new] = handles();
        let mut bus = Bus::new();
        bus.attach(old, 1);
        sub(&mut bus, old, "e");
        bus.publish("e", vec![1]);
        bus.publish("e", vec![2]);
        bus.transfer(old, new, 1);
        assert_eq!(bus.lost(new), 1);
        sub(&mut bus, new, "e");
        assert!(
            bus.take(new).is_empty(),
            "messages for the old module are gone"
        );
        assert_eq!(bus.lost(old), 0);
    }

    #[test]
    fn kind_clash_is_an_error() {
        let mut bus = Bus::new();
        let decl = |kind| TopicDecl {
            name: "x".into(),
            kind,
            max_len: 8,
        };
        bus.declare(&[decl(TopicKind::State)]).unwrap();
        assert!(bus.declare(&[decl(TopicKind::Event)]).is_err());
    }
}
