//! Group `audio` — doctor.sh section 15: the optional in-headset audio loopback.
//!
//! Slugs owned here, in contract order:
//!
//! * `audio.loopback` — `SwitchAudioSource` installed AND `BlackHole 2ch`
//!   present in the output device list; both failure modes are WARN
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a **read-only probe**.
//! Message and remedy strings must match `scripts/demo/doctor.sh` verbatim.

use std::process::Command;

use super::Evaluator;
#[allow(unused_imports)]
use super::{CheckCtx, CheckOutcome, CheckStatus, SkipReason};
use crate::paths::which;

/// doctor.sh section 15:
/// ```sh
/// if command -v SwitchAudioSource >/dev/null 2>&1; then
///   if SwitchAudioSource -a -t output 2>/dev/null | grep -qx "BlackHole 2ch"; then chk ok …
///   else chk warn … "BlackHole 2ch not present …"; fi
/// else chk warn … "switchaudio-osx not installed …"; fi
/// ```
/// `grep -qx` is a *whole-line* match: `"BlackHole 2ch"` must be exactly one
/// line of `-a -t output`'s device list, not a substring of a longer name.
fn audio_loopback(_ctx: &CheckCtx) -> CheckOutcome {
    let Some(bin) = which("SwitchAudioSource") else {
        return CheckOutcome::warn(
            "audio.loopback",
            "switchaudio-osx not installed — audio stays on the Mac (brew install \
             switchaudio-osx blackhole-2ch)",
        );
    };
    let has_blackhole = Command::new(&bin)
        .args(["-a", "-t", "output"])
        .output()
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .any(|line| line == "BlackHole 2ch")
        })
        .unwrap_or(false);
    if has_blackhole {
        CheckOutcome::pass("audio.loopback", "BlackHole 2ch + switchaudio-osx")
    } else {
        CheckOutcome::warn(
            "audio.loopback",
            "BlackHole 2ch not present — no in-headset audio (brew install blackhole-2ch, then \
             reboot)",
        )
    }
}

/// Evaluators this module binds, keyed by contract slug.
pub fn defs() -> Vec<(&'static str, Evaluator)> {
    vec![("audio.loopback", audio_loopback as Evaluator)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::CheckOptions;
    use crate::paths::Paths;

    fn ctx() -> CheckCtx {
        CheckCtx::new(
            Paths::new("/nonexistent/sabrage-audio-probe"),
            CheckOptions::new(),
        )
    }

    #[test]
    fn matches_ground_truth_on_this_machine() {
        // Whether switchaudio-osx / BlackHole are installed is machine state
        // (paths.rs's own testing pattern); assert internal consistency with
        // a direct probe instead of a fixed outcome.
        let o = audio_loopback(&ctx());
        match which("SwitchAudioSource") {
            None => {
                assert_eq!(o.status, CheckStatus::Warn);
                assert_eq!(
                    o.message,
                    "switchaudio-osx not installed — audio stays on the Mac (brew install \
                     switchaudio-osx blackhole-2ch)"
                );
            }
            Some(bin) => {
                let has_blackhole = Command::new(&bin)
                    .args(["-a", "-t", "output"])
                    .output()
                    .map(|out| {
                        String::from_utf8_lossy(&out.stdout)
                            .lines()
                            .any(|line| line == "BlackHole 2ch")
                    })
                    .unwrap_or(false);
                if has_blackhole {
                    assert_eq!(o.status, CheckStatus::Pass);
                    assert_eq!(o.message, "BlackHole 2ch + switchaudio-osx");
                } else {
                    assert_eq!(o.status, CheckStatus::Warn);
                    assert_eq!(
                        o.message,
                        "BlackHole 2ch not present — no in-headset audio (brew install \
                         blackhole-2ch, then reboot)"
                    );
                }
            }
        }
    }

    #[test]
    fn defs_binds_the_one_slug() {
        let slugs: Vec<&str> = defs().into_iter().map(|(s, _)| s).collect();
        assert_eq!(slugs, vec!["audio.loopback"]);
    }
}
