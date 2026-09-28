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
mod log;
mod migrate;
mod panel;
mod rt;
mod selftest;
mod settings;
mod toasts;
mod tray;
mod updater;
mod win32;

use tauri::{Manager, RunEvent};
use tauri_plugin_global_shortcut::ShortcutState;

fn main() {
    let args = crate::core::Args::parse();
    log::init(&args.data_dir, args.debug);
    log!("start {:?}", std::env::args().skip(1).collect::<Vec<_>>());
    migrate::electron_logins(&args.data_dir); // before WebView2 first starts
    if !args.profile && !cfg!(debug_assertions) {
        migrate::electron_leftovers();
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
                .with_handler(|_, _, event| {
                    if event.state() == ShortcutState::Pressed {
                        crate::core::later(|c| c.on_hotkey());
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![crate::core::ui_send])
        .setup(move |app| {
            rt::init(app.handle().clone());
            panel::init(app, args)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("ChatDock could not start");

    app.run(|handle, event| match event {
        // Closing windows never quits: ChatDock lives in the tray until "Quit".
        RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
        RunEvent::Exit => {
            crate::core::with(|c| c.settings.flush());
            let _ = handle.webview_windows(); // keep the handle alive until here
            log!("exit");
        }
        _ => {}
    });
}
