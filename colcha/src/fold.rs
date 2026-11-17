use std::collections::{BTreeMap, BTreeSet};

use crate::error::Error;
use crate::fact::{Fact, FactKind};
use crate::native::NativeRegistry;
use crate::path::Path;
use crate::schema::{Constraint, FoldPolicy, Node, NodeKind, Schema, SchemaId};
use crate::stamp::Stamp;
use crate::state::State;
use crate::value::Value;

/// Pure fold: turn a state into a concrete value for `T`.
pub fn fold<T: Schema>(state: &State, registry: &dyn NativeRegistry) -> Result<Value, Error> {
    let expected = SchemaId::new(T::NAME, T::VERSION);
    for fact in state.facts() {
        if fact.schema != expected {
            return Err(Error::Schema {
                expected,
                found: fact.schema.clone(),
            });
        }
    }

    let node = T::schema();
    let tombstones: Vec<&Fact> = state
        .facts()
        .filter(|fact| matches!(fact.kind, FactKind::Tombstone))
        .collect();

    let facts: Vec<Fact> = state
        .facts()
        .filter(|fact| {
            if matches!(fact.kind, FactKind::Tombstone) {
                return false;
            }
            !tombstones.iter().any(|tombstone| {
                tombstone.path.is_prefix_of(&fact.path) && tombstone.stamp >= fact.stamp
            })
        })
        .cloned()
        .collect();

    fold_node(&node, &Path::root(), &facts, registry, None)
}

fn fold_node(
    node: &Node,
    path: &Path,
    facts: &[Fact],
    registry: &dyn NativeRegistry,
    key: Option<&str>,
) -> Result<Value, Error> {
    if !has_facts(facts, path) && key.is_none() {
        if node.optional {
            return Ok(node.default.clone().unwrap_or(Value::Null));
        }
        match node.kind {
            NodeKind::Leaf => {}
            _ => {
                if let Some(default) = &node.default {
                    return Ok(default.clone());
                }
                if matches!(node.kind, NodeKind::Struct | NodeKind::Map)
                    || (node.kind == NodeKind::List && node.key_field.is_some())
                {
                    return Err(Error::fold(
                        path.clone(),
                        "required field has no facts and no default",
                    ));
                }
            }
        }
    }

    match node.kind {
        NodeKind::Struct => fold_struct(node, path, facts, registry, key),
        NodeKind::List => fold_list(node, path, facts, registry),
        NodeKind::Map => fold_map(node, path, facts, registry),
        NodeKind::Leaf => fold_leaf(node, path, facts, None),
        NodeKind::Native => fold_native(node, path, facts, registry),
    }
}

fn fold_struct(
    node: &Node,
    path: &Path,
    facts: &[Fact],
    registry: &dyn NativeRegistry,
    key: Option<&str>,
) -> Result<Value, Error> {
    let mut map = BTreeMap::new();
    for (segment, child) in &node.fields {
        let child_path = path
            .child(segment)
            .map_err(|error| Error::fold(path.clone(), error.to_string()))?;
        let value = if let Some(key) = key {
            if node.key_field.as_deref() == Some(segment.as_str()) {
                Value::String(key.to_string())
            } else {
                fold_node(child, &child_path, facts, registry, None)?
            }
        } else {
            fold_node(child, &child_path, facts, registry, None)?
        };
        map.insert(segment.clone(), value);
    }
    Ok(Value::Map(map))
}

fn fold_list(
    node: &Node,
    path: &Path,
    facts: &[Fact],
    registry: &dyn NativeRegistry,
) -> Result<Value, Error> {
    if node.key_field.is_none() {
        return fold_leaf(node, path, facts, Some(FoldPolicy::Atomic));
    }

    let keys = direct_child_segments(facts, path);
    if keys.is_empty() {
        let direct_candidate = facts.iter().find(|fact| {
            fact.path == *path
                && matches!(&fact.kind, FactKind::Candidate(Value::List(items)) if items.is_empty())
        });
        if direct_candidate.is_some() {
            return Ok(Value::List(Vec::new()));
        }
        if node.optional {
            return Ok(Value::Null);
        }
        return Err(Error::fold(path.clone(), "keyed list has no elements"));
    }

    let element = node
        .element
        .as_deref()
        .ok_or_else(|| Error::fold(path.clone(), "list has no element schema"))?;
    let mut values = Vec::with_capacity(keys.len());
    for key in keys {
        let child_path = path
            .child(&key)
            .map_err(|error| Error::fold(path.clone(), error.to_string()))?;
        values.push(fold_node(
            element,
            &child_path,
            facts,
            registry,
            Some(&key),
        )?);
    }
    Ok(Value::List(values))
}

