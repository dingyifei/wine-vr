# ALVR patches for the oxrsys embedded backend

`ext/ALVR` is a git submodule pinned to the
[dingyifei/ALVR](https://github.com/dingyifei/ALVR) fork, branch
**oxrsys-v20.14.1** (based on upstream tag `v20.14.1`, `a9f6542`). The branch
commit *is* the patch set already applied — a fresh
`git submodule update --init --recursive` gives you patched sources with
nothing to apply. [`alvr-v20.14.1-oxrsys.patch`](alvr-v20.14.1-oxrsys.patch)
is a reviewable mirror of that branch: one file to read the whole delta
against stock ALVR, or to reapply onto a plain upstream checkout.

## Building

Use the direct-toolchain cargo (Homebrew rustup's shim resolves the wrong
cargo); `./demo.sh build` does this for you:

```sh
RUSTC=~/.rustup/toolchains/stable-aarch64-apple-darwin/bin/rustc \
~/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo \
  build -p alvr_server_core --release --target x86_64-apple-darwin
```

## What the patch contains

- `alvr/events/src/lib.rs` — serialize `GraphStatistics`/`StatisticsSummary`
  payloads into the `[GRAPH]`/`[STATS]` session_log lines (upstream logs them
  with an empty message; the data otherwise only reaches the dashboard
  websocket). Needed for offline latency analysis.
- `alvr/server_core/src/tracking/mod.rs`,
  `alvr/server_core/src/connection.rs` — the tracking and statistics stream
  receive loops retry on `ConnectionError::Other` instead of exiting the
  thread. Upstream exits permanently, so a headset sleep (socket goes quiet /
  errors) killed tracking until a full restart.
- `alvr/server_core/src/connection.rs` — `is_streaming_nonblocking()` for the
  CoreAudio capture callback: the blocking session-lock read deadlocked
  against `connection_pipeline` holding the write lock across cpal device
  enumeration (which needs the HAL mutex the callback's thread holds) —
  wedging every reconnect (macOS-only ABBA deadlock).
- `alvr/sockets/src/stream_socket.rs` + `connection.rs` — closeable stream
  writer: the shared UDP writer sits behind `Option` with `close_writer()`
  called on every disconnect flavor (receive-thread exits, teardown tail, an
  RAII guard for early error returns), so stale `StreamSender` Arc clones
  (e.g. the CoreAudio callback's) can no longer pin the port and break every
  re-bind with EADDRINUSE. Plus a lock-free `disconnecting` flag so the old
  receive thread exits without the session lock, and a bounded (~3 s)
  EADDRINUSE retry on the stream bind for normal close races.
- `alvr/sockets/src/stream_socket.rs` — bounded close: `close()` /
  `close_writer()` take the send mutex with `try_lock_for(250 ms)` instead of
  blocking, so a stalled TCP send cannot hang a caller holding the session
  write lock; on timeout the close is deferred to the sender drop (July 4
  review fix).
- `alvr/server_core/src/connection.rs` — video send errors are logged
  rate-limited (upstream ignored them silently); manual-IP dial failures are
  logged (upstream retried in total silence, indistinguishable from a hang).
- `alvr/sockets/src/lib.rs` — socket buffer sizing: `Maximum` requested
  `u32::MAX`, which macOS rejects with EINVAL, silently leaving the 9216-byte
  default; now starts at 8 MiB and halves until accepted.
- `alvr/audio/src/lib.rs` — capture stops on send error (releases its socket
  clone), and the stop-state poll is 50 ms (was 500 ms) so teardown can't
  race the next connection's bind.
- `alvr/adb/src/lib.rs` + `connection.rs` — `WiredConnectionStatus::NoDevice`
  falls through to manual-IP/discovery dialing instead of retrying the wired
  path forever, so a `client.wired` entry no longer starves WiFi connections
  when no USB device is attached.

## Regenerating the mirror

After changing the branch in `ext/ALVR`, refresh the patch file so the two
stay in sync:

```sh
(cd ext/ALVR && git diff v20.14.1 oxrsys-v20.14.1 > ../../patches/alvr-v20.14.1-oxrsys.patch)
```
