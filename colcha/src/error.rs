use crate::path::Path;
use crate::schema::SchemaId;
use crate::value::Value;

/// Stable error variants. `Display` is informative; callers match on the variant.
#[derive(Debug)]
pub enum Error {
    /// A path is malformed or unknown for the schema.
    Path { path: String, reason: String },
    /// Authored expressions could not be resolved to concrete facts.
    Eval { path: String, reason: String },
    /// The DSL text is not well formed.
    Parse {
        line: usize,
        column: usize,
        message: String,
    },
    /// A fact was produced for a different schema identity.
    Schema { expected: SchemaId, found: SchemaId },
    /// The fold could not produce a concrete value.
    Fold { path: Path, reason: String },
    /// Two valid candidates disagree under a policy that requires agreement.
    Conflict { path: Path, candidates: Vec<Value> },
    /// A native name is missing or not allowed at a path.
    Native { path: Path, name: String },
    /// The host failed to build a native value during `resolve`.
    Build {
        path: Path,
        name: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// A folded value has no representation in the Rust type.
    Materialize { path: Path, message: String },
}

impl Error {
    pub(crate) fn path(path: impl Into<String>, reason: impl Into<String>) -> Error {
        Error::Path {
            path: path.into(),
            reason: reason.into(),
        }
    }

    pub(crate) fn parse(line: usize, column: usize, message: impl Into<String>) -> Error {
        Error::Parse {
            line,
            column,
            message: message.into(),
        }
    }

    pub(crate) fn eval(path: impl Into<String>, reason: impl Into<String>) -> Error {
        Error::Eval {
            path: path.into(),
            reason: reason.into(),
        }
    }

    pub(crate) fn fold(path: Path, reason: impl Into<String>) -> Error {
        Error::Fold {
            path,
            reason: reason.into(),
        }
    }

    pub(crate) fn conflict(path: Path, candidates: Vec<Value>) -> Error {
        Error::Conflict { path, candidates }
    }

    pub(crate) fn native(path: Path, name: impl Into<String>) -> Error {
        Error::Native {
            path,
            name: name.into(),
        }
    }

    pub(crate) fn materialize(path: Path, message: impl Into<String>) -> Error {
        Error::Materialize {
            path,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Path { path, reason } => write!(f, "invalid path {path}: {reason}"),
            Error::Eval { path, reason } => write!(f, "evaluation failed at {path}: {reason}"),
            Error::Parse {
                line,
                column,
                message,
            } => write!(f, "parse error at {line}:{column}: {message}"),
            Error::Schema { expected, found } => {
                write!(f, "schema mismatch: expected {expected}, found {found}")
            }
            Error::Fold { path, reason } => write!(f, "fold failed at {path}: {reason}"),
            Error::Conflict { path, candidates } => {
                write!(f, "conflict at {path}: {candidates:?}")
            }
            Error::Native { path, name } => write!(f, "native {name} is not usable at {path}"),
            Error::Build { path, name, source } => {
                write!(f, "build of native {name} failed at {path}: {source}")
            }
            Error::Materialize { path, message } => {
                write!(f, "materialize failed at {path}: {message}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Build { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}