fn fold_map(
    node: &Node,
    path: &Path,
    facts: &[Fact],
    registry: &dyn NativeRegistry,
) -> Result<Value, Error> {
    let element = node
        .element
        .as_deref()
        .ok_or_else(|| Error::fold(path.clone(), "map has no element schema"))?;
    let mut map = BTreeMap::new();
    for key in direct_child_segments(facts, path) {
        let child_path = path
            .child(&key)
            .map_err(|error| Error::fold(path.clone(), error.to_string()))?;
        map.insert(
            key.clone(),
            fold_node(element, &child_path, facts, registry, None)?,
        );
    }

    let direct: Vec<&Fact> = facts.iter().filter(|fact| fact.path == *path).collect();
    if map.is_empty() && !direct.is_empty() {
        let base = fold_leaf(node, path, facts, Some(FoldPolicy::Lww))?;
        if let Value::Map(base_map) = base {
            return Ok(Value::Map(base_map));
        }
        return Err(Error::fold(path.clone(), "map candidate is not a map"));
    }
    Ok(Value::Map(map))
}

fn fold_native(
    node: &Node,
    path: &Path,
    facts: &[Fact],
    registry: &dyn NativeRegistry,
) -> Result<Value, Error> {
    let direct: Vec<&Fact> = facts.iter().filter(|fact| fact.path == *path).collect();

    let mut names = BTreeSet::new();
    let mut allowed = BTreeSet::new();
    for fact in &direct {
        match &fact.kind {
            FactKind::Candidate(Value::Native { name, .. }) => {
                names.insert(name.clone());
            }
            FactKind::Constraint(Constraint::NativeOneOf(native_names)) => {
                allowed.extend(native_names.iter().cloned());
            }
            _ => {}
        }
    }

    let name = if names.len() > 1 {
        return Err(Error::conflict(
            path.clone(),
            names
                .iter()
                .map(|name| Value::String(name.clone()))
                .collect(),
        ));
    } else if let Some(name) = names.into_iter().next() {
        name
    } else {
        let mut candidates: BTreeSet<String> = if node.natives.is_empty() {
            allowed.clone()
        } else {
            node.natives.clone()
        };
        if !allowed.is_empty() {
            candidates = candidates.intersection(&allowed).cloned().collect();
        }
        match candidates.len() {
            1 => candidates.into_iter().next().expect("one candidate"),
            0 => {
                return Err(Error::native(path.clone(), "<unspecified>"));
            }
            _ => {
                return Err(Error::fold(
                    path.clone(),
                    "native name is ambiguous; write _core.Name",
                ));
            }
        }
    };

    if !node.natives.is_empty() && !node.natives.contains(&name) {
        return Err(Error::native(path.clone(), name));
    }
    if !allowed.is_empty() && !allowed.contains(&name) {
        return Err(Error::native(path.clone(), name));
    }

    let spec = registry
        .spec(&name)
        .ok_or_else(|| Error::native(path.clone(), name.clone()))?;

    if spec.args.kind == NodeKind::Struct {
        for key in direct_child_segments(facts, path) {
            if !spec.args.fields.contains_key(&key) {
                return Err(Error::fold(
                    path.clone(),
                    format!("unknown argument '{key}' for native '{name}'"),
                ));
            }
        }
    } else if !direct_child_segments(facts, path).is_empty() {
        return Err(Error::fold(
            path.clone(),
            format!("native '{name}' does not take named arguments"),
        ));
    }

    let mut arg_facts: Vec<Fact> = facts
        .iter()
        .filter(|fact| fact.path != *path && path.is_prefix_of(&fact.path))
        .cloned()
        .collect();
    for fact in &direct {
        if let FactKind::Candidate(Value::Native { args, .. }) = &fact.kind {
            if let Value::Map(map) = args.as_ref() {
                for (key, value) in map {
                    let child = path
                        .child(key)
                        .map_err(|error| Error::native(path.clone(), format!("{name}: {error}")))?;
                    arg_facts.push(Fact {
                        path: child,
                        stamp: fact.stamp,
                        schema: fact.schema.clone(),
                        kind: FactKind::Candidate(value.clone()),
                    });
                }
            }
        }
    }

    let mut args = fold_node(&spec.args, path, &arg_facts, registry, None)?;
    if let Some(coerce) = spec.coerce {
        args = coerce(&args).ok_or_else(|| {
            Error::fold(
                path.clone(),
                format!("coerce rejected the arguments of native '{name}'"),
            )
        })?;
    }

    Ok(Value::Native {
        name,
        args: Box::new(args),
    })
}

