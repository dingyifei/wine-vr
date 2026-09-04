//! User-facing configuration: [`runtime_toml`], the typed, format-preserving
//! editor for `~/Library/Application Support/OXRSys/oxrsys-runtime.toml`,
//! written once by `demo.sh` and never touched again.
//!
//! Sabrage owns the six `EDITABLE_KEYS` values and nothing else: ordering, spacing,
//! unknown keys, the provenance header and comments survive byte-for-byte because the
//! file is shared with a human and a line-oriented C++ parser that is not a TOML
//! implementation (`runtime_toml::tests::a_real_edit_preserves_crlf_a_bom_and_a_missing_final_newline`).
//! Exceptions, both owned by [`runtime_toml`]'s header: a same-line `#` comment is relocated
//! above the key being edited
//! (`runtime_toml::tests::a_same_line_comment_moves_to_its_own_line_above_the_key`),
//! and a file with mixed line endings is rendered LF.
//! See `sabrage/docs/design/design-core.md` §4.1.
//!
//! Sabrage's own state — settings, the game library — lives in [`crate::store`].

pub mod runtime_toml;

pub use runtime_toml::{
    apply_patch, blocking_session, effective_accepted, effective_string, list_backups, read,
    read_lines_like_the_runtime, runtime_defaults, validate, write, BackupInfo, EncoderProcess,
    InvalidValue, Patched, Protocol, RuntimeConfigPatch, RuntimeConfigValues, RuntimeConfigView,
    VideoCodec, WriteReport, BACKUP_KEEP, BACKUP_PREFIX, EDITABLE_KEYS, TABLE,
};
