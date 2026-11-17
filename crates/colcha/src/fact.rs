use serde::Serialize;

use crate::path::Path;
use crate::schema::{Constraint, SchemaId};
use crate::stamp::Stamp;
use crate::value::Value;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum FactKind {
    Candidate(Value),
    Default(Value),
    Constraint(Constraint),
    Tombstone,
}

/// One immutable contribution to the configuration state.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Fact {
    pub path: Path,
    pub stamp: Stamp,
    pub schema: SchemaId,
    pub kind: FactKind,
}

impl Fact {
    pub fn candidate(path: Path, stamp: Stamp, schema: SchemaId, value: Value) -> Fact {
        Fact {
            path,
            stamp,
            schema,
            kind: FactKind::Candidate(value),
        }
    }

    pub fn default_value(path: Path, stamp: Stamp, schema: SchemaId, value: Value) -> Fact {
        Fact {
            path,
            stamp,
            schema,
            kind: FactKind::Default(value),
        }
    }

    pub fn constraint(path: Path, stamp: Stamp, schema: SchemaId, constraint: Constraint) -> Fact {
        Fact {
            path,
            stamp,
            schema,
            kind: FactKind::Constraint(constraint),
        }
    }

    pub fn tombstone(path: Path, stamp: Stamp, schema: SchemaId) -> Fact {
        Fact {
            path,
            stamp,
            schema,
            kind: FactKind::Tombstone,
        }
    }
}
