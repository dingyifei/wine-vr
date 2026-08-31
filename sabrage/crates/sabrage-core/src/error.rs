//! Error taxonomy (design-core §8).
//!
//! Phase 1 shipped the three variants the check and path layers can produce;
//! Phase 2's frame adds the ones the stage/process/privilege layers raise.
//!
//! Two of the new variants exist to keep `die()` text verbatim **and** carry
//! structure: `Download`'s and `HashMismatch`'s `Display` strings are
//! byte-identical to `lib.sh`'s `fetch_pinned` failures
//! (`download failed: $url`, `sha256 mismatch for $label (got $hash)`), so the
//! GUI can branch on `kind()` while the console text still matches the shell
//! and the docs that quote it.

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

    /// Stable machine-readable discriminant for `--json` output and for the GUI
    /// (which must never parse message prose).
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
    /// The rule a front-end needs when an operation fails: print (or show) the
    /// error, *unless* the layer that raised it already said the same thing in
    /// the event stream. Three variants are contracts to exactly that effect —
    /// [`crate::stages::StageCtx::fatal`] emits the row and returns
    /// [`SabrageError::Fatal`]; `privilege::upgrade_write_error` emits the App
    /// Management explanation and returns [`SabrageError::TccDenied`];
    /// `privilege::elevate_osascript` emits the declined-authorization row and
    /// returns [`SabrageError::AdminDeclined`] — and all three document that
    /// the caller must propagate rather than re-emit.
    ///
    /// [`SabrageError::Cancelled`] is included for a different reason: it is
    /// the user's own Stop or Ctrl-C. `run` already printed run.sh's
    /// `-- interrupted: stopping wine` section, a build stage's child simply
    /// stops, and `demo.sh` prints nothing after its INT trap re-raises the
    /// signal — a trailing `error: cancelled` would be the one line the shell
    /// never shows. The exit code (130) still carries the fact.
    ///
    /// Lives here rather than in either front-end because both need it: the
    /// CLI decides whether to print a final `error:` line, the GUI whether to
    /// surface a second banner over the `Fatal` row already in the run log.
    pub fn already_reported(&self) -> bool {
        matches!(
            self,
            SabrageError::Fatal { .. }
                | SabrageError::TccDenied { .. }
                | SabrageError::AdminDeclined
                | SabrageError::Cancelled
        )
    }

    /// This error as the flat, serializable shape a front-end renders.
    pub fn payload(&self) -> ErrorPayload {
        ErrorPayload {
            kind: self.kind(),
            message: self.to_string(),
            remedy: self.remedy().map(str::to_string),
            already_reported: self.already_reported(),
        }
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

/// One error, flattened for a front-end: the machine-readable discriminant, the
/// prose, the remedy, and whether the user has already been told.
///
/// The GUI must never parse message text ([`SabrageError::kind`]'s whole
/// reason), and both front-ends need the same four fields — the CLI for its
/// `--json` output and its final `error:` line, the GUI for the failure banner
/// it puts over a run log. camelCase because `ui/src/ipc.ts` mirrors these
/// types by hand, like every other serialized shape in this crate.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorPayload {
    /// [`SabrageError::kind`].
    pub kind: &'static str,
    /// The `Display` text — `die()`-verbatim where the shell has a counterpart.
    pub message: String,
    /// [`SabrageError::remedy`], the one-line fix.
    pub remedy: Option<String>,
    /// [`SabrageError::already_reported`]: true when the prose is already in
    /// the event stream as a `Fatal` row, so rendering it again would double
    /// it up.
    pub already_reported: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn already_reported_covers_the_variants_that_emit_their_own_row() {
        for e in [
            SabrageError::fatal("no bottle", "create it"),
            SabrageError::TccDenied {
                path: PathBuf::from("/Applications/CrossOver.app"),
            },
            SabrageError::AdminDeclined,
            SabrageError::Cancelled,
        ] {
            assert!(e.already_reported(), "{e:?}");
        }
        for e in [
            SabrageError::InvalidInput("--nope".into()),
            SabrageError::io(
                "/x",
                std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            ),
            SabrageError::ChildFailed {
                argv0: "cmake".into(),
                status: 2,
                tail: Vec::new(),
            },
            SabrageError::Download {
                url: "https://h/x".into(),
                detail: None,
            },
            SabrageError::HashMismatch {
                label: "DXMT".into(),
                got: "abc".into(),
            },
        ] {
            assert!(!e.already_reported(), "{e:?}");
        }
    }

    #[test]
    fn payload_carries_kind_message_remedy_and_the_reported_flag() {
        let p = SabrageError::fatal("bottle 'Steam' not found", "create it in the CrossOver UI")
            .payload();
        assert_eq!(p.kind, "fatal");
        assert_eq!(p.message, "bottle 'Steam' not found");
        assert_eq!(p.remedy.as_deref(), Some("create it in the CrossOver UI"));
        assert!(p.already_reported);

        let io = SabrageError::io("/x", std::io::Error::from(std::io::ErrorKind::NotFound));
        let p = io.payload();
        assert_eq!(p.kind, "io");
        assert_eq!(p.message, io.to_string());
        assert_eq!(p.remedy, None);
        assert!(!p.already_reported);

        // camelCase on the wire: `ui/src/ipc.ts` mirrors this by hand.
        let j = serde_json::to_value(&p).unwrap();
        assert_eq!(j["kind"], "io");
        assert_eq!(j["alreadyReported"], false);
    }
}
