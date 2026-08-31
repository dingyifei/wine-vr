use std::sync::atomic::Ordering;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    Emitter, Manager,
};

mod commands;

/// Builds the native menu bar: App / Edit / Pipeline / Window.
///
/// The Edit submenu's predefined clipboard items are required — without them
/// Cmd-C / Cmd-V does not work in webview text inputs on macOS. The Pipeline
/// submenu is disabled placeholder items until the pipeline commands land
/// (Phase 1+).
fn build_menu<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) -> tauri::Result<Menu<R>> {
    // App submenu: about, settings placeholder, hide items, quit.
    let about = PredefinedMenuItem::about(handle, Some("About Sabrage"), None)?;
    let app_sep1 = PredefinedMenuItem::separator(handle)?;
    // Disabled until the Settings screen gains a deep-link (Phase 4).
    let settings = MenuItem::with_id(
        handle,
        "app_settings",
        "Settings…",
        false,
        Some("CmdOrCtrl+,"),
    )?;
    let app_sep2 = PredefinedMenuItem::separator(handle)?;
    let hide = PredefinedMenuItem::hide(handle, None)?;
    let hide_others = PredefinedMenuItem::hide_others(handle, None)?;
    let app_sep3 = PredefinedMenuItem::separator(handle)?;
    // NOT `PredefinedMenuItem::quit`: that item sends AppKit's `terminate:`
    // selector, and tao implements no `applicationShouldTerminate:`, so the
    // process is torn down without Tauri ever emitting `ExitRequested` — the
    // live-session dialog can never appear on Cmd-Q (live-verified 2026-08-29:
    // Cmd-Q killed the app mid-teardown, guards restored by their `Drop`
    // fallbacks, the session record left saying "live"). A custom item whose
    // handler calls `app.exit(0)` goes through `Message::RequestExit`, the one
    // path `ExitRequested` + `prevent_exit` actually cover. Dock-menu Quit and
    // logout still arrive as `terminate:`; those get the best-effort detach in
    // the `RunEvent::Exit` arm below.
    let quit = MenuItem::with_id(
        handle,
        "app_quit",
        "Quit Sabrage",
        true,
        Some("CmdOrCtrl+Q"),
    )?;
    let app_menu = Submenu::with_items(
        handle,
        "Sabrage",
        true,
        &[
            &about,
            &app_sep1,
            &settings,
            &app_sep2,
            &hide,
            &hide_others,
            &app_sep3,
            &quit,
        ],
    )?;

    // Edit submenu: predefined clipboard items only.
    let undo = PredefinedMenuItem::undo(handle, None)?;
    let redo = PredefinedMenuItem::redo(handle, None)?;
    let edit_separator = PredefinedMenuItem::separator(handle)?;
    let cut = PredefinedMenuItem::cut(handle, None)?;
    let copy = PredefinedMenuItem::copy(handle, None)?;
    let paste = PredefinedMenuItem::paste(handle, None)?;
    let select_all = PredefinedMenuItem::select_all(handle, None)?;
    let edit_menu = Submenu::with_items(
        handle,
        "Edit",
        true,
        &[
            &undo,
            &redo,
            &edit_separator,
            &cut,
            &copy,
            &paste,
            &select_all,
        ],
    )?;

    // Pipeline submenu: real commands as of Phase 3 (`on_menu_event` in `run()`
    // below maps each id to a `menu://…` event the frontend listens for).
    let run_doctor = MenuItem::with_id(
        handle,
        "pipeline_run_doctor",
        "Run Doctor",
        true,
        Some("CmdOrCtrl+D"),
    )?;
    let launch = MenuItem::with_id(
        handle,
        "pipeline_launch",
        "Launch",
        true,
        Some("CmdOrCtrl+R"),
    )?;
    let stop = MenuItem::with_id(handle, "pipeline_stop", "Stop", true, Some("CmdOrCtrl+."))?;
    let pipeline_menu =
        Submenu::with_items(handle, "Pipeline", true, &[&run_doctor, &launch, &stop])?;

    // Window submenu: close, minimize, zoom.
    let close = PredefinedMenuItem::close_window(handle, None)?;
    let minimize = PredefinedMenuItem::minimize(handle, None)?;
    let zoom = PredefinedMenuItem::maximize(handle, Some("Zoom"))?;
    let window_menu = Submenu::with_items(handle, "Window", true, &[&close, &minimize, &zoom])?;

    Menu::with_items(
        handle,
        &[&app_menu, &edit_menu, &pipeline_menu, &window_menu],
    )
}

