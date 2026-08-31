//! The user-facing configuration layer (Phase 4).
//!
//! One module today: [`runtime_toml`], the typed, format-preserving editor for
//! `~/Library/Application Support/OXRSys/oxrsys-runtime.toml` — the file
//! `demo.sh` writes exactly once and then never touches again.
//!
//! This is the layer's boundary in one sentence: **Sabrage owns the six
//! streaming keys' values and nothing else about that file.** Comments,
//! ordering, spacing, unknown keys and the hand-written provenance header all
//! survive an edit byte-for-byte, because the file is shared with a human and
//! with a line-oriented C++ parser that is not a TOML implementation. See
//! `sabrage/docs/design/design-core.md` §4.1 and [`runtime_toml`]'s header.
//!
//! Sabrage's *own* state — settings, the game library — is not configuration in
//! this sense and lives in [`crate::store`].

pub mod runtime_toml;

pub use runtime_toml::{
    apply_patch, blocking_session, effective_string, list_backups, read,
    read_lines_like_the_runtime, runtime_defaults, validate, write, BackupInfo, EncoderProcess,
    InvalidValue, Patched, Protocol, RuntimeConfigPatch, RuntimeConfigValues, RuntimeConfigView,
    VideoCodec, WriteReport, BACKUP_KEEP, BACKUP_PREFIX, EDITABLE_KEYS, TABLE,
};
