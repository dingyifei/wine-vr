use super::*;
use chrono::TimeZone;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sabrage-logs-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

thread_local! {
    /// Per-thread, so one test's hook cannot reach another's `poll` —
    /// `cargo test` gives each `#[test]` its own thread.
    static AFTER_READ: std::cell::RefCell<Option<Box<dyn Fn()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Called by [`Tailer::poll`] between its read loop and its post-read
/// continuity check — the exact window a truncate-and-regrow has to land
/// in to be read as a continuation (A8-7). Nothing outside `cfg(test)` can
/// register anything, and the default is a no-op.
pub(super) fn after_read_hook() {
    let f = AFTER_READ.with(|h| h.borrow_mut().take());
    if let Some(f) = f {
        f();
        AFTER_READ.with(|h| *h.borrow_mut() = Some(f));
    }
}

/// Register a one-shot rewrite to happen inside that window. Fires once:
/// the test's *later* polls must see an ordinary file.
fn on_next_read(f: impl Fn() + 'static) {
    let fired = std::cell::Cell::new(false);
    AFTER_READ.with(|h| {
        *h.borrow_mut() = Some(Box::new(move || {
            if !fired.replace(true) {
                f();
            }
        }))
    });
}

/// Every test that registers one must clear it — the thread is reused by
/// nothing here, but a leaked hook is a trap for the next edit.
fn clear_read_hook() {
    AFTER_READ.with(|h| *h.borrow_mut() = None);
}

/// A [`Paths`] rooted entirely under `root` — including `oxr_appsup` and
/// `sabrage_appsup`, which [`Paths::new`] otherwise derives from the real
/// `$HOME`. A test must never touch the real `~/Library`.
fn fixture_paths(root: &Path) -> Paths {
    let mut paths = Paths::new(root);
    paths.oxr_appsup = root.join("home/Library/Application Support/OXRSys");
    paths.sabrage_appsup = root.join("home/Library/Application Support/Sabrage");
    paths
}

fn set_mtime(path: &Path, unix_secs: u64) {
    let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(unix_secs);
    let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.set_modified(t).unwrap();
}

#[test]
fn wine_log_candidate_delegates_to_the_stamped_form_byte_for_byte() {
    // Both entry points, one expectation: the stamped form emits these
    // paths for a hand-built stamp with no chrono in sight (F16), and the
    // chrono wrapper delegates to it byte for byte, so neither can drift
    // alone.
    let now = chrono::Local
        .with_ymd_and_hms(2026, 8, 29, 10, 11, 12)
        .unwrap();
    let cases: &[(&str, u32, &str)] = &[
        (
            "attempt 0: bare stamp, no suffix",
            0,
            "/repo/logs/beatsaber-20260829-101112.log",
        ),
        (
            "attempt 1: first collision gets -2",
            1,
            "/repo/logs/beatsaber-20260829-101112-2.log",
        ),
        (
            "attempt 3: fourth candidate gets -4",
            3,
            "/repo/logs/beatsaber-20260829-101112-4.log",
        ),
    ];
    for (label, attempt, expected) in cases {
        let stamped =
            wine_log_candidate_stamped(Path::new("/repo/logs"), "20260829-101112", *attempt);
        assert_eq!(stamped, PathBuf::from(*expected), "{label}");
        assert_eq!(
            wine_log_candidate(Path::new("/repo/logs"), now, *attempt),
            stamped,
            "{label}"
        );
    }
}

#[test]
fn log_source_file_is_a_struct_variant_on_the_wire() {
    let s = LogSource::File {
        path: PathBuf::from("/repo/logs/x.log"),
    };
    let j = serde_json::to_value(&s).unwrap();
    assert_eq!(
        j,
        serde_json::json!({"kind": "file", "path": "/repo/logs/x.log"})
    );
    assert_eq!(serde_json::from_value::<LogSource>(j).unwrap(), s);

    assert_eq!(
        serde_json::to_value(LogSource::WineConsole).unwrap(),
        serde_json::json!({"kind": "wineConsole"})
    );
    assert_eq!(
        serde_json::to_value(LogSource::OxrsysRuntime).unwrap(),
        serde_json::json!({"kind": "oxrsysRuntime"})
    );
    assert_eq!(
        serde_json::to_value(LogSource::AlvrSession).unwrap(),
        serde_json::json!({"kind": "alvrSession"})
    );
}

#[test]
fn resolve_source_maps_the_two_fixed_oxrsys_paths_and_the_file_variant() {
    let dir = scratch("resolve-fixed");
    let paths = fixture_paths(&dir);
    assert_eq!(
        resolve_source(&paths, &LogSource::OxrsysRuntime),
        Some(paths.oxr_appsup.join("oxrsys-runtime.log"))
    );
    assert_eq!(
        resolve_source(&paths, &LogSource::AlvrSession),
        Some(paths.oxr_appsup.join("alvr/session_log.txt"))
    );
    let explicit = dir.join("some/past-run.log");
    assert_eq!(
        resolve_source(
            &paths,
            &LogSource::File {
                path: explicit.clone()
            }
        ),
        Some(explicit)
    );
}

#[test]
fn resolve_source_wine_console_is_none_when_nothing_matches() {
    let dir = scratch("resolve-none");
    let paths = fixture_paths(&dir);
    assert_eq!(resolve_source(&paths, &LogSource::WineConsole), None);
}

#[test]
fn resolve_source_wine_console_falls_back_to_persisted_state_log_if_it_exists() {
    let dir = scratch("resolve-state");
    let paths = fixture_paths(&dir);
    std::fs::create_dir_all(paths.logs_dir()).unwrap();
    std::fs::create_dir_all(&paths.sabrage_appsup).unwrap();

    let state_log = paths.logs_dir().join("beatsaber-fromstate.log");
    std::fs::write(&state_log, b"x\n").unwrap();
    // A newer file also sits in logs/ — the persisted state must still win
    // over "newest", per priority order.
    let newer = paths.logs_dir().join("beatsaber-newer.log");
    std::fs::write(&newer, b"y\n").unwrap();

    let json = format!(
        r#"{{"version":1,"runId":"00000000-0000-0000-0000-000000000000","bottle":"Steam","bsDir":"/games/bs","startedAtUnixMs":0,"logPath":"{}"}}"#,
        state_log.display()
    );
    std::fs::write(paths.session_state_path(), json).unwrap();

    assert_eq!(
        resolve_source(&paths, &LogSource::WineConsole),
        Some(state_log)
    );
}

#[test]
fn resolve_source_wine_console_skips_a_persisted_state_whose_log_file_is_gone() {
    let dir = scratch("resolve-state-gone");
    let paths = fixture_paths(&dir);
    std::fs::create_dir_all(paths.logs_dir()).unwrap();
    std::fs::create_dir_all(&paths.sabrage_appsup).unwrap();

    let missing_log = paths.logs_dir().join("beatsaber-vanished.log"); // never created
    let json = format!(
        r#"{{"version":1,"runId":"00000000-0000-0000-0000-000000000000","bottle":"Steam","bsDir":"/games/bs","startedAtUnixMs":0,"logPath":"{}"}}"#,
        missing_log.display()
    );
    std::fs::write(paths.session_state_path(), json).unwrap();

    let real_log = paths.logs_dir().join("beatsaber-real.log");
    std::fs::write(&real_log, b"ok\n").unwrap();

    assert_eq!(
        resolve_source(&paths, &LogSource::WineConsole),
        Some(real_log)
    );
}

#[test]
fn resolve_source_wine_console_falls_back_to_the_newest_past_run() {
    let dir = scratch("resolve-newest");
    let paths = fixture_paths(&dir);
    std::fs::create_dir_all(paths.logs_dir()).unwrap();

    let older = paths.logs_dir().join("beatsaber-20260101-000000.log");
    let newer = paths.logs_dir().join("beatsaber-20260201-000000.log");
    std::fs::write(&older, b"a\n").unwrap();
    std::fs::write(&newer, b"b\n").unwrap();
    set_mtime(&older, 1_000_000_000);
    set_mtime(&newer, 2_000_000_000);

    assert_eq!(resolve_source(&paths, &LogSource::WineConsole), Some(newer));
}

#[test]
fn resolve_source_wine_console_prefers_the_live_session_over_everything() {
    // Touches the process-global LIVE_SESSION slot; self-contained
    // set→read→clear within one test, matching `session::mod`'s own test.
    use crate::process::ProcInfo;
    use crate::session::{clear_live_session, set_live_session, LiveSessionHandle};

    let dir = scratch("resolve-live");
    let paths = fixture_paths(&dir);
    std::fs::create_dir_all(paths.logs_dir()).unwrap();

    let live_log = paths.logs_dir().join("beatsaber-live.log");
    std::fs::write(&live_log, b"live\n").unwrap();
    let stale_log = paths.logs_dir().join("beatsaber-stale.log");
    std::fs::write(&stale_log, b"stale\n").unwrap();
    set_mtime(&stale_log, 9_999_999_999); // newer than live_log's mtime, must still lose

    let run_id = Uuid::new_v4();
    set_live_session(LiveSessionHandle {
        run_id,
        bottle: "Steam".into(),
        identity: ProcInfo {
            pid: 1,
            start_time: 1,
            exe: PathBuf::from("/bin/true"),
        },
        log_path: live_log.clone(),
        started_at_unix_ms: 0,
        cancel: CancellationToken::new(),
        detach: CancellationToken::new(),
    });

    let resolved = resolve_source(&paths, &LogSource::WineConsole);
    clear_live_session(run_id);

    assert_eq!(resolved, Some(live_log));
}

#[test]
fn list_past_runs_lists_newest_first_and_ignores_non_matching_entries() {
    let dir = scratch("past-runs");
    std::fs::write(dir.join("beatsaber-20260101-000000.log"), b"a").unwrap();
    std::fs::write(dir.join("beatsaber-20260201-000000.log"), b"bb").unwrap();
    std::fs::write(dir.join("notes.txt"), b"irrelevant").unwrap();
    std::fs::create_dir_all(dir.join("beatsaber-adir.log")).unwrap(); // a directory: must be skipped

    set_mtime(&dir.join("beatsaber-20260101-000000.log"), 1_000_000_000);
    set_mtime(&dir.join("beatsaber-20260201-000000.log"), 2_000_000_000);

    let runs = list_past_runs(&dir);
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].file_name, "beatsaber-20260201-000000.log");
    assert_eq!(runs[1].file_name, "beatsaber-20260101-000000.log");
    assert_eq!(runs[0].size, 2);
}

