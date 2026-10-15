use log::{error, warn};
use tauri::{AppHandle, Manager, Window, WindowEvent};

/// The window shown from launch until the main one has its frontend mounted.
const SPLASH_LABEL: &str = "splashscreen";

/// Swap the splash screen for the main window. The frontend calls this once it
/// has mounted, so the main window, hidden until then, is never seen blank.
#[tauri::command]
pub fn close_splashscreen(app: AppHandle) {
    let Some(main) = app.get_webview_window("main") else {
        error!("no main window to show in place of the splash screen");
        return;
    };
    // Shown before the splash goes, so the screen is never left empty between.
    if let Err(e) = main.show() {
        error!("cannot show the main window: {e}");
    }
    if let Err(e) = main.set_focus() {
        warn!("cannot focus the main window: {e}");
    }
    if let Some(splash) = app.get_webview_window(SPLASH_LABEL) {
        // Not `close`, whose close request would read as the user quitting.
        if let Err(e) = splash.destroy() {
            warn!("cannot close the splash screen: {e}");
        }
    }
}

/// Quit when the splash screen is closed by hand, as from the taskbar: the main
/// window is still hidden, and would keep the launcher running unseen.
pub fn quit_on_splash_closed(window: &Window, event: &WindowEvent) {
    if window.label() == SPLASH_LABEL && matches!(event, WindowEvent::CloseRequested { .. }) {
        window.app_handle().exit(0);
    }
}
