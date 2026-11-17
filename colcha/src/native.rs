use std::any::Any;

use crate::schema::Native;
use crate::value::Value;

/// Opaque value produced by `NativeHost::build`.
pub struct NativeHandle(Box<dyn Any + Send + Sync>);

impl NativeHandle {
    pub fn new<T: Any + Send + Sync>(value: T) -> NativeHandle {
        NativeHandle(Box::new(value))
    }

    pub fn downcast<T: Any>(self) -> Result<T, String> {
        self.0
            .downcast::<T>()
            .map(|boxed| *boxed)
            .map_err(|_| "native handle has a different concrete type".to_string())
    }

    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.0.downcast_ref::<T>()
    }
}

impl std::fmt::Debug for NativeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeHandle(..)")
    }
}

/// The host owns the set of native types that configuration may select.
pub trait NativeRegistry {
    fn spec(&self, name: &str) -> Option<&Native>;
}

/// The only point of effect in the crate.
pub trait NativeHost: NativeRegistry {
    fn build(
        &self,
        name: &str,
        args: &Value,
    ) -> Result<NativeHandle, Box<dyn std::error::Error + Send + Sync>>;
}

/// Rust type of a `#[colcha(native)]` field.
pub trait NativeType: Sized {
    const NAME: &'static str;

    fn from_native(handle: NativeHandle) -> Result<Self, String>;
}

/// Registry with no natives. Useful for schemas without native fields.
#[derive(Debug, Default)]
pub struct EmptyHost;

impl NativeRegistry for EmptyHost {
    fn spec(&self, _name: &str) -> Option<&Native> {
        None
    }
}

impl NativeHost for EmptyHost {
    fn build(
        &self,
        name: &str,
        _args: &Value,
    ) -> Result<NativeHandle, Box<dyn std::error::Error + Send + Sync>> {
        Err(format!("no native '{name}' is registered").into())
    }
}
