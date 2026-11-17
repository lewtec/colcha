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
            NodeKind::Leaf | NodeKind::Raw | NodeKind::Union => {}
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
        NodeKind::Union => fold_union(node, path, facts),
        NodeKind::Leaf => fold_leaf(node, path, facts, None),
        NodeKind::Raw => fold_leaf(node, path, facts, None),
        NodeKind::Native => fold_native(node, path, facts, registry),
    }
}

fn fold_union(node: &Node, path: &Path, facts: &[Fact]) -> Result<Value, Error> {
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

    let mut selected = select(node.policy, &candidates, &constraints, path)?;
    if selected.is_none() {
        selected = select(node.policy, &defaults, &constraints, path)?;
    }
    if selected.is_none() {
        if let Some(default) = &node.default {
            if satisfies(default, &constraints, path)? {
                selected = Some(default.clone());
            }
        }
    }

    match selected {
        Some(value) => complete_union(node, path, Some(&value)),
        None if node.optional => Ok(Value::Null),
        None => Err(Error::fold(
            path.clone(),
            "no valid variant and no valid default",
        )),
    }
}

/// Complete a value that lives at one atomic path.
///
/// A tagged union stores its whole value in one fact, so the fold validates and
/// fills the fields of the selected variant here instead of walking child paths.
fn complete_value(node: &Node, path: &Path, value: Option<&Value>) -> Result<Value, Error> {
    match node.kind {
        NodeKind::Struct => complete_struct(node, path, value, None),
        NodeKind::List => {
            if node.key_field.is_some() {
                complete_keyed_list(node, path, value)
            } else {
                complete_list(node, path, value)
            }
        }
        NodeKind::Map => complete_map(node, path, value),
        NodeKind::Union => complete_union(node, path, value),
        NodeKind::Leaf => complete_leaf(node, path, value),
        NodeKind::Raw => match value {
            Some(value) => Ok(value.clone()),
            None => node
                .default
                .clone()
                .ok_or_else(|| Error::fold(path.clone(), "required field has no value")),
        },
        NodeKind::Native => complete_native(node, path, value),
    }
}

fn complete_struct(
    node: &Node,
    path: &Path,
    value: Option<&Value>,
    key: Option<&str>,
) -> Result<Value, Error> {
    let Some(value) = value else {
        match &node.default {
            Some(default) if !default.is_null() => {
                return complete_struct(node, path, Some(default), key);
            }
            _ => {
                return Err(Error::fold(
                    path.clone(),
                    "required object has no value and no default",
                ));
            }
        }
    };
    if value.is_null() {
        if node.optional {
            return Ok(Value::Null);
        }
        match &node.default {
            Some(default) if !default.is_null() => {
                return complete_struct(node, path, Some(default), key);
            }
            _ => {
                return Err(Error::fold(path.clone(), "required object has no value"));
            }
        }
    }
    let Some(map) = value.as_map() else {
        return Err(Error::fold(
            path.clone(),
            format!("expected object, found {}", value.type_name()),
        ));
    };
    if node.key_field.is_some() && key.is_none() {
        return Err(Error::fold(path.clone(), "a keyed object needs a path key"));
    }

    let mut result = BTreeMap::new();
    for (segment, child) in &node.fields {
        let child_path = path
            .child(segment)
            .map_err(|error| Error::fold(path.clone(), error.to_string()))?;
        let child_value = if node.key_field.as_deref() == Some(segment.as_str()) {
            let key = key.expect("checked above");
            Value::String(key.to_string())
        } else {
            complete_value(child, &child_path, map.get(segment))?
        };
        result.insert(segment.clone(), child_value);
    }
    for field in map.keys() {
        if !node.fields.contains_key(field) {
            return Err(Error::fold(
                path.clone(),
                format!("unknown field '{field}'"),
            ));
        }
    }
    Ok(Value::Map(result))
}

fn complete_leaf(node: &Node, path: &Path, value: Option<&Value>) -> Result<Value, Error> {
    let resolved = match value {
        Some(value) if !value.is_null() => {
            if satisfies(value, &node.constraints, path)? {
                value.clone()
            } else {
                match &node.default {
                    Some(default) if satisfies(default, &node.constraints, path)? => {
                        default.clone()
                    }
                    _ => {
                        return Err(Error::fold(
                            path.clone(),
                            format!(
                                "value of type {} does not satisfy the field constraints",
                                value.type_name()
                            ),
                        ));
                    }
                }
            }
        }
        Some(_) => {
            if node.optional {
                return Ok(Value::Null);
            }
            match &node.default {
                Some(default) => default.clone(),
                None => {
                    return Err(Error::fold(path.clone(), "required field has no value"));
                }
            }
        }
        None => match &node.default {
            Some(default) => default.clone(),
            None => {
                return Err(Error::fold(path.clone(), "required field has no value"));
            }
        },
    };

    if !satisfies(&resolved, &node.constraints, path)? {
        return Err(Error::fold(
            path.clone(),
            "default does not satisfy the field constraints",
        ));
    }
    Ok(resolved)
}

fn complete_list(node: &Node, path: &Path, value: Option<&Value>) -> Result<Value, Error> {
    let Some(value) = value else {
        return missing(node, path);
    };
    if value.is_null() {
        if node.optional {
            return Ok(Value::Null);
        }
        return missing(node, path);
    }
    let Some(items) = value.as_list() else {
        return Err(Error::fold(
            path.clone(),
            format!("expected list, found {}", value.type_name()),
        ));
    };
    let element = node
        .element
        .as_deref()
        .ok_or_else(|| Error::fold(path.clone(), "list has no element schema"))?;
    let mut result = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let child = path
            .child(&index.to_string())
            .map_err(|error| Error::fold(path.clone(), error.to_string()))?;
        result.push(complete_value(element, &child, Some(item))?);
    }
    Ok(Value::List(result))
}