#[test]
fn list_past_runs_is_empty_for_a_missing_directory() {
    assert!(list_past_runs(Path::new("/does/not/exist/at/all/sabrage-test")).is_empty());
}

#[test]
fn poll_returns_newly_appended_lines() {
    let dir = scratch("append");
    let path = dir.join("a.log");
    std::fs::write(&path, b"line1\nline2\n").unwrap();

    let mut t = Tailer::open(&path, false, 0).unwrap();
    let b1 = t.poll().unwrap();
    assert_eq!(b1.lines, vec!["line1", "line2"]);
    assert!(!b1.rotated);
    assert!(!b1.truncated);

    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(f, "line3").unwrap();

    let b2 = t.poll().unwrap();
    assert_eq!(b2.lines, vec!["line3"]);
}

#[test]
fn poll_detects_rotation_by_rename() {
    let dir = scratch("rotate");
    let path = dir.join("a.log");
    std::fs::write(&path, b"old1\nold2\n").unwrap();

    let mut t = Tailer::open(&path, false, 0).unwrap();
    let b1 = t.poll().unwrap();
    assert_eq!(b1.lines, vec!["old1", "old2"]);

    std::fs::rename(&path, dir.join("a.log.bak")).unwrap();
    std::fs::write(&path, b"new1\n").unwrap();

    let b2 = t.poll().unwrap();
    assert!(b2.rotated);
    assert_eq!(b2.lines, vec!["new1"]);
}

