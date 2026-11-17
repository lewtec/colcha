use crate::error::Error;
use crate::fact::{Fact, FactKind};
use crate::path::Path;
use crate::schema::{Constraint, Node, NodeKind, SchemaId};
use crate::stamp::Stamp;
use crate::value::Value;

/// A concrete or partially constrained expression, ready to become facts.
#[derive(Default)]
pub(crate) struct Expr {
    pub(crate) candidates: Vec<Value>,
    pub(crate) defaults: Vec<Value>,
    pub(crate) constraints: Vec<Constraint>,
    pub(crate) natives: Vec<String>,
}

impl Expr {
    pub(crate) fn is_pure_values(&self) -> bool {
        self.constraints.is_empty() && self.natives.is_empty()
    }

    pub(crate) fn is_pure_natives(&self) -> bool {
        self.constraints.is_empty()
            && !self.natives.is_empty()
            && self.defaults.is_empty()
            && self
                .candidates
                .iter()
                .all(|value| matches!(value, Value::Native { .. }))
    }

    pub(crate) fn merge(mut self, other: Expr) -> Expr {
        self.candidates.extend(other.candidates);
        self.defaults.extend(other.defaults);
        self.constraints.extend(other.constraints);
        self.natives.extend(other.natives);
        self
    }
}

#[derive(Clone, Copy)]
pub(crate) enum EmitKind {
    Candidate,
    Default,
}

/// Schema-aware fact emitter shared by the concrete parser and the evaluator.
pub(crate) struct Emitter<'a> {
    root: &'a Node,
    schema: SchemaId,
    stamp: Stamp,
    facts: Vec<Fact>,
}

