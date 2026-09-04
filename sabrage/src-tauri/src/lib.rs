use std::sync::atomic::Ordering;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    Emitter, Manager,
};

mod commands;

/// Builds the native menu bar: App / Edit / Pipeline / Window.
///
/// The Edit submenu's predefined clipboard items are load-bearing: without
/// them Cmd-C / Cmd-V do not work in webview text inputs on macOS.
fn build_menu<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) -> tauri::Result<Menu<R>> {
    let about = PredefinedMenuItem::about(handle, Some("About Sabrage"), None)?;
    let app_sep1 = PredefinedMenuItem::separator(handle)?;
    // Disabled: `run()`'s `on_menu_event` has no `app_settings` arm, so the
    // item would open nothing.
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
    // Not `PredefinedMenuItem::quit`: it sends AppKit's `terminate:`, which
    // tao does not intercept, so `ExitRequested` never fires. A custom item
    // calling `app.exit(0)` takes the interceptable `RequestExit` path.
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

    // Pipeline submenu: `run()`'s `on_menu_event` turns these ids into
    // `menu://…` events the frontend acts on.
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

    let close = PredefinedMenuItem::close_window(handle, None)?;
    let minimize = PredefinedMenuItem::minimize(handle, None)?;
    let zoom = PredefinedMenuItem::maximize(handle, Some("Zoom"))?;
    let window_menu = Submenu::with_items(handle, "Window", true, &[&close, &minimize, &zoom])?;

    Menu::with_items(
        handle,
        &[&app_menu, &edit_menu, &pipeline_menu, &window_menu],
    )
}

/// What to do with one `ExitRequested`/`CloseRequested`, per
/// [`commands::quit_intercept_decision`]. A dialog nobody answers cannot make
/// the app unquittable; see
/// `commands::tests::quit_is_intercepted_once_and_given_up_on_when_nobody_answers`.
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
/// Uses `run`'s callback form so `ExitRequested` and the main window's
/// `CloseRequested` can be intercepted: one window means closing it is
/// app-quit, and both arms share [`quit_decision`]. See
/// `commands::tests::quit_is_intercepted_once_and_given_up_on_when_nobody_answers`.
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
        // A webview reload runs no Svelte `onDestroy`, and this app has
        // exactly one window, so every tail registered before a page load
        // belongs to the page being replaced. See `TailRegistry::stop_all`.
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
        // AppKit's `terminate:` (Dock-menu Quit, logout) cannot be
        // intercepted but still reaches here as `Exit`: a session nobody was
        // asked about is detached ([`commands::detach_on_terminate`]) so the
        // guards' `Drop` fallbacks do not yank a running game's audio device.
        tauri::RunEvent::Exit => {
            let quit_approved = app_handle.state::<commands::QuitApproved>();
            commands::detach_on_terminate(quit_approved.0.load(Ordering::SeqCst));
        }
        _ => {}
    });
}
