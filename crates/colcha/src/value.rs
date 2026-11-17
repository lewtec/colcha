use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde::Serialize;

/// `f64` with a total order over its bit pattern. NaN is not representable.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(transparent)]
pub struct Float(f64);

impl Float {
    pub fn new(value: f64) -> Option<Float> {
        if value.is_nan() {
            None
        } else {
            Some(Float(value))
        }
    }

    pub fn get(&self) -> f64 {
        self.0
    }
}

impl PartialEq for Float {
    fn eq(&self, other: &Float) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for Float {}

impl PartialOrd for Float {
    fn partial_cmp(&self, other: &Float) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Float {
    fn cmp(&self, other: &Float) -> Ordering {
        self.0.to_bits().cmp(&other.0.to_bits())
    }
}

/// A concrete value after the fold.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(Float),
    String(String),
    List(Vec<Value>),
    Map(BTreeMap<String, Value>),
    Native { name: String, args: Box<Value> },
}

fn is_bare_key(key: &str) -> bool {
    !key.is_empty()
        && key.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        })
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(value) => Some(value.get()),
            Value::Int(value) => Some(*value as f64),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(values) => Some(values),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Value::Map(values) => Some(values),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Human-readable type name used in error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::String(_) => "string",
            Value::List(_) => "list",
            Value::Map(_) => "map",
            Value::Native { .. } => "native",
        }
    }

    /// Compare two scalar values numerically when possible.
    pub fn compare_scalar(left: &Value, right: &Value) -> Option<Ordering> {
        match (left, right) {
            (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
            (Value::Float(a), Value::Float(b)) => Some(a.cmp(b)),
            (Value::Int(a), Value::Float(b)) => (*a as f64).partial_cmp(&b.get()),
            (Value::Float(a), Value::Int(b)) => a.get().partial_cmp(&(*b as f64)),
            (Value::String(a), Value::String(b)) => Some(a.cmp(b)),
            _ => None,
        }
    }

    /// Compact, config-like rendering used in diagnostics.
    pub(crate) fn describe(&self) -> String {
        match self {
            Value::Null => "null".to_string(),
            Value::Bool(value) => value.to_string(),
            Value::Int(value) => value.to_string(),
            Value::Float(value) => format!("{:?}", value.get()),
            Value::String(value) => format!("{value:?}"),
            Value::List(values) => {
                let items: Vec<String> = values.iter().map(Value::describe).collect();
                format!("[{}]", items.join(", "))
            }
            Value::Map(values) => {
                let items: Vec<String> = values
                    .iter()
                    .map(|(key, value)| {
                        let key = if is_bare_key(key) {
                            key.clone()
                        } else {
                            format!("{key:?}")
                        };
                        format!("{key}: {}", value.describe())
                    })
                    .collect();
                format!("{{{}}}", items.join(", "))
            }
            Value::Native { name, args } => format!("_core.{name} {}", args.describe()),
        }
    }

    /// True when this value or a descendant is a native value.
    pub fn contains_native(&self) -> bool {
        match self {
            Value::Native { .. } => true,
            Value::List(values) => values.iter().any(Value::contains_native),
            Value::Map(values) => values.values().any(Value::contains_native),
            _ => false,
        }
    }
}

/// Opaque concrete value for heterogeneous configuration.
///
/// A `Raw` field accepts any concrete value. The host validates its shape.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Raw(pub Value);

impl Raw {
    pub fn new(value: Value) -> Raw {
        Raw(value)
    }

    pub fn as_value(&self) -> &Value {
        &self.0
    }

    pub fn into_inner(self) -> Value {
        self.0
    }
}

impl From<Value> for Raw {
    fn from(value: Value) -> Raw {
        Raw(value)
    }
}

impl From<Raw> for Value {
    fn from(value: Raw) -> Value {
        value.0
    }
}
