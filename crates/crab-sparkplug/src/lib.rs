//! MQTT Sparkplug B 3.0 (ISO/IEC 20237) without any network code: the protobuf payload,
//! the topic names and the message sequence of an edge node (`specs/pubsub-bus`, step 2).
//!
//! The crate only builds `(topic, bytes)` pairs; the MQTT client (and its will message) belongs to
//! the caller. Field numbers and data type codes follow `sparkplug_b.proto` of Eclipse Tahu.

mod payload;
mod topic;

pub use payload::{DataType, Metric, Payload, Value};
pub use topic::{MessageType, Topic};

/// Name of the metric that carries the birth/death session number.
pub const BD_SEQ: &str = "bdSeq";
/// Name of the metric a host sets to ask the node for a new birth.
pub const REBIRTH: &str = "Node Control/Rebirth";

/// A message ready for the MQTT client: QoS 0, not retained (Sparkplug rule for all but `STATE`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub topic: String,
    pub payload: Vec<u8>,
}

/// Birth metrics for a list of `(name, current value)`: aliases are the positions plus one, so a
/// data message can address metric `i` as alias `i + 1` (the same order as the signal table).
pub fn birth_metrics(signals: &[(String, Value)]) -> Vec<Metric> {
    signals
        .iter()
        .enumerate()
        .map(|(i, (name, value))| Metric::birth(name, i as u64 + 1, value.clone()))
        .collect()
}

/// An edge node: the group and node ids, the session number `bdSeq` and the payload sequence `seq`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    group: String,
    node: String,
    bd_seq: u64,
    seq: u8,
}

impl Node {
    pub fn new(group: &str, node: &str) -> Self {
        Self {
            group: group.to_owned(),
            node: node.to_owned(),
            bd_seq: 0,
            seq: 0,
        }
    }

    pub fn bd_seq(&self) -> u64 {
        self.bd_seq
    }

    /// The next MQTT session: `bdSeq` grows by one (it wraps at 256 as the standard asks).
    pub fn next_session(&mut self) {
        self.bd_seq = (self.bd_seq + 1) % 256;
    }

    fn topic(&self, kind: MessageType, device: Option<&str>) -> String {
        Topic {
            group: &self.group,
            kind,
            node: &self.node,
            device,
        }
        .to_string()
    }

    fn bd_metric(&self) -> Metric {
        Metric::named(BD_SEQ, Value::Int64(self.bd_seq as i64))
    }

    /// The `NDEATH` message to register as the MQTT will before connecting: only `bdSeq`, no `seq`.
    pub fn will(&self) -> Message {
        let payload = Payload {
            timestamp: None,
            metrics: vec![self.bd_metric()],
            seq: None,
        };
        Message {
            topic: self.topic(MessageType::NDeath, None),
            payload: payload.encode(),
        }
    }

