use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::error::Error;
use crate::fact::{Fact, FactKind};
use crate::path::Path;
use crate::schema::{FoldPolicy, NodeKind, Schema, SchemaId};
use crate::value::Value;

/// Replicable configuration state: a set of immutable facts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct State {
    facts: BTreeSet<Fact>,
}

impl State {
    pub fn empty() -> State {
        State {
            facts: BTreeSet::new(),
        }
    }

    /// Return a new state with one fact after validating it against the schema.
    ///
    /// The receiver stays unchanged. Adding an equal fact is idempotent.
    pub fn with<T: Schema>(&self, fact: Fact) -> Result<State, Error> {
        let expected = SchemaId::new(T::NAME, T::VERSION);
        if fact.schema != expected {
            return Err(Error::Schema {
                expected,
                found: fact.schema,
            });
        }
        let root = T::schema();
        let node = root
            .lookup(&fact.path)
            .map_err(|reason| Error::path(fact.path.to_string(), reason))?;
        match &fact.kind {
            FactKind::Candidate(_) | FactKind::Default(_) => match node.kind {
                NodeKind::Struct => {
                    return Err(Error::path(
                        fact.path.to_string(),
                        "a struct is written through its fields",
                    ));
                }
                NodeKind::List if node.key_field.is_some() => {
                    let empty = matches!(
                        &fact.kind,
                        FactKind::Candidate(Value::List(items)) if items.is_empty()
                    );
                    if !empty {
                        return Err(Error::path(
                            fact.path.to_string(),
                            "a keyed list is written through its keys",
                        ));
                    }
                }
                _ => {}
            },
            FactKind::Constraint(_) => {
                if !matches!(node.kind, NodeKind::Leaf | NodeKind::Raw | NodeKind::Native) {
                    return Err(Error::path(
                        fact.path.to_string(),
                        "a constraint applies to a leaf only",
                    ));
                }
            }
            FactKind::Tombstone => {}
        }
        let mut facts = self.facts.clone();
        facts.insert(fact);
        Ok(State { facts })
    }

    /// Total merge: set union. Commutative, associative and idempotent.
    pub fn union(&self, other: &State) -> State {
        let mut facts = self.facts.clone();
        facts.extend(other.facts.iter().cloned());
        State { facts }
    }

    /// Remove facts that cannot change the current fold.
    pub fn compact<T: Schema>(&self) -> State {
        let expected = SchemaId::new(T::NAME, T::VERSION);
        let root = T::schema();
        let same_schema: Vec<&Fact> = self
            .facts
            .iter()
            .filter(|fact| fact.schema == expected)
            .collect();
        let tombstones: Vec<&Fact> = same_schema
            .iter()
            .copied()
            .filter(|fact| matches!(fact.kind, FactKind::Tombstone))
            .collect();

        let mut grouped: BTreeMap<Path, Vec<&Fact>> = BTreeMap::new();
        for fact in &same_schema {
            if matches!(fact.kind, FactKind::Tombstone) {
                continue;
            }
            let absorbed = tombstones.iter().any(|tombstone| {
                tombstone.path.is_prefix_of(&fact.path) && tombstone.stamp >= fact.stamp
            });
            if !absorbed {
                grouped.entry(fact.path.clone()).or_default().push(fact);
            }
        }

        let mut compacted = State::empty();
        for fact in &self.facts {
            if fact.schema != expected {
                compacted.facts.insert((*fact).clone());
            }
        }
        for tombstone in tombstones {
            compacted.facts.insert(tombstone.clone());
        }

        for (path, facts) in grouped {
            let Ok(node) = root.lookup(&path) else {
                for fact in facts {
                    compacted.facts.insert(fact.clone());
                }
                continue;
            };
            if !matches!(node.kind, NodeKind::Leaf | NodeKind::Raw | NodeKind::Union) {
                for fact in facts {
                    compacted.facts.insert(fact.clone());
                }
                continue;
            }
            let mut constraints = node.constraints.clone();
            for fact in &facts {
                if let FactKind::Constraint(constraint) = &fact.kind {
                    constraints.push(constraint.clone());
                }
            }
            let valid = |value: &Value| -> bool {
                constraints
                    .iter()
                    .all(|constraint| constraint.satisfies(value).unwrap_or(false))
            };

            let mut kept: Vec<&Fact> = Vec::new();
            for fact in &facts {
                match &fact.kind {
                    FactKind::Constraint(_) => kept.push(fact),
                    FactKind::Candidate(value) | FactKind::Default(value) => {
                        if !valid(value) {
                            continue;
                        }
                        match node.policy {
                            FoldPolicy::Lww | FoldPolicy::Atomic => kept.push(fact),
                            FoldPolicy::Union => kept.push(fact),
                            FoldPolicy::Flat | FoldPolicy::Native => kept.push(fact),
                        }
                    }
                    FactKind::Tombstone => {}
                }
            }

            if matches!(node.policy, FoldPolicy::Lww | FoldPolicy::Atomic) {
                let max_candidate = kept
                    .iter()
                    .filter(|fact| matches!(fact.kind, FactKind::Candidate(_)))
                    .map(|fact| fact.stamp)
                    .max();
                if let Some(max_stamp) = max_candidate {
                    kept.retain(|fact| {
                        !matches!(fact.kind, FactKind::Candidate(_)) || fact.stamp == max_stamp
                    });
                }
                let max_default = kept
                    .iter()
                    .filter(|fact| matches!(fact.kind, FactKind::Default(_)))
                    .map(|fact| fact.stamp)
                    .max();
                if let Some(max_stamp) = max_default {
                    kept.retain(|fact| {
                        !matches!(fact.kind, FactKind::Default(_)) || fact.stamp == max_stamp
                    });
                }
            }

            for fact in kept {
                compacted.facts.insert(fact.clone());
            }
        }

        compacted
    }

    pub fn facts(&self) -> impl Iterator<Item = &Fact> {
        self.facts.iter()
    }

    pub fn len(&self) -> usize {
        self.facts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }
}

impl FromIterator<Fact> for State {
    fn from_iter<T: IntoIterator<Item = Fact>>(iter: T) -> State {
        State {
            facts: iter.into_iter().collect(),
        }
    }
}