impl<'a> Emitter<'a> {
    pub(crate) fn new(root: &'a Node, schema: SchemaId, stamp: Stamp) -> Emitter<'a> {
        Emitter {
            root,
            schema,
            stamp,
            facts: Vec::new(),
        }
    }

    pub(crate) fn into_facts(self) -> Vec<Fact> {
        self.facts
    }

    pub(crate) fn emit_expr(&mut self, path: &Path, node: &Node, expr: Expr) -> Result<(), Error> {
        for name in &expr.natives {
            if node.kind != NodeKind::Native {
                return Err(Error::native(path.clone(), name));
            }
            if !node.natives.is_empty() && !node.natives.contains(name) {
                return Err(Error::native_allowed(
                    path.clone(),
                    name,
                    node.natives.iter().cloned(),
                ));
            }
        }
        for value in expr.candidates {
            self.emit_value(path, node, value, EmitKind::Candidate)?;
        }
        for value in expr.defaults {
            self.emit_value(path, node, value, EmitKind::Default)?;
        }
        for constraint in expr.constraints {
            self.push_fact(path, FactKind::Constraint(constraint));
        }
        Ok(())
    }

    pub(crate) fn emit_value(
        &mut self,
        path: &Path,
        node: &Node,
        value: Value,
        kind: EmitKind,
    ) -> Result<(), Error> {
        if node.kind == NodeKind::Union {
            check_union_value(node, path, &value)?;
        }

        if node.kind == NodeKind::List && node.key_field.is_none() {
            if let (Value::List(items), Some(element)) = (&value, node.element.as_deref()) {
                if element.kind == NodeKind::Union {
                    for item in items {
                        check_union_value(element, path, item)?;
                    }
                }
            }
        }

        if node.kind == NodeKind::List && node.key_field.is_some() {
            match value {
                Value::List(items) => {
                    if items.is_empty() {
                        self.push_value(path, kind, Value::List(Vec::new()));
                        return Ok(());
                    }
                    let mut seen = std::collections::BTreeSet::new();
                    for item in items {
                        let Value::Map(entry) = item else {
                            return Err(Error::path(
                                path.to_string(),
                                "a keyed list contains objects",
                            ));
                        };
                        let key_field = node.key_field.as_deref().expect("keyed");
                        let key = match entry.get(key_field) {
                            Some(Value::String(key)) => key.clone(),
                            _ => {
                                return Err(Error::path(
                                    path.to_string(),
                                    format!("an element needs a string '{key_field}'"),
                                ));
                            }
                        };
                        if !seen.insert(key.clone()) {
                            return Err(Error::path(
                                path.to_string(),
                                format!("duplicate key '{key}' in list"),
                            ));
                        }
                        let child = path
                            .child(&key)
                            .map_err(|error| Error::path(path.to_string(), error.to_string()))?;
                        for (field, field_value) in entry {
                            if field == key_field {
                                continue;
                            }
                            self.emit_child(&child, &field, field_value, kind)?;
                        }
                    }
                    return Ok(());
                }
                Value::Map(entry) => {
                    let key_field = node.key_field.as_deref().expect("keyed");
                    let key = match entry.get(key_field) {
                        Some(Value::String(key)) => key.clone(),
                        _ => {
                            return Err(Error::path(
                                path.to_string(),
                                format!("an element needs a string '{key_field}'"),
                            ));
                        }
                    };
                    let child = path
                        .child(&key)
                        .map_err(|error| Error::path(path.to_string(), error.to_string()))?;
                    for (field, field_value) in entry {
                        if field == key_field {
                            continue;
                        }
                        self.emit_child(&child, &field, field_value, kind)?;
                    }
                    return Ok(());
                }
                other => {
                    return Err(Error::path(
                        path.to_string(),
                        format!("expected list of objects, found {}", other.type_name()),
                    ));
                }
            }
        }

        if matches!(node.kind, NodeKind::Struct | NodeKind::Map) {
            if let Value::Map(entry) = value {
                for (field, field_value) in entry {
                    self.emit_child(path, &field, field_value, kind)?;
                }
                return Ok(());
            }
            return Err(Error::path(path.to_string(), "expected an object value"));
        }

        self.push_value(path, kind, value);
        Ok(())
    }

    pub(crate) fn emit_child(
        &mut self,
        base: &Path,
        field: &str,
        value: Value,
        kind: EmitKind,
    ) -> Result<(), Error> {
        let child = base
            .child(field)
            .map_err(|error| Error::path(base.to_string(), error.to_string()))?;
        let node = self
            .root
            .lookup(&child)
            .map_err(|reason| Error::path(child.to_string(), reason))?;
        match (&value, node.kind) {
            (
                Value::Map(_),
                NodeKind::Struct
                | NodeKind::Map
                | NodeKind::Native
                | NodeKind::Raw
                | NodeKind::Union,
            ) => self.emit_value(&child, node, value, kind),
            (Value::List(_), NodeKind::List) => self.emit_value(&child, node, value, kind),
            (Value::Map(_), _) => Err(Error::path(
                child.to_string(),
                "unexpected object for a scalar field",
            )),
            (Value::List(_), _) => self.emit_value(&child, node, value, kind),
            _ => {
                self.push_value(&child, kind, value);
                Ok(())
            }
        }
    }

    pub(crate) fn push_value(&mut self, path: &Path, kind: EmitKind, value: Value) {
        let kind = match kind {
            EmitKind::Candidate => FactKind::Candidate(value),
            EmitKind::Default => FactKind::Default(value),
        };
        self.push_fact(path, kind);
    }

    pub(crate) fn push_fact(&mut self, path: &Path, kind: FactKind) {
        self.facts.push(Fact {
            path: path.clone(),
            stamp: self.stamp,
            schema: self.schema.clone(),
            kind,
        });
    }
}

fn check_union_value(node: &Node, path: &Path, value: &Value) -> Result<(), Error> {
    if value.is_null() {
        if node.optional {
            return Ok(());
        }
        return Err(Error::path(
            path.to_string(),
            "a tagged union is an object, found null",
        ));
    }
    let Some(tag) = node.tag.as_deref() else {
        return Err(Error::path(
            path.to_string(),
            "tagged union has no tag field",
        ));
    };
    let Value::Map(map) = value else {
        return Err(Error::path(
            path.to_string(),
            format!("a tagged union is an object, found {}", value.type_name()),
        ));
    };
    let tag_value = map.get(tag).ok_or_else(|| {
        Error::path(
            path.to_string(),
            format!("a tagged union needs a '{tag}' field"),
        )
    })?;
    let Value::String(variant) = tag_value else {
        return Err(Error::path(
            path.to_string(),
            format!(
                "the tag '{tag}' is a string, found {}",
                tag_value.type_name()
            ),
        ));
    };
    if !node.variants.contains_key(variant) {
        return Err(Error::path(
            path.to_string(),
            format!("unknown variant '{variant}'"),
        ));
    }
    Ok(())
}
