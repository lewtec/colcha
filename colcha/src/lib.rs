//! Composable typed configuration from replicable facts.
//!
//! The state is a set of immutable facts. `State::union` is set union.
//! `fold` is pure. `resolve` materializes a Rust value; it is the only
//! operation that may run a host effect.

extern crate self as colcha;

mod error;
mod fact;
mod fold;
mod materialize;
mod native;
mod node_of;
mod parse;
mod path;
mod resolve;
mod schema;
mod stamp;
mod state;
mod value;

pub use colcha_derive::{Materialize, Schema};

pub use crate::error::Error;
pub use crate::fact::{Fact, FactKind};
pub use crate::fold::fold;
pub use crate::materialize::Materialize;
pub use crate::native::{EmptyHost, NativeHandle, NativeHost, NativeRegistry, NativeType};
pub use crate::node_of::NodeOf;
pub use crate::parse::parse;
pub use crate::path::Path;
pub use crate::resolve::resolve;
pub use crate::schema::{Constraint, FoldPolicy, Native, Node, NodeKind, Schema, SchemaId, scalar};
pub use crate::stamp::{NodeId, Stamp};
pub use crate::state::State;
pub use crate::value::{Float, Raw, Value};

/// Common imports for deriving and using a schema.
pub mod prelude {
    pub use crate::{Materialize, NodeOf, Raw, Schema, State};
}