fn complete_keyed_list(node: &Node, path: &Path, value: Option<&Value>) -> Result<Value, Error> {
    let Some(value) = value else {
        return missing(node, path);
    };
    if value.is_null() {
        if node.optional {
            return Ok(Value::Null);
        }
        return missing(node, path);
    }
    let Some(items) = value.as_list() else {
        return Err(Error::fold(
            path.clone(),
            format!("expected list, found {}", value.type_name()),
        ));
    };
    let element = node
        .element
        .as_deref()
        .ok_or_else(|| Error::fold(path.clone(), "list has no element schema"))?;
    let key_field = node
        .key_field
        .as_deref()
        .ok_or_else(|| Error::fold(path.clone(), "keyed list has no key field"))?;

    let mut entries = BTreeMap::new();
    for item in items {
        let Some(map) = item.as_map() else {
            return Err(Error::fold(path.clone(), "a keyed list contains objects"));
        };
        let key = match map.get(key_field) {
            Some(Value::String(key)) => key.clone(),
            _ => {
                return Err(Error::fold(
                    path.clone(),
                    format!("an element needs a string '{key_field}'"),
                ));
            }
        };
        let child = path
            .child(&key)
            .map_err(|error| Error::fold(path.clone(), error.to_string()))?;
        let completed = complete_struct(element, &child, Some(item), Some(&key))?;
        if entries.insert(key.clone(), completed).is_some() {
            return Err(Error::fold(
                path.clone(),
                format!("duplicate key '{key}' in list"),
            ));
        }
    }
    Ok(Value::List(entries.into_values().collect()))
}

fn complete_map(node: &Node, path: &Path, value: Option<&Value>) -> Result<Value, Error> {
    let Some(value) = value else {
        return missing(node, path);
    };
    if value.is_null() {
        if node.optional {
            return Ok(Value::Null);
        }
        return missing(node, path);
    }
    let Some(map) = value.as_map() else {
        return Err(Error::fold(
            path.clone(),
            format!("expected object, found {}", value.type_name()),
        ));
    };
    let element = node
        .element
        .as_deref()
        .ok_or_else(|| Error::fold(path.clone(), "map has no element schema"))?;
    let mut result = BTreeMap::new();
    for (key, item) in map {
        let child = path
            .child(key)
            .map_err(|error| Error::fold(path.clone(), error.to_string()))?;
        result.insert(key.clone(), complete_value(element, &child, Some(item))?);
    }
    Ok(Value::Map(result))
}

fn complete_union(node: &Node, path: &Path, value: Option<&Value>) -> Result<Value, Error> {
    let Some(value) = value else {
        let Some(default) = &node.default else {
            if node.optional {
                return Ok(Value::Null);
            }
            return Err(Error::fold(
                path.clone(),
                "required tagged union has no value",
            ));
        };
        return complete_union(node, path, Some(default));
    };
    if value.is_null() {
        if node.optional {
            return Ok(Value::Null);
        }
        if let Some(default) = &node.default {
            if !default.is_null() {
                return complete_union(node, path, Some(default));
            }
        }
        return Err(Error::fold(
            path.clone(),
            "required tagged union has no value",
        ));
    }
    let Some(map) = value.as_map() else {
        return Err(Error::fold(
            path.clone(),
            format!("a tagged union is an object, found {}", value.type_name()),
        ));
    };
    let Some(tag) = node.tag.as_deref() else {
        return Err(Error::fold(path.clone(), "tagged union has no tag field"));
    };
    let tag_value = map
        .get(tag)
        .ok_or_else(|| Error::fold(path.clone(), format!("missing tag field '{tag}'")))?;
    let Value::String(variant_name) = tag_value else {
        return Err(Error::fold(
            path.clone(),
            format!("tag '{tag}' is a string, found {}", tag_value.type_name()),
        ));
    };
    let Some(variant) = node.variants.get(variant_name) else {
        return Err(Error::fold(
            path.clone(),
            format!("unknown variant '{variant_name}'"),
        ));
    };

    let mut input = map.clone();
    input.remove(tag);
    let completed = complete_struct(variant, path, Some(&Value::Map(input)), None)?;
    let Value::Map(mut result) = completed else {
        return Err(Error::fold(
            path.clone(),
            "a tagged union variant is an object",
        ));
    };
    result.insert(tag.to_string(), Value::String(variant_name.clone()));
    Ok(Value::Map(result))
}

fn complete_native(node: &Node, path: &Path, value: Option<&Value>) -> Result<Value, Error> {
    match value {
        Some(native @ Value::Native { .. }) => Ok(native.clone()),
        Some(other) if other.is_null() && node.optional => Ok(Value::Null),
        Some(other) => Err(Error::fold(
            path.clone(),
            format!("expected native value, found {}", other.type_name()),
        )),
        None => Err(Error::fold(
            path.clone(),
            "required native field has no value",
        )),
    }
}

fn missing(node: &Node, path: &Path) -> Result<Value, Error> {
    if node.optional {
        return Ok(Value::Null);
    }
    node.default
        .clone()
        .ok_or_else(|| Error::fold(path.clone(), "required field has no value"))
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
