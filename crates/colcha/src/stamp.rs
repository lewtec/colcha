use std::fmt;

use serde::Serialize;

/// Opaque host identity for one replica.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct NodeId([u8; 16]);

impl NodeId {
    pub const ZERO: NodeId = NodeId([0; 16]);

    pub fn new(bytes: [u8; 16]) -> NodeId {
        NodeId(bytes)
    }

    pub fn from_u128(value: u128) -> NodeId {
        NodeId(value.to_be_bytes())
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Total order over writes: logical clock first, node id as tie breaker.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Stamp {
    pub clock: u64,
    pub node: NodeId,
}

impl Stamp {
    pub fn new(clock: u64, node: NodeId) -> Stamp {
        Stamp { clock, node }
    }
}

impl fmt::Display for Stamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.clock, self.node)
    }
}