#[test]
fn poll_detects_in_place_truncation() {
    let dir = scratch("truncate");
    let path = dir.join("a.log");
    std::fs::write(&path, b"aaaaaaaaaa\nbbbbbbbbbb\n").unwrap();

    let mut t = Tailer::open(&path, false, 0).unwrap();
    let b1 = t.poll().unwrap();
    assert_eq!(b1.lines, vec!["aaaaaaaaaa", "bbbbbbbbbb"]);
    assert!(!b1.rotated);

    // Same path truncated shorter than our offset (logrotate copytruncate).
    std::fs::write(&path, b"short\n").unwrap();

    let b2 = t.poll().unwrap();
    assert!(
        b2.rotated,
        "shrinking below the last offset must be treated as rotation"
    );
    assert_eq!(b2.lines, vec!["short"]);
}

#[test]
fn a_trailing_partial_line_is_buffered_until_its_newline_arrives() {
    let dir = scratch("partial");
    let path = dir.join("a.log");
    std::fs::write(&path, b"complete\nhalf").unwrap();

    let mut t = Tailer::open(&path, false, 0).unwrap();
    let b1 = t.poll().unwrap();
    assert_eq!(
        b1.lines,
        vec!["complete"],
        "the unterminated tail must not show yet"
    );

    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(f, "-line").unwrap();

    let b2 = t.poll().unwrap();
    assert_eq!(b2.lines, vec!["half-line"]);
}

