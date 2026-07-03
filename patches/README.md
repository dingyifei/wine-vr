# ALVR patches for the oxrsys embedded backend

`ext/ALVR` is a plain clone of [ALVR](https://github.com/alvr-org/ALVR) pinned at
tag **v20.14.1** (`a9f6542`, "Bump to 20.14.1"). It is not a git submodule and its
local modifications are not tracked anywhere else — this directory is the source
of truth for them.

## Applying on a fresh clone

```sh
git clone https://github.com/alvr-org/ALVR ext/ALVR
cd ext/ALVR
git checkout v20.14.1
git submodule update --init openvr
git apply ../../patches/alvr-v20.14.1-oxrsys.patch
```

Then build with the direct-toolchain cargo (Homebrew rustup's shim resolves the
wrong cargo):

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

Regenerate after changing anything in `ext/ALVR`:

```sh
cd ext/ALVR && git diff > ../../patches/alvr-v20.14.1-oxrsys.patch
```
