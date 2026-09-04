use super::*;
use crate::events::step;
use std::sync::Mutex as StdMutex;
use uuid::Uuid;

fn split(input: &[&[u8]]) -> Vec<String> {
    let mut out = Vec::new();
    let mut s = ChunkSplitter::new();
    for part in input {
        s.push(part, &mut |c| out.push(c));
    }
    s.finish(&mut |c| out.push(c));
    out
}

#[test]
fn splits_on_lf_and_cr() {
    // curl's progress bar: CR-separated repaints, no trailing newline.
    assert_eq!(split(&[b"10%\r50%\r100%"]), vec!["10%", "50%", "100%"]);
    assert_eq!(split(&[b"a\n\nb\n"]), vec!["a", "", "b"]);
    assert_eq!(split(&[b"partial"]), vec!["partial"]);
    assert!(split(&[b""]).is_empty());
}

fn split_ends(input: &[&[u8]]) -> Vec<(String, ChunkEnd)> {
    let mut out = Vec::new();
    let mut s = ChunkSplitter::new();
    for part in input {
        s.push_with(part, &mut |c, e| out.push((c, e)));
    }
    s.finish_with(&mut |c, e| out.push((c, e)));
    out
}

/// The terminator is what a byte-faithful renderer puts back: `println!`
/// on a CR-terminated repaint is what turns curl's one line into hundreds.
#[test]
fn chunks_carry_their_terminator() {
    assert_eq!(
        split_ends(&[b"a\nb\n"]),
        vec![
            ("a".to_string(), ChunkEnd::Lf),
            ("b".to_string(), ChunkEnd::Lf)
        ]
    );
    // CRLF is one Lf terminator, not a CR followed by a phantom chunk.
    assert_eq!(
        split_ends(&[b"a\r\nb\r\n"]),
        vec![
            ("a".to_string(), ChunkEnd::Lf),
            ("b".to_string(), ChunkEnd::Lf)
        ]
    );
    // curl's repaints, including the last one before EOF.
    assert_eq!(
        split_ends(&[b"10%\r50%\r"]),
        vec![
            ("10%".to_string(), ChunkEnd::Cr),
            ("50%".to_string(), ChunkEnd::Cr)
        ]
    );
    // A final chunk with no delimiter must not gain one.
    assert_eq!(
        split_ends(&[b"partial"]),
        vec![("partial".to_string(), ChunkEnd::Eof)]
    );
    // Straddling reads, including across the CRLF pair.
    assert_eq!(
        split_ends(&[b"ab\r", b"\ncd\n"]),
        vec![
            ("ab".to_string(), ChunkEnd::Lf),
            ("cd".to_string(), ChunkEnd::Lf)
        ]
    );
}

fn collecting_sink() -> (EventSink, Arc<StdMutex<Vec<StageEvent>>>) {
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let s = seen.clone();
    let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
    (sink, seen)
}

fn spec(program: &str, run_id: Uuid) -> ChildSpec {
    ChildSpec::new(program, step::BUILD_TOOLS, run_id)
}

#[tokio::test]
async fn streams_output_and_reports_exit_status() {
    let run_id = Uuid::new_v4();
    let (sink, seen) = collecting_sink();
    let cancel = CancellationToken::new();
    let s = spec("/bin/sh", run_id)
        .arg("-c")
        .arg("printf 'one\\ntwo\\n'; printf 'err\\n' >&2");
    let status = spawn_streamed(&s, &sink, &cancel).await.unwrap();
    assert!(status.success());

    let evs = seen.lock().unwrap().clone();
    let mut out: Vec<(Stream, String)> = evs
        .iter()
        .filter_map(|e| match e {
            StageEvent::Output {
                stream,
                chunk,
                step,
                run_id: r,
                ..
            } => {
                assert_eq!(step, step::BUILD_TOOLS);
                assert_eq!(*r, run_id);
                Some((*stream, chunk.clone()))
            }
            _ => None,
        })
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    assert_eq!(
        out,
        vec![
            (Stream::Stderr, "err".to_string()),
            (Stream::Stdout, "one".to_string()),
            (Stream::Stdout, "two".to_string()),
        ]
    );
}

