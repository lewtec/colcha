use crate::error::Error;
use crate::materialize::Materialize;
use crate::native::NativeHost;
use crate::path::Path;
use crate::schema::Schema;
use crate::state::State;

/// Fold and materialize, with effects only in `NativeHost::build`.
pub fn resolve<T: Schema + Materialize>(state: &State, host: &dyn NativeHost) -> Result<T, Error> {
    let value = crate::fold::fold::<T>(state, host)?;
    T::materialize(&value, host, &Path::root())
}
