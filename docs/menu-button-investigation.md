# Menu-button investigation — full findings (2026-07-04)

## RESOLUTION (2026-07-04, live-verified on Quest 3)

**Fixed to the maximum the game allows.** After the fixes below, Beat Saber 1.29.4 pauses via
**X, A (legacy joystick bridge revived), and the Quest system button (focus-loss path,
previously structurally dead)**. The left MENU button still does not pause — and a three-way
investigation (Unity OpenXR plugin 1.5.3 source, community reports, game-binary analysis)
proved this is a **game/Unity limitation on ALL OpenXR runtimes** since the 1.29.4 OpenXR port
(May 2023): Unity's `OculusTouchControllerProfile` menuButton action is the only asymmetric one
(left=`menu/click`, right=`system/click`) and the plugin registers usages per-action, never
feeding the legacy `MenuButton -> joystick button 6` mapping the game polls
(`MenuButtonOculusTouch`). Community threads confirm the same failure on SteamVR/Oculus
PC/Virtual Desktop from 1.29.4 through at least 1.34.2; X/A are the designed replacement.
oxrsys now matches real-runtime behavior exactly (and exceeds it: the system-button pause
works here, while many real setups lose it to the dashboard).

**oxrsys fixes landed (verified live, X/A zero-code test ran first and confirmed the dead
bridge on the old build):**
1. Pre-connect `khr/simple_controller` fabrication removed (config flag
   `simple_controller_fallback` for client-less testing).
2. Sticky per-client interaction profile + 1 s debounce on
   `XrEventDataInteractionProfileChanged` — exactly ONE profile event per connect; no events
   on overlay visits or sub-second blips (was: simple→NULL→touch flap that made Unity
   destroy/recreate devices and permanently killed its legacy joystick bridge).
3. Focus emulation: FOCUSED→VISIBLE after 500 ms of streaming-input-gone, immediate restore;
   `xrSyncActions` returns `XR_SESSION_NOT_FOCUSED` with all states inactive per spec.
4. R3 hardening: `/input/system/*` bindings are reserved — accepted at suggest time, never
   bound/active (both polarities live-tested: active-mirror and inactive-unbound; neither
   affects menu pause, confirming the game-side conclusion).

Regression status: reconnect (sleep/wake), 72 fps pacing, and overlay handling all verified
in the same session. Test suites extended (asymmetric-menu case, NOT_FOCUSED case, sticky
lifecycle); the API test suite no longer clobbers `~/Library/Application
Support/OXRSys/oxrsys-runtime.toml` (snapshot/restore guard).

The original findings below stand, with one correction from live testing: fixes 1+2 alone
revive the legacy bridge for symmetric bindings (X/A pause) but cannot revive the menu
button — see section (5) R3 and the RESOLUTION above.

---

# ROOT-CAUSE VERDICT: Beat Saber pause button

## (1) Most probable root cause

**The OpenXR action pipeline is healthy — the defect is oxrsys's fabricated interaction-profile lifecycle, which kills Unity's legacy-input joystick bridge: the ONLY consumer of the menu button in Beat Saber 1.29.4.**

I independently re-verified the routing code (not just the reports): sync buckets by `binding.topLevelPath` atom (`EntryPoint.cpp:2380-2382`), stores per-subaction in `subactionData_[atom]` (`ActionSet.cpp:39-47, 61`), and queries by `getInfo->subactionPath` atom (`EntryPoint.cpp:2431` → `2071-2073`). Atoms live in a single host-owned namespace (wineopenxr passes them verbatim, proven stateless). A left-hand query structurally cannot read the right-hand bucket; there is no misattribution mechanism. Therefore **`xrGetActionStateBoolean(44, /user/hand/left)` genuinely returns `true, active, changed-once, timestamped` on every press.**

But Beat Saber's pause never consumes that action in managed code. Verified from game binaries: pause = `VRPlatformUtils.GetMenuButton[Down]` → **legacy `Input.GetButton("MenuButtonOculusTouch")`** → InputManager axis "joystick button 6" ← engine-internal XR-usage→legacy-joystick bridge ← left device `MenuButton` usage. Sabers/triggers use the Input System asset (`<XRController>{Left/Right}Hand`), a different consumer layer — which is why everything else works while pause alone is dead.

**Causal chain:** game launches with no ALVR client → oxrsys fabricates `khr/simple_controller` for both hands (`InputManager.cpp:623` fallback — no real runtime does this) → Unity creates fake XR devices during the entire load window → client connects ~30 s in → profile flaps `simple|simple` → `NULL|NULL` → `touch|touch` within 90 ms (round6d lines 657→665→671) → Unity destroys and recreates both devices mid-run → the recreated touch devices' usages never (re)feed the legacy joystick buttons → button 6 stays false forever → `GetMenuButtonDown()` never fires → no pause, despite spec-perfect action data.

## (2) Contradiction resolution