fn fold_leaf(
    node: &Node,
    path: &Path,
    facts: &[Fact],
    forced_policy: Option<FoldPolicy>,
) -> Result<Value, Error> {
    let direct: Vec<&Fact> = facts.iter().filter(|fact| fact.path == *path).collect();
    let mut constraints = node.constraints.clone();
    for fact in &direct {
        if let FactKind::Constraint(constraint) = &fact.kind {
            constraints.push(constraint.clone());
        }
    }
    check_constraints(&constraints, path)?;

    let mut candidates: Vec<(Stamp, Value)> = Vec::new();
    let mut defaults: Vec<(Stamp, Value)> = Vec::new();
    for fact in &direct {
        match &fact.kind {
            FactKind::Candidate(value) => candidates.push((fact.stamp, value.clone())),
            FactKind::Default(value) => defaults.push((fact.stamp, value.clone())),
            FactKind::Constraint(_) | FactKind::Tombstone => {}
        }
    }

    let policy = forced_policy.unwrap_or(node.policy);
    if let Some(value) = select(policy, &candidates, &constraints, path)? {
        return Ok(value);
    }
    if let Some(value) = select(policy, &defaults, &constraints, path)? {
        return Ok(value);
    }
    if let Some(default) = &node.default {
        if satisfies(default, &constraints, path)? {
            return Ok(default.clone());
        }
    }
    if node.optional {
        return Ok(Value::Null);
    }
    Err(Error::fold(
        path.clone(),
        "no valid candidate and no valid default",
    ))
}

fn select(
    policy: FoldPolicy,
    entries: &[(Stamp, Value)],
    constraints: &[Constraint],
    path: &Path,
) -> Result<Option<Value>, Error> {
    let mut valid: Vec<&(Stamp, Value)> = Vec::new();
    for entry in entries {
        if satisfies(&entry.1, constraints, path)? {
            valid.push(entry);
        }
    }
    if valid.is_empty() {
        return Ok(None);
    }

    match policy {
        FoldPolicy::Lww | FoldPolicy::Atomic | FoldPolicy::Native => {
            let max_stamp = valid
                .iter()
                .map(|(stamp, _)| *stamp)
                .max()
                .expect("valid is not empty");
            let values: BTreeSet<Value> = valid
                .iter()
                .filter(|(stamp, _)| *stamp == max_stamp)
                .map(|(_, value)| value.clone())
                .collect();
            if values.len() > 1 {
                return Err(Error::conflict(path.clone(), values.into_iter().collect()));
            }
            Ok(values.into_iter().next())
        }
        FoldPolicy::Flat => {
            let values: BTreeSet<Value> = valid.iter().map(|(_, value)| value.clone()).collect();
            if values.len() > 1 {
                return Err(Error::conflict(path.clone(), values.into_iter().collect()));
            }
            Ok(values.into_iter().next())
        }
        FoldPolicy::Union => {
            let mut values = BTreeSet::new();
            for (_, value) in valid {
                match value {
                    Value::List(items) => values.extend(items.iter().cloned()),
                    other => {
                        values.insert(other.clone());
                    }
                }
            }
            Ok(Some(Value::List(values.into_iter().collect())))
        }
    }
}

fn satisfies(value: &Value, constraints: &[Constraint], path: &Path) -> Result<bool, Error> {
    for constraint in constraints {
        match constraint.satisfies(value) {
            Ok(false) => return Ok(false),
            Ok(true) => {}
            Err(reason) => return Err(Error::fold(path.clone(), reason)),
        }
    }
    Ok(true)
}

fn check_constraints(constraints: &[Constraint], path: &Path) -> Result<(), Error> {
    for (index, left) in constraints.iter().enumerate() {
        for right in &constraints[index + 1..] {
            if let Some(reason) = left.conflict_with(right) {
                return Err(Error::fold(path.clone(), reason));
            }
        }
    }
    Ok(())
}

fn has_facts(facts: &[Fact], path: &Path) -> bool {
    facts.iter().any(|fact| path.is_prefix_of(&fact.path))
}

fn direct_child_segments(facts: &[Fact], path: &Path) -> BTreeSet<String> {
    let mut segments = BTreeSet::new();
    for fact in facts {
        if let Some(rest) = fact.path.strip_prefix(path) {
            if let Some(first) = rest.segments().first() {
                segments.insert(first.clone());
            }
        }
    }
    segments
}
