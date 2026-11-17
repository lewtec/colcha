use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::error::Error;
use crate::native::NativeHost;
use crate::path::Path;
use crate::value::{Raw, Value};

/// Turn a folded value into the Rust type it represents.
pub trait Materialize: Sized {
    fn materialize(value: &Value, host: &dyn NativeHost, path: &Path) -> Result<Self, Error>;
}

impl Materialize for Raw {
    fn materialize(value: &Value, _host: &dyn NativeHost, path: &Path) -> Result<Raw, Error> {
        if value.contains_native() {
            return Err(Error::materialize(
                path.clone(),
                "a raw value cannot contain a native value",
            ));
        }
        Ok(Raw(value.clone()))
    }
}

impl Materialize for String {
    fn materialize(value: &Value, _host: &dyn NativeHost, path: &Path) -> Result<String, Error> {
        match value {
            Value::String(text) => Ok(text.clone()),
            other => Err(Error::materialize(
                path.clone(),
                format!("expected string, found {}", other.type_name()),
            )),
        }
    }
}

impl Materialize for bool {
    fn materialize(value: &Value, _host: &dyn NativeHost, path: &Path) -> Result<bool, Error> {
        match value {
            Value::Bool(value) => Ok(*value),
            other => Err(Error::materialize(
                path.clone(),
                format!("expected bool, found {}", other.type_name()),
            )),
        }
    }
}

impl Materialize for f64 {
    fn materialize(value: &Value, _host: &dyn NativeHost, path: &Path) -> Result<f64, Error> {
        value.as_f64().ok_or_else(|| {
            Error::materialize(
                path.clone(),
                format!("expected float, found {}", value.type_name()),
            )
        })
    }
}

impl Materialize for f32 {
    fn materialize(value: &Value, host: &dyn NativeHost, path: &Path) -> Result<f32, Error> {
        Ok(f64::materialize(value, host, path)? as f32)
    }
}

macro_rules! integer_materialize {
    ($($ty:ty),* $(,)?) => {
        $(
            impl Materialize for $ty {
                fn materialize(
                    value: &Value,
                    _host: &dyn NativeHost,
                    path: &Path,
                ) -> Result<$ty, Error> {
                    match value {
                        Value::Int(value) => <$ty>::try_from(*value).map_err(|_| {
                            Error::materialize(
                                path.clone(),
                                format!("int {value} does not fit {}", stringify!($ty)),
                            )
                        }),
                        other => Err(Error::materialize(
                            path.clone(),
                            format!("expected int, found {}", other.type_name()),
                        )),
                    }
                }
            }
        )*
    };
}

integer_materialize!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

impl<T: Materialize> Materialize for Option<T> {
    fn materialize(value: &Value, host: &dyn NativeHost, path: &Path) -> Result<Option<T>, Error> {
        if value.is_null() {
            Ok(None)
        } else {
            T::materialize(value, host, path).map(Some)
        }
    }
}

impl<T: Materialize> Materialize for Vec<T> {
    fn materialize(value: &Value, host: &dyn NativeHost, path: &Path) -> Result<Vec<T>, Error> {
        match value {
            Value::List(items) => items
                .iter()
                .map(|item| T::materialize(item, host, path))
                .collect(),
            other => Err(Error::materialize(
                path.clone(),
                format!("expected list, found {}", other.type_name()),
            )),
        }
    }
}

impl<T: Materialize + Ord> Materialize for BTreeSet<T> {
    fn materialize(
        value: &Value,
        host: &dyn NativeHost,
        path: &Path,
    ) -> Result<BTreeSet<T>, Error> {
        match value {
            Value::List(items) => items
                .iter()
                .map(|item| T::materialize(item, host, path))
                .collect(),
            other => Err(Error::materialize(
                path.clone(),
                format!("expected list, found {}", other.type_name()),
            )),
        }
    }
}

impl<T: Materialize + Eq + std::hash::Hash> Materialize for HashSet<T> {
    fn materialize(value: &Value, host: &dyn NativeHost, path: &Path) -> Result<HashSet<T>, Error> {
        match value {
            Value::List(items) => items
                .iter()
                .map(|item| T::materialize(item, host, path))
                .collect(),
            other => Err(Error::materialize(
                path.clone(),
                format!("expected list, found {}", other.type_name()),
            )),
        }
    }
}

impl<T: Materialize> Materialize for BTreeMap<String, T> {
    fn materialize(
        value: &Value,
        host: &dyn NativeHost,
        path: &Path,
    ) -> Result<BTreeMap<String, T>, Error> {
        match value {
            Value::Map(map) => {
                let mut result = BTreeMap::new();
                for (key, item) in map {
                    let child = path
                        .child(key)
                        .map_err(|error| Error::materialize(path.clone(), error.to_string()))?;
                    result.insert(key.clone(), T::materialize(item, host, &child)?);
                }
                Ok(result)
            }
            other => Err(Error::materialize(
                path.clone(),
                format!("expected map, found {}", other.type_name()),
            )),
        }
    }
}

impl<T: Materialize> Materialize for HashMap<String, T> {
    fn materialize(
        value: &Value,
        host: &dyn NativeHost,
        path: &Path,
    ) -> Result<HashMap<String, T>, Error> {
        match value {
            Value::Map(map) => {
                let mut result = HashMap::new();
                for (key, item) in map {
                    let child = path
                        .child(key)
                        .map_err(|error| Error::materialize(path.clone(), error.to_string()))?;
                    result.insert(key.clone(), T::materialize(item, host, &child)?);
                }
                Ok(result)
            }
            other => Err(Error::materialize(
                path.clone(),
                format!("expected map, found {}", other.type_name()),
            )),
        }
    }
}
