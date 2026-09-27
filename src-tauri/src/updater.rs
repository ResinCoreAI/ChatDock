//! Updates from the project's GitHub Releases (tauri-plugin-updater; every installer is signed with
//! ChatDock's key and checked before it runs). Checks quietly 20 s after start and every 6 hours,
//! can fetch the installer in the background, and installs only when the user presses the button:
//! the "Updating ChatDock" window, then the installer's own progress window (passive mode), and
//! ChatDock starts again by itself with a "now on Beta Build 1.x ✓" pop-up.
//! CHATDOCK_UPDATE_FEED=http://127.0.0.1:8765/latest.json (localhost only) points it at a test feed.

use std::sync::Mutex;

use serde_json::{json, Value};
use tauri::{WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::{
    core::{later, timer, Core, PanelState, Win},
    log, rt,
    win32::{self, Rect},
};

const RECHECK_MS: u64 = 6 * 60 * 60 * 1000;
const SHOW_MS: u64 = 1800;

static PENDING: Mutex<Option<(Update, Option<Vec<u8>>)>> = Mutex::new(None);

#[derive(Default)]
pub struct UpdState {
    pub status: &'static str,
    pub version: String,
    pub percent: u32,
    pub error: String,
    pub checked_at: i64,
    pub notes: String,
    pub enabled: bool,
    first: u64,
    recheck: u64,
}

/// A local test feed: the folder (like the Electron builds took it) or its latest.json.
fn test_feed() -> Option<String> {
    let feed = std::env::var("CHATDOCK_UPDATE_FEED").ok()?;
    let ok = feed.starts_with("http://127.0.0.1:") || feed.starts_with("http://localhost:");
    if !ok {
        return None;
    }
    Some(if feed.ends_with(".json") { feed } else { format!("{}/latest.json", feed.trim_end_matches('/')) })
}

fn auto_install() -> bool {
    test_feed().is_some() && std::env::var("CHATDOCK_UPDATE_AUTOINSTALL").as_deref() == Ok("1")
}

impl Core {
    pub fn init_updater(&mut self) {
        // development builds and the test copy only ever look at a local test feed
        self.upd.enabled = (!cfg!(debug_assertions) && !crate::core::test_product()) || test_feed().is_some();
        self.upd.status = if self.upd.enabled { "idle" } else { "dev" };
        self.schedule_update_checks();
    }

    pub fn schedule_update_checks(&mut self) {
        rt::cancel(self.upd.first);
        rt::cancel(self.upd.recheck);
        self.upd.first = 0;
        self.upd.recheck = 0;
        if !self.upd.enabled || !self.settings.bool("updateAutoCheck") {
            return;
        }
        self.upd.first = timer(20_000, |c| c.check_update()); // let the chats load first
        self.upd.recheck = timer(RECHECK_MS, |c| {
            c.check_update();
            c.schedule_update_checks();
        });
    }

    pub fn update_state_json(&self) -> Value {
        json!({
            "status": self.upd.status,
            "version": self.upd.version,
            "percent": self.upd.percent,
            "error": self.upd.error,
            "checkedAt": self.upd.checked_at,
            "notes": self.upd.notes,
            "current": rt::version(),
            "enabled": self.upd.enabled,
        })
    }

    fn set_update(&mut self, status: &'static str) {
        self.upd.status = status;
        self.broadcast_state();
    }

    pub fn check_update(&mut self) {
        if !self.upd.enabled || ["checking", "downloading", "ready", "installing"].contains(&self.upd.status) {
            return;
        }
        self.upd.error.clear();
        self.set_update("checking");
        tauri::async_runtime::spawn(async move {
            let result: Result<Option<Update>, String> = async {
                let mut b = rt::app().updater_builder();
                if let Some(feed) = test_feed() {
                    let url = tauri::Url::parse(&feed).map_err(|e| e.to_string())?;
                    b = b.endpoints(vec![url]).map_err(|e| e.to_string())?;
                }
                b.build().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())
            }
            .await;
            match result {
                Ok(Some(update)) => {
                    let version = update.version.clone();
                    let notes = update.body.clone().unwrap_or_default();
                    *PENDING.lock().unwrap() = Some((update, None));
                    later(move |c| c.update_found(version, notes));
                }
                Ok(None) => later(|c| {
                    c.upd.checked_at = rt::epoch_ms();
                    c.upd.version.clear();
                    log!("updater: on the latest version");
                    c.set_update("latest");
                }),
                Err(err) => later(move |c| {
                    log!("updater error: {err}");
                    c.upd.error = crate::core::clean_text(&err, 300);
                    c.set_update("error");
                }),
            }
        });
    }

    fn update_found(&mut self, version: String, notes: String) {
        log!("updater: found version {version}");
        self.upd.version = version;
        self.upd.notes = notes.trim().chars().take(1500).collect();
        self.upd.percent = 0;
        if self.settings.bool("updateAutoDownload") {
            self.set_update("available");
            self.download_update();
        } else {
            self.set_update("available");
            let v = self.upd.version.clone();
            self.announce_update(&v, false);
        }
    }

    pub fn download_update(&mut self) {
        if self.upd.status != "available" {
            return;
        }
        let Some((update, _)) = PENDING.lock().unwrap().clone() else { return };
        self.upd.percent = 0;
        self.set_update("downloading");
        tauri::async_runtime::spawn(async move {
            let mut got: u64 = 0;
            let mut last = 0u32;
            let result = update
                .download(
                    |chunk, total| {
                        got += chunk as u64;
                        if let Some(total) = total.filter(|t| *t > 0) {
                            let pct = ((got * 100) / total).min(100) as u32;
                            if pct >= last + 5 {
                                last = pct;
                                later(move |c| {
                                    c.upd.percent = pct;
                                    c.broadcast_state();
                                });
                            }
                        }
                    },
                    || {},
                )
                .await;
            match result {
                Ok(bytes) => {
                    if let Some(p) = PENDING.lock().unwrap().as_mut() {
                        p.1 = Some(bytes);
                    }
                    later(|c| c.update_ready());
                }
                Err(err) => {
                    let err = err.to_string();
                    later(move |c| {
                        log!("updater error: {err}");
                        c.upd.error = crate::core::clean_text(&err, 300);
                        c.set_update("error");
                    });
                }
            }
        });
    }

    fn update_ready(&mut self) {
        log!("updater: version {} has been downloaded", self.upd.version);
        self.upd.percent = 100;
        self.upd.checked_at = rt::epoch_ms();
        self.set_update("ready");
        let v = self.upd.version.clone();
        self.announce_update(&v, true);
        if auto_install() {
            timer(1500, |c| c.install_update()); // update test with a local feed: no button press
        }
    }

    /// A new version is downloaded (or found, when downloading is left to the user): one pop-up per
    /// version. The update button in the panel header stays until it is installed.
    pub fn announce_update(&mut self, version: &str, ready: bool) {
        if version.is_empty() || self.update_announced == version {
            return;
        }
        self.update_announced = version.to_string();
        self.broadcast_state();
        if !self.popup_allowed(None) {
            return;
        }
        let name = self.build_name(version);
        let item = crate::toasts::Item::new_own(
            "chatdock",
            "ChatDock",
            "logo",
            "#8b5cf6",
            self.t("toast.update"),
            self.tv(if ready { "toast.updReadyTitle" } else { "toast.updAvailTitle" }, &[("version", name)]),
            self.t(if ready { "toast.updReadyBody" } else { "toast.updAvailBody" }),
            self.t("toast.updHint"),
            "chatdock:update",
            "update",
        );
        self.toasts_push(item);
    }

    /// Show the "Updating" window, then hand over to the installer (it shows its own progress and
    /// starts the new version when it is done).
    pub fn install_update(&mut self) {
        if self.upd.status != "ready" {
            return;
        }
        let version = self.upd.version.clone();
        log!("installing update {version}");
        self.set_update("installing");
        // Remembered so the new version can say "updated" and show what's new when it starts.
        let notes = self.upd.notes.clone();
        self.set_setting("pendingUpdate", json!({ "from": rt::version(), "to": version, "notes": notes }));
        self.settings.flush();
        self.toasts_dismiss_all();
        self.hide_tab(true);
        if self.panel_state != PanelState::Hidden {
            self.close_panel(false, "update");
        }
        self.show_update_window(&version);
        timer(SHOW_MS + 200, |c| c.run_installer());
    }

    fn run_installer(&mut self) {
        self.quitting = true;
        self.settings.flush();
        log!("updater: starting the installer");
        std::thread::spawn(|| {
            let pending = PENDING.lock().unwrap().take();
            let result = match pending {
                Some((update, Some(bytes))) => update.install(bytes).map_err(|e| e.to_string()), // exits ChatDock on success
                _ => Err("nothing downloaded".into()),
            };
            if let Err(err) = result {
                later(move |c| c.install_failed(&err));
            }
        });
        // Normally ChatDock has quit long before this. If not, the installer didn't start.
        timer(20_000, |c| c.install_failed("still running"));
    }

    fn install_failed(&mut self, why: &str) {
        if self.upd.status != "installing" {
            return;
        }
        log!("update install did not start: {why}");
        self.quitting = false;
        self.set_setting("pendingUpdate", Value::Null);
        if let Some(w) = self.update_win.take() {
            let _ = w.w.destroy();
        }
        self.upd.error = crate::core::clean_text(why, 300);
        self.set_update("error");
    }

    /// "Updating ChatDock" window, shown for a moment before ChatDock quits for the installer.
    pub fn show_update_window(&mut self, version: &str) {
        let args = self.args.clone();
        let to = version.to_string();
        // made off the main thread: building a window waits for the main thread to create it
        std::thread::spawn(move || {
            let built = WebviewWindowBuilder::new(rt::app(), "update", WebviewUrl::App("update.html".into()))
                .title("ChatDock")
                .visible(false)
                .decorations(false)
                .resizable(false)
                .skip_taskbar(true)
                .always_on_top(true)
                .shadow(false)
                .transparent(true)
                .data_directory(args.webview_dir())
                .additional_browser_args(&args.browser_args())
                .inner_size(452.0, 196.0)
                .position(-30000.0, -30000.0)
                .build();
            match built {
                Ok(w) => {
                    let hwnd = w.hwnd().map(|h| h.0 as isize).unwrap_or(0);
                    win32::set_tool_window(hwnd);
                    later(move |c| {
                        c.update_win = Some(Win { w, hwnd });
                        c.pending_update_to = to;
                    });
                }
                Err(err) => log!("update window: {err}"),
            }
        });
    }

    /// The update window's page is ready: fill it in and show it in the middle of the screen.
    pub fn update_window_ready(&mut self) {
        let Some(w) = self.update_win.clone() else { return };
        let d = self.target_display();
        let (width, height) = ((452.0 * d.scale).round() as i32, (196.0 * d.scale).round() as i32);
        win32::set_bounds(
            w.hwnd,
            Rect { x: d.work.x + (d.work.w - width) / 2, y: d.work.y + (d.work.h - height) / 2, w: width, h: height },
        );
        let from = self.build_name(&rt::version());
        let to = self.build_name(&self.pending_update_to.clone());
        self.emit("update", "update:show", json!([{ "lang": self.lang, "from": from, "to": to, "ms": SHOW_MS }]));
        win32::show(w.hwnd);
        win32::raise(w.hwnd);
    }
}