The premise was wrong. The right-hand menu bindings on vive/microsoft/simple belong to **different actions** (10/22/31 — Unity creates one action set per profile feature). Action 44's own suggested bindings (log lines 584-585) are **left = `/input/menu/click`, right = `/input/system/click`** — Unity's `OculusTouchControllerProfile` builds its menuButton action asymmetrically, and the "7 menu bindings" count missed the 8th because `system/click` doesn't contain the substring "menu". The two same-millisecond reads are Unity's per-device polls (left, then right). Right reads `false` because its only active-profile source is `system/click`, which `GetButtonClick` has no case for (`InputManager.cpp:655-739` → falls through to `false`), while `AccumulateBindingState` unconditionally sets `isActive=true` (`EntryPoint.cpp:2212`). **The alternating `true/false` is per-spec correct behavior**, made to look pathological by tracer bugs: the logger never prints `subactionPath`, poisons its per-handle menu cache on first read (`try_emplace`, `EntryPoint.cpp:2441`), and shares one `static lastMenuState` across handles and subactions (`EntryPoint.cpp:2439`). Global `menuClick_` is irrelevant to the right subaction because the vive/microsoft profiles are never synced (`GetActiveInteractionProfiles`, `InputManager.cpp:626-653`, returns only `[touch, simple]`). Note: report 3's "changedSinceLastSync consumed-by-first-reader" claim is refuted by code — `boolChanged` is computed at sync and stored per subaction (`ActionSet.cpp:93`), reads are idempotent.

## (3) Minimal fix

1. **`InputManager.cpp:623`** — delete the `kSimpleControllerProfile` fallback; return `{}` when not streaming (keep behind a dev/no-client config flag). Devices then don't exist until real controllers do.
2. **`Session.cpp:203-228`** (`MaybeEmitInteractionProfileChanged`) — debounce: require N stable frames before emitting, so the transient `NULL|NULL` between connect states never reaches the game. Result: exactly one profile event (touch), devices created once — matching real runtimes.
3. Tracer (prerequisite for any retest): log `getInfo->subactionPath` and make state tracking per (handle, subactionPath) in `EntryPoint.cpp:2438-2458`.

## (4) Discriminating live experiment (zero code, one song)

**Press X (left primary) or A (right primary) mid-song.** Both are pause bindings in this build (`OpenXRPrimaryButton{Left,Right}Hand` → joystick buttons 2/0, same legacy bridge, symmetric bindings so immune to the menu asymmetry).
- **X/A does NOT pause** → legacy bridge is dead → confirms the root cause above; apply fixes 1+2, retest.
- **X/A DOES pause** → bridge alive, blame shifts to the touch-profile menu control specifically → apply tracer fix 3 to confirm live which subaction carries `true`, and apply R3 (below) — the right device's active-but-false `system/click` feed is then the prime suspect for confusing Unity's MenuButton usage mapping.

## (5) Runners-up, ranked

1. **R2 — focus loss never emitted** (`Session.cpp:867-912` only ratchets upward): the system-button pause path is structurally dead. Independent, confirmed defect. Fix regardless (emit FOCUSED→VISIBLE on overlay/stream-pause + return `XR_SESSION_NOT_FOCUSED` from `OxrSyncActions`, `EntryPoint.cpp:2333-2412`) — guarantees a working pause path even if the menu listener stays stubborn.
2. **R3 — right-hand `system/click` reported bound + `isActive=true` + permanently false** (`EntryPoint.cpp:2212`, missing mapping at `InputManager.cpp:739`): real runtimes reserve system and report inactive. The one observable deviation on the very action being read; promoted to co-primary if the X/A test pauses.
3. **Candidate A (atom skew / subaction misattribution)** — no mechanism found in code or bridge; kill definitively with the subactionPath log.
4. **R4 — global `menuClick_`** (`InputManager.cpp:305,502`): latent; wrong the day the current profile is simple (action 22 right subaction reads the left physical button today).
5. **Latent `BindingSourcePriority` reset hazard** (`EntryPoint.cpp:2206-2210`): dead code today (hand_interaction appended last), fires only if priority tiers change.
6. **wineopenxr secondaries**: timespec PE→host `tv_nsec` garbage high-half (`pe/loader_thunks.c:95-107`); event suppression for unknown sessions (`pe/openxr_loader.c:1290-1296`) — watch this during R2 work, it can swallow the very session events R2 emits.

Key files: `/Users/yifeiding/projects/personal/wine-vr/ext/oxrsys/runtime/src/InputManager.cpp`, `/Users/yifeiding/projects/personal/wine-vr/ext/oxrsys/runtime/src/Session.cpp`, `/Users/yifeiding/projects/personal/wine-vr/ext/oxrsys/runtime/src/EntryPoint.cpp`, `/Users/yifeiding/projects/personal/wine-vr/ext/oxrsys/runtime/src/ActionSet.cpp`; logs `/tmp/bs_launch_round6d.log`, `/tmp/bs_launch_round6e.log`.

---

# Appendix: individual probe reports


## Probe report 1

# oxrsys Action/Input Subsystem — Deep Review Findings

## 1. THE CONTRADICTION IS RESOLVED (hard evidence, not inference)

**The two alternating reads are the left and right subactions of action 44, and the `false` one is correct behavior.** The premise "action 44's right-hand bindings are menu on vive/microsoft/simple" is wrong — those right-menu bindings belong to *different actions*.