/// A14-3 pins that `StageEvent::Output` carries each chunk's
/// [`ChunkEnd`] rather than computing and discarding it: a `\r` progress
/// repaint, a `\n`-terminated line and the final unterminated chunk
/// ([`ChunkEnd::Eof`]) must stay distinguishable downstream.
#[tokio::test]
async fn output_events_carry_their_chunk_terminator() {
    let run_id = Uuid::new_v4();
    let (sink, seen) = collecting_sink();
    let cancel = CancellationToken::new();
    // No trailing newline after "100%" — a real progress bar's last
    // repaint before the child exits.
    let s = spec("/bin/sh", run_id)
        .arg("-c")
        .arg("printf '10%%\\r50%%\\r100%%'; printf 'done\\n'");
    let status = spawn_streamed(&s, &sink, &cancel).await.unwrap();
    assert!(status.success());

    let ends: Vec<(String, ChunkEnd)> = seen
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            StageEvent::Output { chunk, end, .. } => Some((chunk.clone(), *end)),
            _ => None,
        })
        .collect();
    assert_eq!(
        ends,
        vec![
            ("10%".to_string(), ChunkEnd::Cr),
            ("50%".to_string(), ChunkEnd::Cr),
            ("100%done".to_string(), ChunkEnd::Lf),
        ],
        "{ends:?}"
    );
}

#[tokio::test]
async fn run_ok_attaches_the_output_tail_on_failure() {
    let run_id = Uuid::new_v4();
    let (sink, _seen) = collecting_sink();
    let cancel = CancellationToken::new();
    let s = spec("/bin/sh", run_id)
        .arg("-c")
        .arg("echo boom >&2; exit 3");
    let err = run_ok(&s, &sink, &cancel).await.unwrap_err();
    match &err {
        SabrageError::ChildFailed {
            argv0,
            status,
            tail,
        } => {
            assert_eq!(argv0, "/bin/sh");
            assert_eq!(*status, 3);
            assert_eq!(tail, &vec!["boom".to_string()]);
        }
        other => panic!("expected ChildFailed, got {other:?}"),
    }
    assert_eq!(err.exit_code(), 1);
    assert_eq!(err.kind(), "child_failed");
}

/// `spawn_streamed` never reads the tail ([`SabrageError::ChildFailed`] is
/// `run_ok`'s error, not its), so `spawn_streamed_inner` must not spend a
/// clone per output chunk populating one nobody looks at.
#[tokio::test]
async fn spawn_streamed_does_not_populate_a_tail_nobody_reads() {
    let run_id = Uuid::new_v4();
    let (sink, _seen) = collecting_sink();
    let cancel = CancellationToken::new();
    let s = spec("/bin/sh", run_id)
        .arg("-c")
        .arg("echo boom >&2; exit 3");
    let (status, tail) = spawn_streamed_inner(&s, &sink, &cancel, false)
        .await
        .unwrap();
    assert_eq!(exit_code_of(status), 3);
    assert!(tail.is_empty(), "tail should be empty: {tail:?}");
}

#[tokio::test]
async fn cancellation_kills_the_process_group() {
    let run_id = Uuid::new_v4();
    let (sink, _seen) = collecting_sink();
    let cancel = CancellationToken::new();
    let s = spec("/bin/sh", run_id)
        .arg("-c")
        .arg("sleep 60")
        .kill_grace(Duration::from_millis(300));
    let token = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(120)).await;
        token.cancel();
    });
    let err = spawn_streamed(&s, &sink, &cancel).await.unwrap_err();
    assert!(matches!(err, SabrageError::Cancelled));
    assert_eq!(err.exit_code(), 130);
}

/// The SIGKILL escalation watches the process *group*, not the leader: a
/// descendant that ignores SIGTERM (an ignored disposition survives
/// `exec`) keeps the pipes open long after the leader is reaped, so
/// `spawn_streamed` must not block on that EOF.
#[tokio::test]
async fn cancellation_escalates_when_a_descendant_outlives_the_leader() {
    let run_id = Uuid::new_v4();
    let (sink, _seen) = collecting_sink();
    let cancel = CancellationToken::new();
    let s = spec("/bin/sh", run_id)
        .arg("-c")
        .arg("(trap '' TERM; sleep 30) & sleep 30")
        .kill_grace(Duration::from_millis(300));
    let token = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        token.cancel();
    });
    let started = tokio::time::Instant::now();
    let err = tokio::time::timeout(Duration::from_secs(10), spawn_streamed(&s, &sink, &cancel))
        .await
        .expect("spawn_streamed must not wait for the ignoring descendant")
        .unwrap_err();
    assert!(matches!(err, SabrageError::Cancelled));
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "took {:?}",
        started.elapsed()
    );
}