#[test]
fn open_from_end_preloads_the_last_tail_lines_lines() {
    let dir = scratch("preload");
    let path = dir.join("a.log");
    let content: String = (1..=10).map(|n| format!("line{n}\n")).collect();
    std::fs::write(&path, content.as_bytes()).unwrap();

    let mut t = Tailer::open(&path, true, 3).unwrap();
    let b1 = t.poll().unwrap();
    assert_eq!(b1.lines, vec!["line8", "line9", "line10"]);

    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(f, "line11").unwrap();

    let b2 = t.poll().unwrap();
    assert_eq!(b2.lines, vec!["line11"]);
}

#[test]
fn open_from_end_with_zero_tail_lines_starts_at_eof() {
    let dir = scratch("preload-zero");
    let path = dir.join("a.log");
    std::fs::write(&path, b"old1\nold2\n").unwrap();

    let mut t = Tailer::open(&path, true, 0).unwrap();
    let b1 = t.poll().unwrap();
    assert!(b1.lines.is_empty(), "nothing preloaded, nothing new yet");
}

#[test]
fn open_and_poll_tolerate_a_file_that_does_not_exist_yet() {
    let dir = scratch("missing");
    let path = dir.join("not-yet.log");

    let mut t = Tailer::open(&path, false, 5).unwrap();
    let b1 = t.poll().unwrap();
    assert!(b1.lines.is_empty() && !b1.rotated);

    std::fs::write(&path, b"first\n").unwrap();
    let b2 = t.poll().unwrap();
    assert_eq!(b2.lines, vec!["first"]);
    assert!(
        b2.rotated,
        "a file appearing for the first time counts as a fresh open"
    );
}