Evidence — `/tmp/bs_launch_round6d.log:584-585` (the suggested-binding dump):
```
OXRSys: Binding 44 -> /user/hand/left/input/menu/click
OXRSys: Binding 44 -> /user/hand/right/input/system/click
```
Action 44 lives in Unity's **oculus/touch_controller action map** (bindings 41–56, "Stored 32 bindings for profile /interaction_profiles/oculus/touch_controller", log line 610). Unity's `OculusTouchControllerProfile` deliberately builds its `menuButton`-usage action as *left=menu/click, right=system/click*. The 7 menu bindings are spread across 4 actions: vive=action 10, simple=action 22, microsoft=action 31, touch=action 44 (left only). Unity only polls the map for the *current* profile (touch), which is why only action 44 appears in the read trace.

Per-subaction walk at press time (`EntryPoint.cpp`):
- Sync loop `OxrSyncActions` (2349–2385) iterates per-hand over `InputManager::GetActiveInteractionProfiles()` (`InputManager.cpp:626-653` → `[oculus/touch, khr/simple]` for controllers; **vive/microsoft are never synced**), filters bindings by `binding.topLevelPath == expectedTopLevelPath` (2369), buckets aggregates by `(actionHandle, topLevelPath)` (2380–2382).
- **Left subaction**: touch binding `menu/click` → `AccumulateBindingState` (2179) → `GetBooleanComponentForProfile(Left,"menu/click",touch)` (`InputManager.cpp:747`) → `GetButtonClick` → `GetMenuClick()` = global `menuClick_` (`InputManager.cpp:305,502`) → **true, active=true**.
- **Right subaction**: touch binding `system/click` → `GetButtonClick(Right,"system/click")` falls through **every** case in the if-chain (`InputManager.cpp:655-739` — there is no `system/click` branch) → `return false` (line 739). But `AccumulateBindingState` sets `aggregate.isActive = true` unconditionally once the device is active (`EntryPoint.cpp:2212`) → **false, active=true**.

Confirmed live in `/tmp/bs_launch_round6e.log:704-755`: pairs at exactly 72 Hz — `state=true active=true changed=true` (first left read of the press), then `state=false active=true changed=false` (right), repeating with `changed=false`. The pattern is byte-for-byte what the code predicts. `BindingSourcePriority` (2170) is irrelevant here: all controller profiles are priority 2; hand_interaction (priority 1) is appended *last* by `GetActiveInteractionProfiles` (InputManager.cpp:647-650), so the `aggregate = {}` reset (2206-2210) is dead code in practice. The hypothesized "reset then never re-add menu" hazard is real but latent — it can only fire if a >2 priority tier is ever added or automation reorders profiles.

**Conclusion: the runtime delivers `left menu = true, changed once, active, correctly timestamped` to the game on every frame of every press. The action/aggregation path has no defect for action 44.** The pause failure is downstream of correct action data — which redirects blame to the lifecycle/contract deviations below.

## 2. Root-cause candidates for "game never pauses", ranked

**R1 (high) — Interaction-profile fabrication + mid-run device churn breaks Beat Saber's menu-button listener.**
Timeline (round6d/6e, identical): at app start (no client) the runtime reports `khr/simple_controller` for both hands (`InputManager.cpp:623` fallback in `GetCurrentInteractionProfileCandidates`; event emitted from `Session::WaitFrame` → `MaybeEmitInteractionProfileChanged`, `Session.cpp:203-228,430`). 30 s later on client connect it flaps `simple|simple` → `|` (NULL both hands, `InputManager.cpp:618-621`) → `touch|touch` within 90 ms (log lines 657→665→671). Unity destroys and recreates its XR `InputDevice`s each time. Beat Saber's pause listener (event-subscription based, unlike its per-frame-polled trigger/pose reads — which is exactly why sabers/trigger survive the churn but pause can die) initializes against the fake simple-controller devices during the whole loading window. No real runtime ever does this: they report `XR_NULL_PATH` until a physical controller is bound.
Fix sketch: delete the `kSimpleControllerProfile` fallback at `InputManager.cpp:623` (return `{}`; keep it only behind a dev/no-client config flag), and debounce `MaybeEmitInteractionProfileChanged` so the transient `'|'` signature between disconnect/reconnect is never emitted (require N stable frames before emitting, `Session.cpp:203`).

**R2 (high, independent pause path) — Focus loss is never emitted, so the system-button pause path is structurally dead.**
`AdvanceSessionStateAfterFrameSubmission` (`Session.cpp:867-912`) only ratchets READY→SYNCHRONIZED→VISIBLE→FOCUSED (and back down on exit). Nothing ever transitions FOCUSED→VISIBLE while streaming, so Unity's `OnApplicationFocus(false)` auto-pause (the path the Meta button uses on real runtimes) can never fire. The `'|'` event at round6d line 765 shows the runtime *can already detect* the client dropping into the system overlay (controller flags drop / stream pauses).
Fix sketch: on stream-pause/overlay detection (controllers+hands all inactive while `IsStreaming()`, debounced, or an explicit ALVR pause signal), `TransitionState(XR_SESSION_STATE_VISIBLE)`; restore FOCUSED on resume. Must be paired with C1 below to be spec-correct.

