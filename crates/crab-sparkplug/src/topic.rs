//! Sparkplug topic names: `spBv1.0/<group>/<message type>/<node>[/<device>]`.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    NBirth,
    NDeath,
    DBirth,
    DDeath,
    NData,
    DData,
    NCmd,
    DCmd,
}

impl MessageType {
    pub fn name(self) -> &'static str {
        match self {
            Self::NBirth => "NBIRTH",
            Self::NDeath => "NDEATH",
            Self::DBirth => "DBIRTH",
            Self::DDeath => "DDEATH",
            Self::NData => "NDATA",
            Self::DData => "DDATA",
            Self::NCmd => "NCMD",
            Self::DCmd => "DCMD",
        }
    }
}

/// A topic of the namespace `spBv1.0`.
#[derive(Debug, Clone, Copy)]
pub struct Topic<'a> {
    pub group: &'a str,
    pub kind: MessageType,
    pub node: &'a str,
    pub device: Option<&'a str>,
}

impl fmt::Display for Topic<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "spBv1.0/{}/{}/{}",
            self.group,
            self.kind.name(),
            self.node
        )?;
        if let Some(d) = self.device {
            write!(f, "/{d}")?;
        }
        Ok(())
    }
}