#[test]
fn a_burst_over_the_cap_is_deferred_not_dropped() {
    let dir = scratch("burst");
    let path = dir.join("a.log");
    let content: String = (0..2500).map(|n| format!("l{n}\n")).collect();
    std::fs::write(&path, content.as_bytes()).unwrap();

    let mut t = Tailer::open(&path, false, 0).unwrap();
    let b1 = t.poll().unwrap();
    assert_eq!(b1.lines.len(), MAX_LINES_PER_POLL);
    assert!(b1.truncated);
    assert_eq!(b1.lines[0], "l0");

    let b2 = t.poll().unwrap();
    assert_eq!(b2.lines.len(), 500);
    assert!(!b2.truncated);
    assert_eq!(b2.lines[0], "l2000");
    assert_eq!(*b2.lines.last().unwrap(), "l2499");
}

/// A poll must not read the whole file: `MAX_LINES_PER_POLL` caps what is
/// *delivered*, `POLL_BYTE_BUDGET` caps what is *read*. A file far larger
/// than the budget is drained over several polls, in order and complete.
#[test]
fn one_poll_reads_at_most_the_byte_budget_and_still_delivers_every_line() {
    let dir = scratch("byte-budget");
    let path = dir.join("a.log");
    // ~3 MiB of 128-byte lines: three budgets' worth.
    let line = "x".repeat(127);
    let count = (3 * POLL_BYTE_BUDGET) / 128;
    let content: String = (0..count).map(|_| format!("{line}\n")).collect();
    std::fs::write(&path, content.as_bytes()).unwrap();
    let size = std::fs::metadata(&path).unwrap().len();

    let mut t = Tailer::open(&path, false, 0).unwrap();
    let b1 = t.poll().unwrap();
    assert!(
        t.offset <= POLL_BYTE_BUDGET as u64,
        "one poll read {} bytes of a {size}-byte file",
        t.offset
    );
    assert!(b1.truncated, "the caller is told there is more to come");
    assert_eq!(b1.lines.len(), MAX_LINES_PER_POLL);

    // Every line still arrives, in order, across as many polls as it takes.
    let mut delivered = b1.lines.len();
    for _ in 0..1000 {
        if delivered == count {
            break;
        }
        let b = t.poll().unwrap();
        assert!(b.lines.iter().all(|l| *l == line));
        delivered += b.lines.len();
    }
    assert_eq!(delivered, count, "nothing was dropped");
    assert_eq!(t.offset, size, "and the whole file was eventually read");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A file with no delimiter at all must not grow the splitter's buffer
/// without limit: past `MAX_UNTERMINATED_LINE_BYTES` the partial is broken
/// into a line of its own.
#[test]
fn a_single_line_with_no_newline_is_broken_at_the_bound() {
    let dir = scratch("no-newline");
    let path = dir.join("a.log");
    std::fs::write(&path, vec![b'z'; 2 * MAX_UNTERMINATED_LINE_BYTES + 4096]).unwrap();

    let mut t = Tailer::open(&path, false, 0).unwrap();
    let mut lines = 0usize;
    for _ in 0..16 {
        lines += t.poll().unwrap().lines.len();
    }
    assert!(
        lines >= 2,
        "the unterminated run must have been broken, not buffered whole"
    );
    assert!(
        t.unterminated < MAX_UNTERMINATED_LINE_BYTES,
        "the counter is reset by each forced break"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A8-7: ALVR opens `session_log.txt` with `.truncate(true)`, so a new
/// session keeps the inode. If it writes back past our cursor before the
/// next poll, neither the inode check nor `len < offset` sees anything —
/// only the bytes before the cursor say the file is not the one we were
/// reading.
#[test]
fn truncate_and_regrow_past_the_cursor_between_polls_reports_rotation() {
    let cases: &[(&str, usize)] = &[("equal", 10), ("larger", 30)];
    for (regrow, new_lines) in cases {
        let dir = scratch(&format!("rewrite-{regrow}"));
        let path = dir.join("a.log");
        let old: String = (1..=10)
            .map(|n| format!("OLD session line {n}\n"))
            .collect();
        std::fs::write(&path, old.as_bytes()).unwrap();

        // Opened from the end: the cursor sits at EOF, exactly where a
        // live pane on `session_log.txt` sits.
        let mut t = Tailer::open(&path, true, 5).unwrap();
        let _ = t.poll().unwrap();

        let new: String = (1..=*new_lines)
            .map(|n| format!("NEW session line {n}\n"))
            .collect();
        // In place, same inode — `OpenOptions::truncate`, not remove+create.
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&path)
                .unwrap();
            f.write_all(new.as_bytes()).unwrap();
        }

        let b = t.poll().unwrap();
        assert!(b.rotated, "{regrow}: the rewrite must be reported");
        assert_eq!(
            b.lines.first().map(String::as_str),
            Some("NEW session line 1"),
            "{regrow}: the new session's FIRST line must not be skipped"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// The signature check must not cry rotation on an ordinary append — the
/// bytes before the cursor are unchanged, which is the whole point.
#[test]
fn an_ordinary_append_is_never_mistaken_for_a_rewrite() {
    let dir = scratch("append-not-rotation");
    let path = dir.join("a.log");
    std::fs::write(&path, b"one\n").unwrap();

    let mut t = Tailer::open(&path, false, 0).unwrap();
    assert_eq!(t.poll().unwrap().lines, vec!["one"]);
    for n in 2..=20 {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(f, "line{n}").unwrap();
        let b = t.poll().unwrap();
        assert!(!b.rotated, "append {n} must not read as a rotation");
        assert_eq!(b.lines, vec![format!("line{n}")]);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// F14 (finding 14): a backlog queued in `pending` from *before* the path
/// vanished must survive a vanish-then-reappear cycle, even when that
/// backlog itself spans more than one `MAX_LINES_PER_POLL`-capped poll —
/// the exact shape that let the old code's unconditional
/// `self.pending.clear()` in the reopen branch discard real, already-read
/// content no caller had seen yet.
#[test]
fn a_backlog_queued_before_a_vanish_survives_reappearance_uncut() {
    let dir = scratch("vanish-reappear");
    let path = dir.join("a.log");
    // 4500 lines: one poll's cap (2000) leaves a remainder (2500) that
    // itself does not fit in a single further poll either.
    let content: String = (0..4500).map(|n| format!("l{n}\n")).collect();
    std::fs::write(&path, content.as_bytes()).unwrap();

    let mut t = Tailer::open(&path, false, 0).unwrap();

    let b1 = t.poll().unwrap();
    assert_eq!(b1.lines.len(), MAX_LINES_PER_POLL);
    assert!(b1.truncated);
    assert_eq!(b1.lines[0], "l0");
    assert_eq!(*b1.lines.last().unwrap(), "l1999");

    // The file vanishes with 2500 lines still queued in `pending`.
    std::fs::remove_file(&path).unwrap();

    let b2 = t.poll().unwrap();
    assert!(!b2.rotated, "a vanish alone is not a rotation");
    assert_eq!(b2.lines.len(), MAX_LINES_PER_POLL);
    assert!(
        b2.truncated,
        "500 lines of the original backlog are still queued"
    );
    assert_eq!(b2.lines[0], "l2000");
    assert_eq!(*b2.lines.last().unwrap(), "l3999");

    // It reappears as a new file — genuinely rotated — while 500 lines
    // from the vanished original are still sitting in `pending`.
    std::fs::write(&path, b"new1\n").unwrap();

    let b3 = t.poll().unwrap();
    assert!(
        !b3.rotated,
        "A8-4: these are the OLD file's lines. Announcing the rotation here \
             makes the consumer clear its buffer and then label `l4000..l4499` \
             as the new file's beginning"
    );
    assert!(
        !b3.truncated,
        "the whole remaining backlog fit under the cap"
    );
    assert_eq!(
        b3.lines.len(),
        500,
        "the pre-vanish backlog must be delivered, not dropped by the reopen"
    );
    assert_eq!(b3.lines[0], "l4000");
    assert_eq!(*b3.lines.last().unwrap(), "l4499");

    // …and the marker travels with the batch that really does begin the
    // new incarnation.
    let b4 = t.poll().unwrap();
    assert!(b4.rotated, "A8-4: THIS batch is the new file's first");
    assert_eq!(b4.lines, vec!["new1"]);

    // One rotation, one marker: the ordinary appends that follow are
    // continuations.
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(f, "new2").unwrap();
    }
    let b5 = t.poll().unwrap();
    assert!(!b5.rotated);
    assert_eq!(b5.lines, vec!["new2"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A8-4, the consumer's half: `Logs.svelte` clears its buffer on
/// `rotated` and then appends the batch. Replaying that contract over the
/// sequence above must leave no line of the previous incarnation on
/// screen — and must not have shown the new file's first lines appended
/// under the *old* file's tail either.
#[test]
fn a_rotation_marker_partitions_the_batches_into_clean_incarnations() {
    let dir = scratch("rotation-epochs");
    let path = dir.join("a.log");
    std::fs::write(&path, b"old1\nold2\nold3\n").unwrap();

    let mut t = Tailer::open(&path, false, 0).unwrap();
    // `Logs.svelte`'s `onBatch`, in three lines.
    fn show(screen: &mut Vec<String>, b: &LogBatch) {
        if b.rotated {
            screen.clear();
        }
        screen.extend(b.lines.iter().cloned());
    }
    let mut screen: Vec<String> = Vec::new();

    let b1 = t.poll().unwrap();
    show(&mut screen, &b1);
    assert_eq!(screen, vec!["old1", "old2", "old3"]);

    // Rewritten in place, past the cursor — ALVR's `.truncate(true)`.
    std::fs::write(&path, b"new1\nnew2\nnew3\nnew4\n").unwrap();
    let b2 = t.poll().unwrap();
    show(&mut screen, &b2);
    assert!(b2.rotated);
    assert_eq!(
        screen,
        vec!["new1", "new2", "new3", "new4"],
        "not one line of the previous session survived the rotation"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A8-7: the continuity witness is read *before* the data. A truncate that
/// lands between the two used to be read as a continuation — the tailer
/// kept the stale cursor, overwrote its witness with bytes from the new
/// file's suffix, and skipped the new file's whole prefix for good.
#[test]
fn a_rewrite_landing_inside_the_read_window_is_not_read_as_a_continuation() {
    let dir = scratch("read-window-race");
    let path = dir.join("a.log");
    let first: String = (0..200).map(|n| format!("old{n}\n")).collect();
    std::fs::write(&path, first.as_bytes()).unwrap();

    let mut t = Tailer::open(&path, false, 0).unwrap();
    let b1 = t.poll().unwrap();
    assert_eq!(b1.lines.len(), 200);

    // Append, then arrange for the file to be rewritten in place *after*
    // the read of that append and before the post-read check — the exact
    // window the precheck cannot cover.
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(f, "old200").unwrap();
    }
    let rewritten: String = (0..300).map(|n| format!("new{n}\n")).collect();
    let p = path.clone();
    on_next_read(move || {
        // Same inode, grown back past the old cursor: `truncate(true)`,
        // never a rename.
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&p)
            .unwrap();
        f.write_all(rewritten.as_bytes()).unwrap();
    });

    let b2 = t.poll().unwrap();
    clear_read_hook();
    assert!(
        b2.lines.is_empty(),
        "the straddled read is undone whole, not delivered: {:?}",
        b2.lines
    );
    assert!(!b2.rotated, "nothing from the new incarnation is out yet");

    // The next poll reopens from byte 0 and delivers the new file
    // entire, prefix included.
    let b3 = t.poll().unwrap();
    assert!(b3.rotated, "and THIS batch begins the new incarnation");
    assert_eq!(b3.lines.len(), 300);
    assert_eq!(b3.lines[0], "new0");
    assert_eq!(*b3.lines.last().unwrap(), "new299");
    std::fs::remove_dir_all(&dir).unwrap();
}