    fn sequenced(&mut self, ts: u64, metrics: Vec<Metric>, reset: bool) -> Payload {
        if reset {
            self.seq = 0;
        }
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1);
        Payload {
            timestamp: Some(ts),
            metrics,
            seq: Some(u64::from(seq)),
        }
    }

    /// `NBIRTH`: `seq` restarts at 0; the standard metrics `bdSeq` and `Node Control/Rebirth` come first,
    /// then `metrics` (each with name, alias, type and current value).
    pub fn birth(&mut self, ts: u64, metrics: Vec<Metric>) -> Message {
        let mut all = vec![self.bd_metric(), Metric::named(REBIRTH, Value::Bool(false))];
        all.extend(metrics);
        Message {
            topic: self.topic(MessageType::NBirth, None),
            payload: self.sequenced(ts, all, true).encode(),
        }
    }

    /// `NDATA`: changed metrics, normally addressed by alias only.
    pub fn data(&mut self, ts: u64, metrics: Vec<Metric>) -> Message {
        Message {
            topic: self.topic(MessageType::NData, None),
            payload: self.sequenced(ts, metrics, false).encode(),
        }
    }

    /// `DBIRTH` of a device of this node (the sequence continues the node's).
    pub fn device_birth(&mut self, device: &str, ts: u64, metrics: Vec<Metric>) -> Message {
        Message {
            topic: self.topic(MessageType::DBirth, Some(device)),
            payload: self.sequenced(ts, metrics, false).encode(),
        }
    }

    /// `DDATA` of a device.
    pub fn device_data(&mut self, device: &str, ts: u64, metrics: Vec<Metric>) -> Message {
        Message {
            topic: self.topic(MessageType::DData, Some(device)),
            payload: self.sequenced(ts, metrics, false).encode(),
        }
    }

    /// `DDEATH`: the device is gone, the node stays.
    pub fn device_death(&mut self, device: &str, ts: u64) -> Message {
        Message {
            topic: self.topic(MessageType::DDeath, Some(device)),
            payload: self.sequenced(ts, Vec::new(), false).encode(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `bdSeq` metric = name (field 1), datatype Int64 = 4 (field 4), long_value (field 11).
    const BDSEQ_5: [u8; 11] = [
        0x0A, 0x05, b'b', b'd', b'S', b'e', b'q', 0x20, 0x04, 0x58, 0x05,
    ];

    #[test]
    fn will_is_ndeath_with_only_bdseq() {
        let mut node = Node::new("Plant", "Esp1");
        for _ in 0..5 {
            node.next_session();
        }
        let m = node.will();
        assert_eq!(m.topic, "spBv1.0/Plant/NDEATH/Esp1");
        let mut want = vec![0x12, BDSEQ_5.len() as u8];
        want.extend(BDSEQ_5);
        assert_eq!(m.payload, want, "payload: one metric, no timestamp, no seq");
    }

    #[test]
    fn birth_starts_with_bdseq_and_rebirth_and_resets_seq() {
        let mut node = Node::new("G", "N");
        node.data(1, vec![]); // seq 0, then 1
        let m = node.birth(1000, vec![Metric::birth("t", 1, Value::Bool(true))]);
        assert_eq!(m.topic, "spBv1.0/G/NBIRTH/N");
        let p = &m.payload;
        // timestamp 1000 = varint E8 07 (field 1)
        assert_eq!(&p[..3], &[0x08, 0xE8, 0x07]);
        // metric 1: bdSeq (value 0)
        assert_eq!(
            &p[3..16],
            &[0x12, 0x0B, 0x0A, 0x05, b'b', b'd', b'S', b'e', b'q', 0x20, 0x04, 0x58, 0x00]
        );
        // metric 2: "Node Control/Rebirth", Boolean (11), false
        let name = REBIRTH.as_bytes();
        let mut rebirth = vec![0x12, (2 + name.len() + 2 + 2) as u8, 0x0A, name.len() as u8];
        rebirth.extend(name);
        rebirth.extend([0x20, 0x0B, 0x70, 0x00]);
        assert_eq!(&p[16..16 + rebirth.len()], &rebirth[..]);
        // metric 3: name "t", alias 1, Boolean, true; then seq = 0
        let tail = &p[16 + rebirth.len()..];
        assert_eq!(
            tail,
            &[0x12, 0x09, 0x0A, 0x01, b't', 0x10, 0x01, 0x20, 0x0B, 0x70, 0x01, 0x18, 0x00]
        );
    }

    #[test]
    fn data_by_alias_and_sequence_wraps_at_256() {
        let mut node = Node::new("G", "N");
        node.birth(1, vec![]); // seq 0
        let mut last = Vec::new();
        for _ in 0..256 {
            last = node
                .data(2, vec![Metric::alias(1, Value::Int32(-1))])
                .payload;
        }
        // 256 messages after the birth: seq goes 1..=255, then wraps to 0 on the 256th
        assert_eq!(*last.last().unwrap(), 0x00);
        assert_eq!(*last.get(last.len() - 2).unwrap(), 0x18);
        // metric: alias 1 (field 2), Int32 = 3, int_value -1 as u32 varint (field 10)
        assert!(last
            .windows(11)
            .any(|w| w == [0x10, 0x01, 0x20, 0x03, 0x50, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x18]));
    }

    #[test]
    fn device_messages_use_the_device_topic_and_the_node_sequence() {
        let mut node = Node::new("G", "N");
        node.birth(1, vec![]);
        let m = node.device_birth("D1", 2, vec![]);
        assert_eq!(m.topic, "spBv1.0/G/DBIRTH/N/D1");
        assert_eq!(&m.payload[m.payload.len() - 2..], &[0x18, 0x01]);
        assert_eq!(
            node.device_data("D1", 3, vec![]).topic,
            "spBv1.0/G/DDATA/N/D1"
        );
        assert_eq!(node.device_death("D1", 4).topic, "spBv1.0/G/DDEATH/N/D1");
    }

    #[test]
    fn aliases_follow_the_order_of_the_signal_table() {
        let m = birth_metrics(&[
            ("pump.out".to_owned(), Value::Bool(false)),
            ("t.value".to_owned(), Value::Float(21.5)),
        ]);
        assert_eq!(m[0].alias, Some(1));
        assert_eq!(m[1].alias, Some(2));
        assert_eq!(m[1].name.as_deref(), Some("t.value"));
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The master firmware (`spdump 1000` on a board with `examples/sparkplug`) wrote these bytes
    /// in C++; the encoder here must give the same ones.
    #[test]
    fn matches_the_bytes_of_the_master_firmware() {
        let mut node = Node::new("CrabLogic", "Esp1");
        assert_eq!(hex(&node.will().payload), "120b0a05626453657120045800");
        let birth = node.birth(
            1000,
            vec![
                Metric::birth("Temperature", 1, Value::Float(21.5)),
                Metric::birth("Hot", 2, Value::Bool(false)),
            ],
        );
        assert_eq!(
            hex(&birth.payload),
            "08e807120b0a05626453657120045800121a0a144e6f646520436f6e74726f6c2f5265626972746820\
             0b700012160a0b54656d706572617475726510012009650000ac41120b0a03486f741002200b70001800"
        );
        let data = node.data(
            1000,
            vec![
                Metric::alias(1, Value::Float(21.5)),
                Metric::alias(2, Value::Bool(false)),
            ],
        );
        assert_eq!(
            hex(&data.payload),
            "08e807120910012009650000ac4112061002200b70001801"
        );
    }

    #[test]
    fn bdseq_wraps_at_256() {
        let mut node = Node::new("G", "N");
        for _ in 0..256 {
            node.next_session();
        }
        assert_eq!(node.bd_seq(), 0);
    }
}
