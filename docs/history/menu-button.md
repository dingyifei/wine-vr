# Menu button: investigation and resolution

**Status: RESOLVED** (2026-07-04, live-verified on Quest 3; oxrsys `466cbab`).

This is a condensed record. The full ~4,800-word findings — including three probe-report
appendices (oxrsys action-subsystem review, wineopenxr bridge audit, game-binary analysis) —
are in the git history of `docs/menu-button-investigation.md`.

## Resolution

Fixed to the maximum the game allows. After the fixes below, Beat Saber 1.29.4 pauses via
**X, A (legacy joystick bridge revived), and the Quest system button (focus-loss path,
previously structurally dead)**. The left MENU button still does not pause — and a three-way
investigation (Unity OpenXR plugin 1.5.3 source, community reports, game-binary analysis)
proved this is a **game/Unity limitation on ALL OpenXR runtimes** since the 1.29.4 OpenXR port
(May 2023): Unity's `OculusTouchControllerProfile` menuButton action is the only asymmetric one
(left = `menu/click`, right = `system/click`) and the plugin registers usages per-action, never
feeding the legacy `MenuButton → joystick button 6` mapping the game polls
(`MenuButtonOculusTouch`). Community threads confirm the same failure on SteamVR/Oculus
PC/Virtual Desktop from 1.29.4 through at least 1.34.2; X/A are the designed replacement.
oxrsys now matches real-runtime behavior exactly (and exceeds it: the system-button pause
works here, while many real setups lose it to the dashboard).

## Root cause

Two independent problems, one in oxrsys and one in the game.

**oxrsys: a fabricated interaction-profile lifecycle killed Unity's legacy-input joystick
bridge** — the only consumer of the menu button in Beat Saber 1.29.4. The OpenXR action
pipeline itself was healthy: `xrGetActionStateBoolean` genuinely returned
`true, active, changed-once, timestamped` for the left menu press on every frame. But the
game never consumes that action in managed code. Pause = `VRPlatformUtils.GetMenuButton[Down]`
→ legacy `Input.GetButton("MenuButtonOculusTouch")` → InputManager axis "joystick button 6"
← Unity's engine-internal XR-usage-to-legacy-joystick bridge. Sabers and triggers use the
Input System asset — a different consumer layer — which is why everything else worked while
pause alone was dead.

Causal chain: the game launches with no ALVR client connected → oxrsys fabricated a
`khr/simple_controller` profile for both hands (no real runtime does this) → Unity created
fake XR devices for the entire load window → the client connects ~30 s in → the profile
flapped `simple|simple` → `NULL|NULL` → `touch|touch` within 90 ms → Unity destroyed and
recreated both devices mid-run → the recreated touch devices' usages never re-fed the legacy
joystick buttons → button 6 stayed false forever → `GetMenuButtonDown()` never fired,
despite spec-perfect action data.

**Game/Unity: the menu button is unreachable even with a healthy bridge**, per the Resolution
above. The zero-code discriminating test ran first: on the old build, X/A did not pause,
confirming the dead bridge. After fixes 1+2 revived it, X/A paused but the menu button still
did not — the asymmetric binding means the left device's legacy MenuButton usage is never fed,
and the right hand's `system/click` has no game-side consumer either (the game binds no
InputManager axis to joystick button 7).

## Fixes landed in oxrsys (all verified live)

1. Pre-connect `khr/simple_controller` fabrication removed (config flag
   `simple_controller_fallback` retained for client-less testing).
2. Sticky per-client interaction profile + 1 s debounce on
   `XrEventDataInteractionProfileChanged` — exactly ONE profile event per connect; no events
   on overlay visits or sub-second blips (was: the simple→NULL→touch flap that made Unity
   destroy/recreate devices and permanently killed its legacy joystick bridge).
3. Focus emulation: FOCUSED→VISIBLE after 500 ms of streaming-input-gone, immediate restore;
   `xrSyncActions` returns `XR_SESSION_NOT_FOCUSED` with all states inactive per spec.
   (Previously the session state only ratcheted upward, so Unity's
   `OnApplicationFocus(false)` auto-pause — the path the Quest system button uses — was
   structurally dead.)
4. `/input/system/*` hardening: those bindings are reserved — accepted at suggest time, never
   bound or active (both polarities were live-tested, active-mirror and inactive-unbound;
   neither affects menu pause, confirming the game-side conclusion).

## Investigation notes worth keeping

- The "alternating true/false" reads that launched the investigation were per-spec correct
  behavior: Unity polls the menu action once per device per frame (left reads `menu/click` =
  true, right reads `system/click` = false, forever). It only looked pathological because of
  tracer bugs — the logger never printed `subactionPath`, poisoned its per-handle cache on
  the first read, and shared one static last-state across handles and subactions.
- The wineopenxr bridge was fully exonerated: it is a stateless, same-address-space
  pass-through with identical struct ABIs on both sides; action handles and path atoms are
  raw host values everywhere. Secondary findings noted for later: timespec PE→host `tv_nsec`
  garbage high-half, and event suppression for unknown sessions.
- Regression status from the same session: reconnect (sleep/wake), 72 fps pacing, and overlay
  handling all verified. Test suites extended (asymmetric-menu case, NOT_FOCUSED case, sticky
  profile lifecycle), and the API test suite no longer clobbers
  `~/Library/Application Support/OXRSys/oxrsys-runtime.toml` (snapshot/restore guard).

Key source files (post-fix, in the pinned submodule): `ext/oxrsys/runtime/src/InputManager.cpp`,
`Session.cpp`, `EntryPoint.cpp`, `ActionSet.cpp`. The round-6 launch logs cited in the full
report (`/tmp/bs_launch_round6d.log`, `/tmp/bs_launch_round6e.log`) are local artifacts, not
in the repo. The three individual probe reports (oxrsys deep review, wineopenxr bridge review,
game-consumption analysis) are preserved verbatim in the git history of
`docs/menu-button-investigation.md`.
