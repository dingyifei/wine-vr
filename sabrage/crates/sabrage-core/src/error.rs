//! Error taxonomy. v1 frame — the stage/process/privilege variants land with the
//! stages that raise them (design-core §8); the three below are what the check
//! layer and the path layer can actually produce today.

use std::path::PathBuf;

/// Convenience alias for fallible sabrage-core APIs.
pub type Result<T> = std::result::Result<T, SabrageError>;

#[derive(Debug, thiserror::Error)]
pub enum SabrageError {
    /// `die()` parity: the message text is preserved verbatim so docs quoting
    /// demo.sh keep being true, and `remedy` carries the one-line fix demo.sh
    /// prints after `remedy:`. CLI exit 1.
    #[error("{message}")]
    Fatal {
        message: String,
        remedy: Option<String>,
    },

    /// Usage errors — unknown flag, missing value, unparseable argument.
    /// CLI exit 2 (demo.sh usage parity).
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// I/O with the path attached; bare [`std::io::Error`] loses which file broke.
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl SabrageError {
    /// Build a [`SabrageError::Fatal`] with a remedy string.
    pub fn fatal(message: impl Into<String>, remedy: impl Into<String>) -> SabrageError {
        SabrageError::Fatal {
            message: message.into(),
            remedy: Some(remedy.into()),
        }
    }

    /// Build a [`SabrageError::Fatal`] with no remedy.
    pub fn fatal_bare(message: impl Into<String>) -> SabrageError {
        SabrageError::Fatal {
            message: message.into(),
            remedy: None,
        }
    }

    /// Wrap an [`std::io::Error`] with the path that produced it.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> SabrageError {
        SabrageError::Io {
            path: path.into(),
            source,
        }
    }

    /// Stable machine-readable discriminant for `--json` output and for the GUI
    /// (which must never parse message prose).
    pub fn kind(&self) -> &'static str {
        match self {
            SabrageError::Fatal { .. } => "fatal",
            SabrageError::InvalidInput(_) => "invalid_input",
            SabrageError::Io { .. } => "io",
        }
    }

    /// The remedy line, when this error carries one.
    pub fn remedy(&self) -> Option<&str> {
        match self {
            SabrageError::Fatal { remedy, .. } => remedy.as_deref(),
            _ => None,
        }
    }

    /// demo.sh exit-code parity: `2` for usage errors, `1` otherwise.
    pub fn exit_code(&self) -> i32 {
        match self {
            SabrageError::InvalidInput(_) => 2,
            _ => 1,
        }
    }
}