/// What to do with one `ExitRequested`/`CloseRequested`
/// ([`commands::quit_intercept_decision`], with this app's three live inputs).
///
/// `Ask` is the ordinary interception. `GiveUp` — a dialog that has gone
/// unanswered past [`commands::QUIT_DIALOG_TIMEOUT`] and a user asking to quit
/// again — is handled by *not* intercepting: the exit proceeds, and the
/// `RunEvent::Exit` arm's `detach_on_terminate` then applies the same
/// keep-running answer the un-interceptable AppKit `terminate:` path gets. The
/// app can therefore never become unquittable because the webview died before
/// it subscribed to `app://quit-requested`.
fn quit_decision<R: tauri::Runtime>(app_handle: &tauri::AppHandle<R>) -> commands::QuitIntercept {
    let quit_approved = app_handle.state::<commands::QuitApproved>();
    let pending = app_handle.state::<commands::PendingQuit>();
    commands::quit_intercept_decision(
        quit_approved.0.load(Ordering::SeqCst),
        sabrage_core::live_session().is_some(),
        pending.pending_for(),
    )
}

/// Open (or re-open) the "stop / keep / cancel" dialog and start the
/// unanswered-quit clock if it is not already running.
fn ask_to_quit<R: tauri::Runtime>(app_handle: &tauri::AppHandle<R>) {
    app_handle.state::<commands::PendingQuit>().mark();
    let _ = app_handle.emit("app://quit-requested", ());
}

