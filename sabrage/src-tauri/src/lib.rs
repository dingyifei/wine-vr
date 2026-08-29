use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    Manager,
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
    let quit = PredefinedMenuItem::quit(handle, Some("Quit Sabrage"))?;
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

    // Pipeline submenu: disabled placeholders, real commands land in Phase 1+.
    let run_doctor = MenuItem::with_id(
        handle,
        "pipeline_run_doctor",
        "Run Doctor",
        false,
        Some("CmdOrCtrl+D"),
    )?;
    let launch = MenuItem::with_id(
        handle,
        "pipeline_launch",
        "Launch",
        false,
        Some("CmdOrCtrl+R"),
    )?;
    let stop = MenuItem::with_id(handle, "pipeline_stop", "Stop", false, Some("CmdOrCtrl+."))?;
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

pub fn run() {
    tauri::Builder::default()
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
            commands::stop_session
        ])
        .manage(commands::RunRegistry::default())
        .setup(|app| {
            let handle = app.handle();
            let menu = build_menu(handle)?;
            app.set_menu(menu)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
