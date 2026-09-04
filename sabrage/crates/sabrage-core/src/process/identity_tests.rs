use super::*;
use crate::events::step;
use uuid::Uuid;

#[test]
fn observing_this_process_agrees_with_the_full_scan() {
    let me = std::process::id();
    let observed = ProcInfo::observe(me).expect("own pid is observable");
    assert_eq!(observed.pid, me);
    assert!(observed.start_time > 0);

    // The single-pid refresh and the whole-table scan must agree.
    let exe = std::env::current_exe().unwrap();
    let scanned = find_processes_by_exe(&exe)
        .into_iter()
        .find(|p| p.pid == me)
        .expect("own pid in the scan");
    assert_eq!(scanned.start_time, observed.start_time);

    // A pid that cannot exist for this user.
    assert!(ProcInfo::observe(u32::MAX - 1).is_none());
}

#[test]
fn identity_rejects_a_recycled_pid_and_the_unobservable_fallback() {
    let mut me = ProcInfo::observe(std::process::id()).unwrap();
    assert!(me.is_same_process());

    // Same pid, different start time = a recycled pid: never ours.
    me.start_time += 1;
    assert!(!me.is_same_process());

    // The spawn-time fallback (start_time 0) is deliberately unverifiable.
    let fallback = ProcInfo {
        pid: std::process::id(),
        start_time: 0,
        exe: PathBuf::from("/bin/sh"),
    };
    assert!(!fallback.is_same_process());

    let dead = ProcInfo {
        pid: u32::MAX - 1,
        start_time: 1,
        exe: PathBuf::new(),
    };
    assert!(!dead.is_same_process());
}

#[tokio::test]
async fn capture_collects_both_pipes_and_the_status() {
    let spec = ChildSpec::new("/bin/sh", step::BUILD_TOOLS, Uuid::nil())
        .arg("-c")
        .arg("printf 'devices\\n'; printf 'oops\\n' >&2; exit 2");
    let out = capture(&spec).await.unwrap();
    assert_eq!(out.stdout, "devices\n");
    assert_eq!(out.stdout_trimmed(), "devices");
    assert_eq!(out.stderr, "oops\n");
    assert_eq!(exit_code_of(out.status), 2);

    // A missing program is an Io error naming it, not a panic.
    let missing = ChildSpec::new("/nonexistent/sabrage/adb", step::BUILD_TOOLS, Uuid::nil());
    assert_eq!(capture(&missing).await.unwrap_err().kind(), "io");
}

#[test]
fn cmdline_matching_is_the_pgrep_f_shape() {
    assert!(cmdline_contains(
        &["wine".into(), "Z:\\games\\Beat Saber.exe".into()],
        "Beat Saber.exe"
    ));
    // Split across two argv elements still matches via the joined line.
    assert!(cmdline_contains(
        &["Beat".into(), "Saber.exe".into()],
        "Beat Saber.exe"
    ));
    assert!(!cmdline_contains(&["wineserver".into()], "Beat Saber.exe"));
    assert!(!cmdline_contains(&[], "Beat Saber.exe"));
}

/// The cmdline scan matches a substring of the joined command line, so it
/// finds this very test binary — the same false-positive class `pgrep -f`
/// has, which is why the reap steps match on the exe path instead (module
/// header).
#[test]
fn find_processes_by_cmdline_finds_this_test_binary_by_a_name_suffix() {
    let exe = std::env::current_exe().expect("test binary path");
    let name = exe
        .file_name()
        .and_then(|n| n.to_str())
        .expect("utf8 test binary name");
    // A short, distinctive suffix of the real binary name, not the whole
    // path — proving this is a *substring-of-cmdline* match, unlike
    // `find_processes_by_exe`'s exact-path equality.
    let needle = &name[name.len().saturating_sub(6)..];
    let found = find_processes_by_cmdline(needle);
    let me = std::process::id();
    assert!(
        found.iter().any(|p| p.pid == me),
        "own pid {me} not found by cmdline needle {needle:?} among {found:?}"
    );
    assert!(find_processes_by_cmdline("nonexistent-sabrage-needle.exe").is_empty());
}