/// The descendant that *releases* the pipes is the harder half of the same
/// hazard: both pumps reach EOF, so an escalation conditioned on the drain
/// timing out never fires and a TERM-ignoring process survives Cancel while
/// still doing its work. Group liveness, not pipe EOF, is what decides.
#[tokio::test]
async fn cancellation_kills_a_descendant_that_ignored_term_and_released_the_pipes() {
    let dir = std::env::temp_dir().join(format!(
        "sabrage-proc-escalate-{}-{}",
        std::process::id(),
        Uuid::new_v4().as_simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let pidfile = dir.join("pid");
    let run_id = Uuid::new_v4();
    let (sink, _seen) = collecting_sink();
    let cancel = CancellationToken::new();
    let s = spec("/bin/sh", run_id)
        .arg("-c")
        .arg(format!(
            "( trap '' TERM; exec >/dev/null 2>&1; exec sleep 30 ) & echo $! > {}; sleep 30",
            pidfile.display()
        ))
        .kill_grace(Duration::from_millis(300));
    let token = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        token.cancel();
    });
    let err = tokio::time::timeout(Duration::from_secs(10), spawn_streamed(&s, &sink, &cancel))
        .await
        .expect("spawn_streamed must not wait for the ignoring descendant")
        .unwrap_err();
    assert!(matches!(err, SabrageError::Cancelled));

    let pid: u32 = std::fs::read_to_string(&pidfile)
        .expect("the child recorded its descendant")
        .trim()
        .parse()
        .expect("a pid");
    // The SIGKILL is delivered before the call returns; reaping the
    // reparented corpse is the kernel's own business, so poll briefly.
    for _ in 0..100 {
        if !is_alive(pid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    if is_alive(pid) {
        let _ = kill_hard(pid);
        panic!("descendant {pid} survived cancellation");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Same hazard without any cancellation: a child that backgrounds a
/// descendant holding stdout returns its status now, not when the orphan
/// finally exits. Nothing is killed — the surviving process is often one the
/// pipeline wanted (wine's `reg add` starts wineserver).
#[tokio::test]
async fn a_backgrounded_descendant_does_not_wedge_the_stage() {
    let run_id = Uuid::new_v4();
    let (sink, seen) = collecting_sink();
    let cancel = CancellationToken::new();
    let s = spec("/bin/sh", run_id)
        .arg("-c")
        .arg("(sleep 30) & printf 'done\\n'; exit 0");
    let status = tokio::time::timeout(Duration::from_secs(10), spawn_streamed(&s, &sink, &cancel))
        .await
        .expect("the orphan must not hold the stage open")
        .unwrap();
    assert!(status.success());
    // The output written before the leader exited is still delivered.
    let evs = seen.lock().unwrap().clone();
    assert!(
        evs.iter().any(|e| matches!(
            e,
            StageEvent::Output { chunk, .. } if chunk == "done"
        )),
        "output lost: {evs:?}"
    );
}

#[tokio::test]
async fn a_probe_that_never_answers_times_out_instead_of_hanging() {
    let spec = ChildSpec::new("/bin/sleep", step::BUILD_TOOLS, Uuid::nil()).arg("60");
    let started = tokio::time::Instant::now();
    let err = capture_with(&spec, &CancellationToken::new(), Duration::from_millis(200))
        .await
        .unwrap_err();
    // Degrades exactly like a missing binary, which every caller handles.
    assert_eq!(err.kind(), "io");
    assert!(started.elapsed() < Duration::from_secs(5));
    match err {
        SabrageError::Io { source, .. } => {
            assert_eq!(source.kind(), std::io::ErrorKind::TimedOut)
        }
        other => panic!("expected Io, got {other:?}"),
    }
}

#[tokio::test]
async fn a_cancelled_probe_returns_promptly() {
    let spec = ChildSpec::new("/bin/sleep", step::BUILD_TOOLS, Uuid::nil()).arg("60");
    let cancel = CancellationToken::new();
    cancel.cancel();
    let started = tokio::time::Instant::now();
    let err = capture_with(&spec, &cancel, DEFAULT_PROBE_TIMEOUT)
        .await
        .unwrap_err();
    assert!(matches!(err, SabrageError::Cancelled));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn exit_code_conventions() {
    assert_eq!(exit_code_of(exit_status_from_code(0)), 0);
    assert_eq!(exit_code_of(exit_status_from_code(7)), 7);
    assert!(exit_status_from_code(0).success());
    assert!(!exit_status_from_code(1).success());
}

#[test]
fn finds_this_test_binary_by_its_exe_path() {
    let exe = std::env::current_exe().expect("test binary path");
    let found = find_processes_by_exe(&exe);
    let me = std::process::id();
    assert!(
        found.iter().any(|p| p.pid == me),
        "own pid {me} not found among {found:?}"
    );
    assert!(find_processes_by_exe(Path::new("/nonexistent/sabrage/helper")).is_empty());
}

/// `find_processes_by_exe`/`find_processes_by_cmdline` are thin wrappers
/// over one [`ProcessScan`]; a caller sharing a scan across several
/// needles (`stages::stop`) must see exactly the same matches the
/// single-needle convenience functions would.
#[test]
fn process_scan_agrees_with_the_single_needle_convenience_functions() {
    let exe = std::env::current_exe().expect("test binary path");
    let name = exe.file_name().and_then(|n| n.to_str()).unwrap();
    let needle = &name[name.len().saturating_sub(6)..];

    assert!(ProcessScan::scan()
        .by_exe(Path::new("/nonexistent/sabrage/helper"))
        .is_empty());

    // The convenience functions run their **own** scan, so a difference is
    // either a filter disagreement (what this test is about) or the process
    // table changing between the two walks (what it is not): under `cargo
    // test` every child a concurrent test spawns is briefly a copy of this
    // exe, between fork and exec. So the comparison is only made across a
    // window two bracketing scans agree on, and there the filters must match.
    for _ in 0..50 {
        let before = ProcessScan::scan();
        let fresh_exe = find_processes_by_exe(&exe);
        let fresh_cmd = find_processes_by_cmdline(needle);
        let after = ProcessScan::scan();

        if before.by_exe(&exe) != after.by_exe(&exe)
            || before.by_cmdline(needle) != after.by_cmdline(needle)
        {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        assert_eq!(before.by_exe(&exe), fresh_exe);
        assert_eq!(before.by_cmdline(needle), fresh_cmd);
        return;
    }
    panic!("the process table never sat still long enough to compare the two scans");
}

#[test]
fn default_child_path_puts_the_toolchains_first_and_dedupes() {
    let p = default_child_path();
    let parts: Vec<&str> = p.split(':').collect();
    assert_eq!(parts[0], "/opt/homebrew/bin");
    assert_eq!(parts[1], "/usr/local/bin");
    assert!(parts[2].ends_with("/.cargo/bin"));
    assert!(parts[3].ends_with("/Library/Android/sdk/platform-tools"));
    let unique: std::collections::BTreeSet<_> = parts.iter().collect();
    assert_eq!(unique.len(), parts.len(), "duplicate PATH entry");
}

#[test]
fn spec_builder_renders_a_display_command() {
    let s = spec("cmake", Uuid::nil())
        .args(["-S", "/repo/ext/oxrsys"])
        .arg("-B")
        .arg(PathBuf::from("/repo/ext/oxrsys/build-x64"))
        .cwd("/repo")
        .env("CMAKE_BUILD_PARALLEL_LEVEL", "8")
        .env_path("/opt/homebrew/bin");
    assert_eq!(
        s.display(),
        "cmake -S /repo/ext/oxrsys -B /repo/ext/oxrsys/build-x64"
    );
    assert_eq!(s.argv0(), "cmake");
    assert_eq!(s.cwd, Some(PathBuf::from("/repo")));
    assert_eq!(s.env_path.as_deref(), Some("/opt/homebrew/bin"));
    assert_eq!(s.kill_grace, DEFAULT_KILL_GRACE);
}