/// Builds and runs the app.
///
/// Uses the callback form of `run` (`.build(context)?.run(|app, event| …)`
/// rather than `.run(context)`) specifically so `RunEvent::ExitRequested` and
/// `RunEvent::WindowEvent { event: WindowEvent::CloseRequested, .. }` can be
/// intercepted: this app has exactly one window, and closing it is app-quit
/// in every way that matters (critique.md, "app-quit semantics for a live
/// session"), so both are gated by the same rule
/// ([`quit_decision`]) — only a session this process is still supervising,
/// only while the pending quit has not already been approved, and only while
/// the dialog it opens still has a plausible responder, is worth stopping the
/// OS teardown for. When it fires, the handler calls
/// `prevent_exit`/`prevent_close` and emits
/// `app://quit-requested`; the frontend's dialog resolves that through
/// `commands::resolve_quit`, whose `Stop`/`Keep` arms flip `QuitApproved`
/// before calling `app.exit(0)` themselves — which re-enters this same
/// handler, and this time passes through untouched. Cmd-Q reaches this path
/// only because the Quit menu item is a custom one calling `app.exit(0)`
/// (see `build_menu`); AppKit's own `terminate:` is handled best-effort in
/// the `RunEvent::Exit` arm.
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Focus the existing main window instead of spawning a second instance.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }))
        .invoke_handler(tauri::generate_handler![
            commands::run_doctor,
            commands::get_app_state,
            commands::run_stage,
            commands::cancel_stage,
            commands::fix,
            commands::stop_session,
            commands::launch,
            commands::get_session_status,
            commands::detach_session,
            commands::reconcile_session,
            commands::start_log_tail,
            commands::stop_log_tail,
            commands::list_past_runs,
            commands::get_log_source_path,
            commands::resolve_quit,
            commands::read_runtime_config,
            commands::write_runtime_config,
            commands::get_settings,
            commands::save_settings,
            commands::get_repo_info,
            commands::suggest_bs_dir,
            commands::get_library,
            commands::new_game_template,
            commands::save_game,
            commands::remove_game,
            commands::validate_game,
            commands::revert_original_steam_dll,
        ])
        .manage(commands::RunRegistry::default())
        .manage(commands::TailRegistry::default())
        .manage(commands::SessionMonitorState::default())
        .manage(commands::QuitApproved::default())
        .manage(commands::PendingQuit::default())
        .manage(commands::SettingsPathsCache::default())
        // A webview reload (vite HMR in dev, any navigation) runs no Svelte
        // `onDestroy`, so the log tails that page started would poll their
        // files every 250 ms for the rest of the app's life — and they cannot
        // notice on their own, because a `Channel::send` on macOS is a
        // `webview.eval` that keeps succeeding after a reload. This app has
        // exactly one window, so every tail registered before a page load
        // belongs to the page being replaced.
        .on_page_load(|webview, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Started {
                webview
                    .app_handle()
                    .state::<commands::TailRegistry>()
                    .stop_all();
            }
        })
        .setup(|app| {
            let handle = app.handle();
            let menu = build_menu(handle)?;
            app.set_menu(menu)?;
            // Pipeline menu -> `menu://…` events; the frontend's screens are
            // the ones that know how to actually run doctor/launch/stop.
            app.on_menu_event(|app_handle, event| {
                if event.id().as_ref() == "app_quit" {
                    // Routed through Tauri's own exit request so the
                    // `ExitRequested` arm below can intercept it.
                    app_handle.exit(0);
                    return;
                }
                let topic = match event.id().as_ref() {
                    "pipeline_run_doctor" => Some("menu://doctor"),
                    "pipeline_launch" => Some("menu://launch"),
                    "pipeline_stop" => Some("menu://stop"),
                    _ => None,
                };
                if let Some(topic) = topic {
                    let _ = app_handle.emit(topic, ());
                }
            });
            commands::spawn_session_status_broadcaster(app.handle().clone());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // The callback form (rather than `.run(context)`) is what lets
    // `ExitRequested`/`CloseRequested` be intercepted at all — see
    // `commands.rs`'s module doc for why `detach_session`/`resolve_quit`
    // exist. Both arms share one rule
    // (`commands::should_intercept_quit`): only a session this process is
    // still supervising, and only while the pending quit has not already
    // been approved, is worth stopping the OS from tearing down for.
    app.run(|app_handle, event| match event {
        tauri::RunEvent::ExitRequested { api, .. } => {
            if quit_decision(app_handle) == commands::QuitIntercept::Ask {
                api.prevent_exit();
                ask_to_quit(app_handle);
            }
        }
        tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::CloseRequested { api, .. },
            ..
        } if label == "main" => {
            if quit_decision(app_handle) == commands::QuitIntercept::Ask {
                api.prevent_close();
                ask_to_quit(app_handle);
            }
        }
        // AppKit's `terminate:` (Dock-menu Quit, logout, an AppleScript
        // `quit`) cannot be intercepted — tao has no
        // `applicationShouldTerminate:` — but `applicationWillTerminate` does
        // reach here as `Exit`, synchronously, before the process dies. A
        // session that was never asked about gets the "keep running" answer:
        // detach (guards disarmed, record marked detached) so the game keeps
        // streaming and the next launch offers Stop/re-attach, instead of the
        // guards' `Drop` fallbacks yanking the audio device out from under a
        // still-running game.
        tauri::RunEvent::Exit => {
            let quit_approved = app_handle.state::<commands::QuitApproved>();
            commands::detach_on_terminate(quit_approved.0.load(Ordering::SeqCst));
        }
        _ => {}
    });
}
