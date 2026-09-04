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