**R3 (medium) — right-hand `system/click` reported bound + active(+false forever).**
Real runtimes treat `/input/system/click` as reserved: not bound, `isActive=false`. oxrsys feeds Unity's right-device `menuButton` control a permanently-false-but-active source (`EntryPoint.cpp:2212` + missing component mapping at `InputManager.cpp:739`) and lies in `xrEnumerateBoundSourcesForAction`. Unlikely to block the left press by itself, but it is the one observable deviation *on the very action being read*.
Fix sketch: in `AccumulateBindingState`, only set `isActive`/push `boundSources` when the component is actually mappable (e.g. a `SupportsComponent(componentPath)` check); or drop `*/input/system/*` bindings at accept time in `OxrSuggestInteractionProfileBindings` (still return XR_SUCCESS per spec).

**R4 (low, latent) — global `menuClick_`**: any right-hand `menu/click` binding (simple/vive/WMR maps — action 22 right subaction *does* sync true today) reads the left physical button. Harmless while Unity only polls the touch map; wrong the day the current profile is simple. Fix: make menu per-hand in the protocol or gate `GetMenuClick` to Left.

## 3. Contract audit vs OpenXR spec (Unity-relevant)

- **`xrSyncActions` never returns `XR_SESSION_NOT_FOCUSED`** — no session-state check anywhere in `OxrSyncActions` (`EntryPoint.cpp:2333-2412`). Spec requires it when not FOCUSED, with all action states forced inactive. Currently benign (session is always FOCUSED — itself the R2 problem) but mandatory before implementing R2. Haptics do check focus (`EntryPoint.cpp:2586,2648`).
- **`xrSyncActions` active-set handling**: `IsActionSetActive` (`2151-2168`) treats `countActiveActionSets==0` as "all attached sets" (spec: nothing should sync; also `XR_ERROR_VALIDATION_FAILURE` territory), never validates the handles (`XR_ERROR_HANDLE_INVALID`), never rejects unattached sets (`XR_ERROR_ACTIONSET_NOT_ATTACHED`), and ignores `XrActiveActionSet::subactionPath` filtering entirely (2380 uses the binding's top-level path, `(void)subactionPath` at 2261).
- **Session state transitions**: IDLE→READY emitted synchronously inside `xrCreateSession` (`Session.cpp:127-128`) — legal but unusual; EndSession order IDLE→EXITING (290-291) OK; never VISIBLE mid-session (R2); state numbers in logs (1,2,3,4,5 = IDLE…FOCUSED) confirm READY-before-BeginSession worked for Unity.
- **`xrGetCurrentInteractionProfile`** (`2284-2331`): fabricates `khr/simple_controller` with no device (R1); transiently NULL between "client connected" and "first tracking packet with controller flags"; validates top-level path and returns `XR_ERROR_PATH_UNSUPPORTED` correctly; does not validate `interactionProfile->type` on input (minor).
- **`xrEnumerateBoundSourcesForAction`** (`2865-2902`): derives from last-sync aggregates only (`ActionSet.cpp:107-120`, populated in `ApplySyncState`), so it's empty before the first sync and after device deactivation, and includes reserved `system/click`. Real runtimes return the profile-resolved binding set stably.
- **`XrActionStateGetInfo` validation — compliant**: `ValidateActionSubactionPath` (`2045-2067`) does verify `subactionPath` against the action's declared subaction paths (`XR_ERROR_PATH_UNSUPPORTED`) and invalid atoms (`XR_ERROR_PATH_INVALID`); type mismatch → `XR_ERROR_ACTION_TYPE_MISMATCH`; unattached → `XR_ERROR_ACTIONSET_NOT_ATTACHED` (2111-2149). `getInfo->type`/`state->type` input validation missing (minor). NULL-subaction aggregation (`GetQueriedActionState`, 2069-2109) follows the combine rules.
- **Handle namespacing**: single global registry with unchecked `static_cast` in `Runtime::FromHandle` (`Runtime.h:35-45`) — a wrong-type handle yields type confusion/UB instead of `XR_ERROR_HANDLE_INVALID`. Handles are globally unique so no aliasing; conformance robustness gap only.
- **Diagnostic bug that corrupted this investigation**: the menu-read logger in `OxrGetActionStateBoolean` (`EntryPoint.cpp:2438-2458`) caches "is menu action" per handle from the *first* read's `boundSources` (`try_emplace` at 2441) — if the first read is the right subaction (`system/click`) or pre-sync (empty sources), the handle is poisoned non-menu forever (why round6d has zero `game read menu action` lines and round6e has them); and `lastMenuState` is a single static shared across handles *and* subactions, which is what produced the "alternating" log cadence. It also never logs `subactionPath` — add it; that one field would have resolved this in round 1.

**Suggested verification order**: (1) add subactionPath to the read log; (2) A/B: remove the simple-controller fallback (R1) → retest pause; (3) implement focus-loss emulation (R2 + the `xrSyncActions` focus return) → gives a working pause path even if Beat Saber's left-menu listener remains stubborn.

Key files: `/Users/yifeiding/projects/personal/wine-vr/ext/oxrsys/runtime/src/EntryPoint.cpp`, `/Users/yifeiding/projects/personal/wine-vr/ext/oxrsys/runtime/src/InputManager.cpp`, `/Users/yifeiding/projects/personal/wine-vr/ext/oxrsys/runtime/src/Session.cpp`, `/Users/yifeiding/projects/personal/wine-vr/ext/oxrsys/runtime/src/ActionSet.cpp`, logs `/tmp/bs_launch_round6d.log`, `/tmp/bs_launch_round6e.log`.

## Probe report 2

All evidence gathered. Here is the review.

# wineopenxr bridge review: action-state path

## Verdict on the raw `getInfo` pass-through: NOT A BUG — the bridge is exonerated for the alternating-state contradiction

**`thunk_xrGetActionStateBoolean` (unix/openxr_thunks.c:450-464) passing `params->getInfo` raw is correct in this architecture.** Three independent reasons:

1. **Same address space, same thread.** `UNIX_CALL` is `__wine_unix_call_dispatcher` (include/openxr_loader.h:206-207) — a synchronous in-process call. The host runtime dereferences the game's pointer directly; no marshalling layer exists to corrupt it.
2. **Identical ABI on both sides.** PE side is mingw x86_64 (pe/CMakeLists.txt, `--kill-at` no-op comment confirms x86_64), unix side is pinned `OSX_ARCHITECTURES "x86_64"` (unix/CMakeLists.txt:47). Both compile against the *same* vendored `extern/OpenXR-SDK/include/openxr/openxr.h`. `XrActionStateGetInfo` (openxr.h:1479-1484) = `type@0(4)+pad, next@8(ptr), action@16(XR_DEFINE_HANDLE = pointer, 8B), subactionPath@24(XR_DEFINE_ATOM = uint64_t)`, size 32 — offsets identical under MS and SysV ABIs. `XrActionStateBoolean` (openxr.h:1486-1493) = `type@0, next@8, currentState@16, changedSinceLastSync@20, lastChangeTime@24 (XrTime = int64_t, offset already 8-aligned, no hidden padding), isActive@32`, size 40 — identical. LLP64-vs-LP64 `long` divergence never appears; all fields are fixed-width, pointer, or handle.
3. **The values inside `getInfo` are already host-native** (see next section), so no translation is *needed*.

## Task 2 — Handle mapping: actions/actionsets/spaces are RAW host handles; only instance/session/swapchain are wrapped

- Wrapped: `wine_XrInstance`/`wine_XrSession`/`wine_XrSwapchain` only (include/openxr_loader.h:108-193; `*_from_handle` are plain casts, host handle at offset 0).
- `thunk_xrCreateActionSet` (unix/openxr_thunks.c:163-177): converts the instance, then passes the game's output pointer straight to the host — **the game receives the host's XrActionSet verbatim**. `thunk_xrCreateAction` (147-161) likewise passes `params->actionSet` raw in and writes the host XrAction raw out. Same for spaces (179-193, 211-225) and hand trackers (195-209).
- **Consequence: the "action handle 44" the game holds is bit-identical to the handle oxrsys's `OxrGetActionStateBoolean` receives.** The action-44 assumption in the evidence chain is confirmed; there is no PE↔host handle skew.

## Task 1 — XrPath atoms: single namespace, host-owned, consistent everywhere

- `thunk_xrStringToPath` (unix/openxr_thunks.c:838-852): converts only the instance; the host writes its own atom directly into the game's `XrPath*`. `thunk_xrPathToString` (745-761) feeds the game's atom straight back to the host. **The bridge maintains no path table; every atom the game ever sees is a host atom.**
- Every action-related carrier checked and consistent: `xrCreateAction` createInfo `subactionPaths` (raw, host atoms — correct); `xrSuggestInteractionProfileBindings` (886-899, `XrActionSuggestedBinding {action, binding}` = 16B both ABIs, raw host values — correct); `xrGetCurrentInteractionProfile` (514-528, `topLevelUserPath` in and `interactionProfile` atom out both host-native — correct); `xrSyncActions` (901-914, `XrActiveActionSet {actionSet, subactionPath}` raw host values — correct); `xrGetActionStateBoolean` `getInfo->subactionPath` (raw host atom — correct). No entry point exists where an unconverted "PE-side atom" could arise, because PE-side atoms do not exist as a separate species.

## Task 3 — No caching or deduplication anywhere on the dispatch path

- `wine_xrGetActionStateBoolean` (pe/loader_thunks.c:365-377) is a stateless pass-through built per call. Dispatch is a binary-search name table resolved once at `xrGetInstanceProcAddr` time (pe/loader_thunks.c:834, 870-885); the returned function pointer is direct. The unix side dispatches through `g_xr_host_instance_dispatch_table` populated once at instance create (unix/openxr.c:345-354). **No layer between the game and oxrsys stores, merges, or replays action state.**

## What this means for the contradiction

The bridge adds zero state and zero transformation to `xrGetActionStateBoolean`. Therefore the two same-millisecond calls that return alternating true/false are **two genuinely distinct host calls whose results differ because their inputs differ** — and with `action` fixed at 44 and `next` unused, the only field that *can* differ is `getInfo->subactionPath`. Predicted values: the host atoms for `/user/hand/left` and `/user/hand/right` (Unity's OpenXR plugin polls per-device). The defect is therefore on the oxrsys side, with a precise signature to hunt: **one subaction returns `currentState=false` while `isActive=true`, even though `menuClick_` is global** — i.e., `isActive` is computed globally (or per-action) while `currentState` accumulation is per-subaction and the right hand's only menu bindings live on the inactive vive/microsoft/simple profiles (so they should legitimately contribute nothing — but then `isActive` should be false for that subaction, and it isn't). Fix the `isActive`/`currentState` consistency or the per-subaction accumulation in `AccumulateBindingState`/`BindingSourcePriority`, and log `subactionPath` in the tracer to confirm.

## Task 4 — Debug affordances and live-probe recipe

`WINEDEBUG=+openxr` currently prints **nothing** for action-state calls: neither generated thunk file (pe/loader_thunks.c, unix/openxr_thunks.c) contains a single TRACE. What does print: PE `xrGetInstanceProcAddr` per lookup (pe/openxr_loader.c:80), instance/session/swapchain lifecycle (200, 225, 245, 579, 668, 679), texture import (929), and — **usefully** — `"suppressing event type %u for unknown session"` (pe/openxr_loader.c:1295). Unix side (`WINE_DEFAULT_DEBUG_CHANNEL(openxr)` at unix/openxr.c:27): extension translation (146), instance create (274, 333, 357), swapchain format translation (491), gpu_sync timings (676, 695, gated on `WINEOPENXR_GPU_SYNC_STATS=1`).

Probe recipe (both thunk files are generated by make_openxr.py — hand-patch and note regeneration, or patch the generator):
- In unix/openxr_thunks.c `thunk_xrGetActionStateBoolean` (line ~459): add `#include "wine/debug.h"` + `WINE_DEFAULT_DEBUG_CHANNEL(openxr)` (per-TU static, safe alongside openxr.c's) and after the host call: `TRACE("gasb act=%p sub=0x%llx -> res=%d cur=%u chg=%u act=%u t=%lld\n", params->getInfo->action, (unsigned long long)params->getInfo->subactionPath, params->result, params->state->currentState, params->state->changedSinceLastSync, params->state->isActive, (long long)params->state->lastChangeTime);` — this closes the exact tracer gap (subactionPath) at the boundary. Run with `WINEDEBUG=+openxr`.
- Cheaper alternative: since the bridge is proven transparent, adding the subactionPath log in oxrsys's `OxrGetActionStateBoolean` (EntryPoint.cpp, already instrumented) is equivalent evidence.
- While probing, also watch for the line-1295 suppression trace: if it ever fires during play, the bridge is swallowing host session events — relevant to evidence item 5 (focus-loss pause path never working).

## Secondary findings (unrelated to the menu bug, ranked)

1. **timespec ABI mismatch, PE→host direction** — pe/loader_thunks.c:95-107 / unix/openxr_thunks.c:131-145: mingw LLP64 `struct timespec.tv_nsec` is 4-byte `long` + 4 padding; Darwin's is 8-byte. Host→PE reads are accidentally safe on little-endian (tv_nsec < 1e9), but PE→host (`xrConvertTimespecTimeToTimeKHR`) reads 4 bytes of uninitialized PE padding into the high half of tv_nsec. Reachable: the timespec functions are in the PE dispatch table (pe/loader_thunks.c:804, 806) and `wine_is_available_instance_function` (unix/openxr.c:66-101) gates only on host GIPA — which succeeds whenever the app enabled the *win32* time extension (the bridge itself enables the timespec extension on the host via the substitution table). Also a minor spec leak: the app can call functions of an extension it never enabled.
2. **xrPollEvent session-handle gaps** — pe/openxr_loader.c:1233-1262: `rewrite_event_session` handles 5 event types; other session-carrying events (e.g. `XR_TYPE_EVENT_DATA_PERF_SETTINGS_EXT`) pass through with a raw host session handle the game cannot match. And 1290-1296 silently drops up to 64 events for non-primary host sessions.

Files reviewed: /Users/yifeiding/projects/personal/wine-vr/ext/wineopenxr/src/pe/openxr_loader.c, /Users/yifeiding/projects/personal/wine-vr/ext/wineopenxr/src/pe/loader_thunks.c, /Users/yifeiding/projects/personal/wine-vr/ext/wineopenxr/src/unix/openxr.c, /Users/yifeiding/projects/personal/wine-vr/ext/wineopenxr/src/unix/openxr_thunks.c, /Users/yifeiding/projects/personal/wine-vr/ext/wineopenxr/src/include/openxr_loader.h, /Users/yifeiding/projects/personal/wine-vr/ext/wineopenxr/src/include/loader_thunks.h, /Users/yifeiding/projects/personal/wine-vr/ext/wineopenxr/extern/OpenXR-SDK/include/openxr/openxr.h.

## Probe report 3

All evidence gathered. Here is the full report.

---

# How Beat Saber 1.29.4 consumes the pause button, what the runtime must provide, and the likely mismatch

## Q4 first — hard facts from Player.log (`/Users/yifeiding/Library/Application Support/CrossOver/Bottles/Steam/drive_c/users/crossover/AppData/LocalLow/Hyperbolic Magnetism/Beat Saber/Player.log`)

- Engine: **Unity 2021.3.16f1** (not 2019.4 — the 1.29.4 build was upgraded when Beat Games ported to OpenXR).
- XR stack: **Unity OpenXR plugin, provider version 1.5.3** (spec 1.0.23). Only `UnityOpenXR` is loaded for both Display and Input ("Loading plugin UnityOpenXR for subsystem OpenXR Input"). OculusXRPlugin descriptors are discovered but never loaded. **No OpenVR fallback exists in this build at all.**
- 5 interaction-profile features enabled: Oculus Touch, HTC Vive, Valve Index, Microsoft Motion, Khronos Simple. Diagnostic report shows **5 action sets** attached (`oculustouchcontroller: ActionCount=16`, etc.).
- Session reaches FOCUSED; no input errors; Player.log does not log device creation (Unity doesn't print those lines by default), but devices demonstrably exist — see below.

## Q1 — How the game detects the pause press (this is the crux)

Verified from the game binaries (not community folklore):

- The `IVRPlatformHelper` implementation running here is `UnityXRHelper`, defined in **HMLib.dll** (`.../Beat Saber 1294/Beat Saber_Data/Managed/HMLib.dll`; confirmed via the MonoScript registry in `globalgamemanagers.assets`).
- In 1.29.4, `UnityXRHelper` does **NOT** read the menu button via `UnityEngine.XR.InputDevices`/`CommonUsages.menuButton`, and **NOT** via an Input System action. (HMLib.dll contains no `TryGetFeatureValue`/`CommonUsages` metadata references; the Input System `InputActionReference` in it is only for `_userPresenceAction`. The `_pauseGameActionReference` field seen in bs-cordl HEAD headers is a **later** game version.) The game's `BeatSaberInputActions` asset (sharedassets0.assets) binds only `<XRController>{Left/Right}Hand` devicePosition/deviceRotation/trigger/thumbstick — **no menuButton binding anywhere in any asset**.
- Pause polling goes through `VRPlatformUtils.GetMenuButton[Down]DefaultImplementation()` (HMLib.dll) which calls **legacy `UnityEngine.Input.GetButton/GetButtonDown`** (HMLib references `UnityEngine.InputLegacyModule`: `GetButton`, `GetButtonDown`, `GetAxis`) with these constants (values confirmed in bs-cordl and in HMLib/globalgamemanagers string heaps):
  - `kMenuButtonOculusTouch = "MenuButtonOculusTouch"` → InputManager axis bound to **"joystick button 6"**
  - `kMenuButtonLeftHand = "OpenXRPrimaryButtonLeftHand"` → **"joystick button 2"**
  - `kMenuButtonRightHand = "OpenXRPrimaryButtonRightHand"` → **"joystick button 0"**
  (axis definitions dumped from `globalgamemanagers`; note there is **no axis bound to joystick button 7** — the right-hand menu usage.)
- Per Unity 2021.3's legacy XR input mapping table, those virtual joystick buttons are fed from XR controller usages: **menuButton = button 6 (L) / 7 (R); primaryButton = button 2 (L) / 0 (R)**. So in this build, pause = **left menu button OR X button OR A button**.
- Downstream flow: `IMenuButtonTrigger` (`InstantMenuButtonTrigger` polls `GetMenuButtonDown` edge; `DelayedMenuButtonTrigger` polls `GetMenuButton` held, per `MainSettingsModel.PauseButtonPressDurationLevel` — default 0 = instant; no settings.cfg override present in your prefix) → `menuButtonTriggeredEvent` → `PauseController`. `PauseController` also pauses on input-focus-captured and HMD-unmounted events (your focus-loss observation).

**Key consequence: the game never consumes `xrGetActionStateBoolean` results in managed code for pause.** The per-frame action queries you traced are the Unity OpenXR provider's device-update loop. Pause depends on an additional, engine-internal hop: OpenXR action state → XR input subsystem device feature (usage "MenuButton") → **native legacy-joystick bridge** → `Input.GetButtonDown("MenuButtonOculusTouch")`.

## Q2 — What the Unity OpenXR 1.5.3 provider actually does

From the 1.5.3 package source (needle-mirror) and your diag report:

- One action set per profile feature; one action per control, created once with **both** `/user/hand/left` and `/user/hand/right` subaction paths.
- The Oculus Touch profile's `menu` action (`OculusTouchControllerProfile.cs`, needle-mirror 1.5.3, lines ~415-437): usage `"MenuButton"`, bindings **left → `/input/menu/click`, right → `/input/system/click`**. **The game therefore suggested `/user/hand/right/input/system/click` on your ACTIVE touch profile** — your "7 menu bindings" count missed it because that path doesn't contain the substring "menu". Action 44 has 8 suggested bindings, and one of them is a right-hand binding on the active profile.
- Each frame after `xrSyncActions`, the provider polls each action once **per instantiated device with `subactionPath` = that device's user path**. That is exactly your two same-millisecond `xrGetActionStateBoolean(44)` calls: left device reading menu/click, right device reading system/click. **Alternating true/false is the per-spec CORRECT result** (left = pressed, right = system/click never pressed — the stock store client never transmits the reserved Meta/system button). Your contradiction dissolves: with active-profile gating, the right subaction correctly reads false because its only active-profile source is system/click, not the vive/microsoft/simple menu bindings (inactive profiles must not contribute).
- Device instantiation requires `xrGetCurrentInteractionProfile` returning the touch profile plus `XrEventDataInteractionProfileChanged` — this evidently works (sabers track and triggers click UI through `<XRController>` Input System bindings, which need those devices). Beyond `currentState`, the provider consumes `isActive`; `boundSources` enumeration and `lastChangeTime` are not functionally required for buttons.

## Q3 — Known issues (corroboration)

- ALVR issue #938: menu button opens SteamVR dashboard instead of reaching the game (SteamVR-runtime-specific reservation; not your case, since oxrsys receives the press).
- Community reports after the 1.29.x OpenXR switch of pause triggering from face buttons / pause quirks per controller type (Vive wands etc.) — consistent with the `OpenXRPrimaryButton*Hand` (X/A) pause bindings found in the binary.
- Oculus/Meta runtimes reserve `system` (right-hand path on the touch profile) — a runtime returning constant false there, as yours effectively does, is normal.

## The most likely mismatch, and how to discriminate (two candidates)

Everything in your verified chain (steps 1-4) can be simultaneously true **and healthy** — the defect is in the one hop you haven't instrumented. Two candidates remain:

**Candidate A — subaction misattribution in oxrsys (`true` read belongs to the RIGHT device).** If `AccumulateBindingState`'s subaction filtering misroutes (e.g., matches by binding order or falls back to global `menuClick_` when no binding matches the queried hand), the RIGHT device's menu control goes true → legacy joystick **button 7** → Beat Saber has **no axis bound to button 7** → silent no-op, while everything else in the game looks perfect. This uniquely affects the menu action because it is the **only boolean action whose two hands bind to different component paths on the active profile** (menu/click vs system/click) — per-hand symmetric actions (trigger, thumbstick) wouldn't expose the bug. Your tracer must log `subactionPath` on `OxrGetActionStateBoolean` — this is the single decisive instrumentation.

**Candidate B — the legacy-joystick bridge isn't being fed at all under oxrsys.** Nothing else in Beat Saber uses legacy input (UI/trigger/poses all go through the Input System asset), so a wholly broken bridge would be invisible except for pause — exactly your symptom profile.

**Zero-code discriminating test: press X (left primary) or A (right primary) mid-song.** Both are pause buttons in this build (joystick buttons 2/0 from usage `PrimaryButton`).
- X/A pauses → bridge works → Candidate A (menu state is landing on the wrong device / never landing on the left device). Fix subaction routing for asymmetric bindings; also make the right-hand query return `isActive=false` (no pressed source) rather than active=true, matching spec intent for the reserved system path.
- X/A doesn't pause → Candidate B → investigate the provider-visible prerequisites for the legacy bridge: device characteristics/handedness in the device descriptors, and whether oxrsys handles the right-hand `/input/system/click` suggestion in a way that corrupts the touch profile's binding table (e.g., partial rejection). Also verify `xrGetCurrentInteractionProfile` returns the touch profile for **both** top-level paths and that both controller devices exist (Unity's Input Debugger equivalent: look for two "Oculus Touch Controller (OpenXR)" devices).

Secondary note: `changedSinceLastSync` being true "exactly once" globally is a latent oxrsys bug (it must be computed per action+subaction relative to the last `xrSyncActions`, not consumed by the first reader), but Unity's button path doesn't depend on it.

Key local artifacts: `/Users/yifeiding/wine_shared/SteamLibrary/steamapps/common/Beat Saber 1294/Beat Saber_Data/Managed/HMLib.dll` (UnityXRHelper/VRPlatformUtils), `.../Beat Saber_Data/globalgamemanagers` (InputManager axes), `.../Beat Saber_Data/sharedassets0.assets` (BeatSaberInputActions), Player.log as above.

Sources:
- [Oculus Touch Controller Profile | OpenXR Plugin 1.5 docs](https://docs.unity3d.com/Packages/com.unity.xr.openxr@1.5/manual/features/oculustouchcontrollerprofile.html)
- [OculusTouchControllerProfile.cs, needle-mirror com.unity.xr.openxr 1.5.3](https://raw.githubusercontent.com/needle-mirror/com.unity.xr.openxr/1.5.3/Runtime/Features/Interactions/OculusTouchControllerProfile.cs)
- [Unity 2021.3 Manual — XR input / legacy input mappings](https://docs.unity3d.com/2021.3/Documentation/Manual/xr_input.html)
- [bs-cordl generated headers — VRPlatformUtils / UnityXRHelper](https://github.com/QuestPackageManager/bs-cordl)
- [sc2ad/BeatSaber-Quest-Codegen — VRControllersInputManager constants](https://github.com/sc2ad/BeatSaber-Quest-Codegen)
- [ALVR issue #938 — menu button opens SteamVR dashboard instead of in-game menu](https://github.com/alvr-org/ALVR/issues/938)
- [Khronos blog — Porting Beat Saber to OpenXR](https://www.khronos.org/blog/keeping-the-beat-porting-beat-saber-to-openxr-for-an-improved-developer-experience)
- [Steam discussions — Beat Saber pause button issues](https://steamcommunity.com/app/620980/discussions/2/6821308966017816627/)
