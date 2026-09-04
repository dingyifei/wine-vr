//! Error taxonomy (design-core §8).
//!
//! `Download` and `HashMismatch` keep `Display` byte-identical to `lib.sh`'s
//! `fetch_pinned` die text, so a front-end can branch on `kind()` while the
//! console text still matches the shell and the docs that quote it
//! (tests::display_matches_lib_sh_die_text).

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

    /// A spawned child exited non-zero. `tail` carries the last
    /// [`CHILD_TAIL_LINES`] output lines so the error explains itself without a
    /// log hunt (design-core §6.5: no swallowed diagnostics).
    #[error("child failed: {argv0} exited {status}")]
    ChildFailed {
        argv0: String,
        status: i32,
        tail: Vec<String>,
    },

    /// The operation was cancelled (user Stop, or the CLI's INT handler).
    /// CLI exit 130.
    #[error("cancelled")]
    Cancelled,

    /// `curl -fL --retry 3` failed. Message text is `lib.sh`'s
    /// `die "download failed: $url"` verbatim.
    #[error("download failed: {url}")]
    Download {
        url: String,
        /// Child exit status / stderr tail, when there is one. Never part of
        /// the `Display` string — that must stay shell-identical.
        detail: Option<String>,
    },

    /// A pinned artifact hashed differently than the contract says. Message text
    /// is `lib.sh`'s `die "sha256 mismatch for $label (got $x)"` verbatim.
    #[error("sha256 mismatch for {label} (got {got})")]
    HashMismatch { label: String, got: String },

    /// The user declined (or the prompt failed) the one privileged write in the
    /// pipeline — install layer 4.
    #[error("administrator authorization declined")]
    AdminDeclined,

    /// A write under a `.app` bundle was refused: macOS App Management (TCC),
    /// which `sudo` cannot fix. Install layers 1–2 only.
    #[error("macOS App Management permission denied for {path}")]
    TccDenied { path: PathBuf },
}

/// How many trailing output lines [`SabrageError::ChildFailed`] carries.
pub const CHILD_TAIL_LINES: usize = 20;

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

    /// Stable machine-readable discriminant of the variant, for callers that
    /// must not parse message prose; today only the tests key on it.
    pub fn kind(&self) -> &'static str {
        match self {
            SabrageError::Fatal { .. } => "fatal",
            SabrageError::InvalidInput(_) => "invalid_input",
            SabrageError::Io { .. } => "io",
            SabrageError::ChildFailed { .. } => "child_failed",
            SabrageError::Cancelled => "cancelled",
            SabrageError::Download { .. } => "download",
            SabrageError::HashMismatch { .. } => "hash_mismatch",
            SabrageError::AdminDeclined => "admin_declined",
            SabrageError::TccDenied { .. } => "tcc_denied",
        }
    }

    /// Has this error's prose already reached the user as a `Fatal` row?
    ///
    /// True for the variants whose raiser emits the row and then returns the
    /// error, its callers propagating rather than re-emitting
    /// ([`crate::stages::StageCtx::fatal`], `privilege::upgrade_write_error`,
    /// `privilege::upgrade_child_write_error`, `privilege::elevate_osascript`),
    /// and for [`SabrageError::Cancelled`], where the user's own Stop or Ctrl-C
    /// is the report and the exit code (130) carries the fact. A front-end
    /// reports the error only when this is false — the CLI's final `error:`
    /// line — and the rule lives in core so the GUI can share it
    /// (tests::already_reported_covers_the_variants_that_emit_their_own_row).
    pub fn already_reported(&self) -> bool {
        matches!(
            self,
            SabrageError::Fatal { .. }
                | SabrageError::TccDenied { .. }
                | SabrageError::AdminDeclined
                | SabrageError::Cancelled
        )
    }

    /// The remedy line, when this error carries one.
    pub fn remedy(&self) -> Option<&str> {
        match self {
            SabrageError::Fatal { remedy, .. } => remedy.as_deref(),
            _ => None,
        }
    }

    /// demo.sh exit-code parity: `2` for usage errors, `130` for cancellation
    /// (INT parity), `1` otherwise.
    pub fn exit_code(&self) -> i32 {
        match self {
            SabrageError::InvalidInput(_) => 2,
            SabrageError::Cancelled => 130,
            _ => 1,
        }
    }

    /// The captured child output tail, when this error carries one.
    pub fn tail(&self) -> &[String] {
        match self {
            SabrageError::ChildFailed { tail, .. } => tail,
            _ => &[],
        }
    }
}

#[cfg(test)]
mod tests;
