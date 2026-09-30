//! ChatDock: Instagram, Facebook, X and Discord chats in a panel docked on the edge of the screen,
//! always on top of borderless games, with pop-ups that show who wrote.
//!
//! Tauri gives the windows (ChatDock's own pages in ui/), the tray, the global hotkey, one instance
//! only, and signed updates; the chat sites run in WebView2 views of their own (chats.rs).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod apps;
mod autostart;
mod chats;
mod core;
mod edge;
mod frames;
mod i18n;
mod identify;
mod inbox;
mod log;
mod migrate;
mod panel;
mod popups;
mod rt;
mod selftest;
mod settings;
mod toasts;
mod tray;
mod updater;
mod wheel;
mod win32;

use tauri::{Manager, RunEvent};
use tauri_plugin_global_shortcut::ShortcutState;

fn main() {
    let args = crate::core::Args::parse();
    log::init(&args.data_dir, args.debug);
    log!("start {:?}", std::env::args().skip(1).collect::<Vec<_>>());
    // a crash (panic = "abort") leaves its reason in the log instead of just ending
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log!("panic: {info}");
        default_hook(info);
    }));
    if args.selftest && !args.profile {
        // It switches settings around and logs out of apps: never on a real install's data.
        log!("selftest refused: it needs --profile=<a test folder of its own>");
        eprintln!("--selftest needs --profile=<a test folder of its own>: it changes settings and logs out of apps");
        std::process::exit(2);
    }
    // Nothing from outside moves ChatDock onto another WebView2 engine or channel, switches on
    // remote debugging or a script debugger, or sets other engine options: every WEBVIEW2_* variable
    // goes, before any WebView2 exists (the loader reads seven, and more may come).
    for (name, _) in std::env::vars_os() {
        if webview2_variable(&name) {
            std::env::remove_var(&name);
        }
    }

    let mut builder = tauri::Builder::default();
    if !args.profile {
        // Starting ChatDock again (Start menu, desktop icon) opens the one that is already running.
        builder = builder.plugin(tauri_plugin_single_instance::init(|_, argv, _| {
            if argv.iter().any(|a| a == "--hidden") {
                return; // "start with Windows" while it already runs
            }
            crate::core::later(|c| c.open_panel(None, "launch"));
        }));
    }
    let app = builder
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|_, shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        let id = shortcut.id();
                        crate::core::later(move |c| c.on_shortcut(id));
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![crate::core::ui_send])
        .setup(move |app| {
            // Here the single-instance lock is held and no WebView2 has started yet.
            log::allow_rotation();
            migrate::electron_logins(&args.data_dir);
            migrate::electron_caches(&args.data_dir);
            if !args.profile && !cfg!(debug_assertions) {
                migrate::electron_leftovers();
            }
            rt::init(app.handle().clone());
            panel::init(app, args)?;
            Ok(())
        })
        .build(context())
        .expect("ChatDock could not start");

    app.run(|handle, event| match event {
        // Closing windows never quits: ChatDock lives in the tray until "Quit".
        RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
        // Alt+F4 (or any close message) never closes one of ChatDock's windows: the panel slides
        // away instead (see panel.rs), the others stay. Quit and updates use app.exit / destroy.
        RunEvent::WindowEvent { event: tauri::WindowEvent::CloseRequested { api, .. }, .. } => api.prevent_close(),
        RunEvent::Exit => {
            crate::core::with(|c| c.settings.flush());
            let _ = handle.webview_windows(); // keep the handle alive until here
            log!("exit");
        }
        _ => {}
    });
}

/// Windows keeps variable names in any case (WebView2_... is the same variable).
fn webview2_variable(name: &std::ffi::OsStr) -> bool {
    name.to_string_lossy().to_ascii_uppercase().starts_with("WEBVIEW2_")
}

/// The app's configuration, and one exception: a local test feed (updater::test_feed, development
/// builds and the test copy only) is plain http, which the updater otherwise refuses in a release
/// build. Real copies keep it refused.
fn context() -> tauri::Context<tauri::Wry> {
    let mut context = tauri::generate_context!();
    if updater::test_feed().is_some() {
        if let Some(updater) = context.config_mut().plugins.0.get_mut("updater") {
            updater["dangerousInsecureTransportProtocol"] = serde_json::json!(true);
        }
    }
    context
}

#[cfg(test)]
mod tests {
    use super::webview2_variable;

    #[test]
    fn every_webview2_variable_goes() {
        for name in ["WEBVIEW2_RELEASE_CHANNELS", "WEBVIEW2_WAIT_FOR_SCRIPT_DEBUGGER", "WebView2_Use_Edge_View", "WEBVIEW2_"] {
            assert!(webview2_variable(std::ffi::OsStr::new(name)), "{name}");
        }
        for name in ["PATH", "MY_WEBVIEW2_THING", "WEBVIEW2"] {
            assert!(!webview2_variable(std::ffi::OsStr::new(name)), "{name}");
        }
    }
}
