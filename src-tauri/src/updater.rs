//! Updates from the project's GitHub Releases (tauri-plugin-updater; every installer is signed with
//! ChatDock's key and checked before it runs). Checks quietly 20 s after start and every 6 hours,
//! can fetch the installer in the background, and installs only when the user presses the button:
//! the "Updating ChatDock" window, then the installer's own progress window (passive mode), and
//! ChatDock starts again by itself with a "What's new" window ("now on Beta Build 1.x ✓").
//! CHATDOCK_UPDATE_FEED=http://127.0.0.1:8765/latest.json (localhost only) points it at a test feed.

use std::sync::Mutex;

use serde_json::{json, Value};
use tauri::{WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::{
    core::{later, timer, Args, Core, PanelState, Win},
    i18n, log, rt,
    win32::{self, Rect},
};

const RECHECK_MS: u64 = 6 * 60 * 60 * 1000;
const SHOW_MS: u64 = 1800;
/// the What's new window's width, CSS px (its card is 448 px wide, the rest is room for the shadow)
const NEWS_WIDTH: f64 = 480.0;

static PENDING: Mutex<Option<(Update, Option<Vec<u8>>)>> = Mutex::new(None);

/// The "What's new" window after an update.
#[derive(Default)]
pub struct WhatsNew {
    pub win: Option<Win>,
    /// being made (off the main thread)
    building: bool,
    /// the last other window in front: it gets the keyboard back when What's new closes
    fg_before: isize,
    watch: u64,
}

/// While What's new is up: remember the last other window in front (the game, a browser), which
/// gets the keyboard back when the user has clicked What's new and closes it.
fn whats_new_watch(c: &mut Core) {
    rt::cancel(c.whatsnew.watch);
    c.whatsnew.watch = 0;
    let Some(w) = c.whatsnew.win.as_ref() else { return };
    let fg = win32::foreground_window();
    if fg != 0 && fg != w.hwnd && !c.is_ours(fg) {
        c.whatsnew.fg_before = fg;
    }
    c.whatsnew.watch = timer(250, whats_new_watch);
}

/// One of ChatDock's own small windows (update, What's new, monitor numbers): hidden and off screen until its page
/// has said it is ready. Made off the main thread: building a window waits for the main thread.
/// `focused`: whether it may take the keyboard (else its page doesn't grab it as it loads).
pub(crate) fn own_window(
    label: String,
    page: &'static str,
    size: (f64, f64),
    focused: bool,
    args: Args,
    then: impl FnOnce(&mut Core, Result<Win, String>) + Send + 'static,
) {
    std::thread::spawn(move || {
        let built = WebviewWindowBuilder::new(rt::app(), label.clone(), WebviewUrl::App(page.into()))
            .title("ChatDock")
            .visible(false)
            .decorations(false)
            .resizable(false)
            .skip_taskbar(true)
            .always_on_top(true)
            .shadow(false)
            .transparent(true)
            .focused(focused)
            .data_directory(args.webview_dir())
            .additional_browser_args(&args.browser_args())
            .inner_size(size.0, size.1)
            .position(-30000.0, -30000.0)
            .build();
        match built {
            Ok(w) => {
                let hwnd = w.hwnd().map(|h| h.0 as isize).unwrap_or(0);
                win32::set_tool_window(hwnd);
                crate::panel::lock_down_page(&w, args.debug);
                later(move |c| then(c, Ok(Win { w, hwnd })));
            }
            Err(err) => {
                let err = format!("{label} window: {err}");
                later(move |c| then(c, Err(err)));
            }
        }
    });
}

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

/// Start the downloaded installer the way Tauri's updater would (progress bar only, restart
/// ChatDock after), but check that it really started before ChatDock quits.
fn launch_installer(version: &str, bytes: &[u8]) -> Result<(), String> {
    use windows::{
        core::{HSTRING, PCWSTR},
        Win32::{
            Foundation::CloseHandle,
            UI::{
                Shell::{ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
                WindowsAndMessaging::SW_SHOWNORMAL,
            },
        },
    };
    if !bytes.starts_with(b"MZ") {
        return Err("the download is not an installer".into());
    }
    let dir = std::env::temp_dir().join(format!("ChatDock-{version}-update"));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("ChatDock_{version}_x64-setup.exe"));
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    let mut params = String::from("/P /R /UPDATE");
    let keep: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with("--selftest") && a != "--keep").collect();
    if !keep.is_empty() {
        params.push_str(" /ARGS");
        for a in keep {
            params.push(' ');
            params.push_str(&if a.contains(' ') { format!("\"{a}\"") } else { a });
        }
    }
    let (verb, path, params) = (HSTRING::from("open"), HSTRING::from(file.as_os_str()), HSTRING::from(params.as_str()));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(path.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe {
        ShellExecuteExW(&mut info).map_err(|e| format!("the installer did not start: {e}"))?;
        if !info.hProcess.is_invalid() {
            let _ = CloseHandle(info.hProcess);
        }
    }
    log!("updater: installer started ({})", file.display());
    Ok(())
}

/// Update errors as the settings screen understands them: no connection shows its translated
/// "can't reach GitHub" text (it looks for "net::ERR_"); the full reason goes to the log.
fn describe(err: &tauri_plugin_updater::Error) -> String {
    use std::error::Error as _;
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(e) = source {
        text.push_str(": ");
        text.push_str(&e.to_string());
        source = e.source();
    }
    if let tauri_plugin_updater::Error::Reqwest(e) = err {
        if e.is_connect() || e.is_timeout() || e.is_request() || e.is_body() {
            return format!("net::ERR_INTERNET_DISCONNECTED ({text})");
        }
    }
    text
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
                b.build().map_err(|e| e.to_string())?.check().await.map_err(|e| describe(&e))
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
                    let err = describe(&err);
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
        let mut item = item;
        item.chime = self.settings.bool("popupSound");
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
        crate::tray::set_shown(false); // no dead icon left behind in the tray
        log!("updater: starting the installer");
        let version = self.upd.version.clone();
        std::thread::spawn(move || {
            let pending = PENDING.lock().unwrap().take();
            let result = match pending {
                // the download's signature was checked when it arrived (Update::download)
                Some((_, Some(bytes))) => launch_installer(&version, &bytes),
                _ => Err("nothing downloaded".into()),
            };
            match result {
                Ok(()) => later(|c| c.quit()), // the installer runs: step aside (it restarts ChatDock)
                Err(err) => later(move |c| c.install_failed(&err)),
            }
        });
        // Normally ChatDock has quit long before this. If not, something is stuck.
        timer(20_000, |c| c.install_failed("still running"));
    }

    fn install_failed(&mut self, why: &str) {
        if self.upd.status != "installing" {
            return;
        }
        log!("update install did not start: {why}");
        self.quitting = false;
        crate::tray::set_shown(true);
        self.set_setting("pendingUpdate", Value::Null);
        if let Some(w) = self.update_win.take() {
            let _ = w.w.destroy();
        }
        self.upd.error = crate::core::clean_text(why, 300);
        self.set_update("error");
    }

    /// "Updating ChatDock" window, shown for a moment before ChatDock quits for the installer.
    pub fn show_update_window(&mut self, version: &str) {
        let to = version.to_string();
        own_window("update".into(), "update.html", (452.0, 196.0), true, self.args.clone(), move |c, made| match made {
            Ok(w) => {
                c.update_win = Some(w);
                c.pending_update_to = to;
            }
            Err(err) => log!("{err}"),
        });
    }

    /// The update window's page is ready: fill it in and show it in the middle of the screen.
    pub fn update_window_ready(&mut self) {
        let Some(w) = self.update_win.clone() else { return };
        let d = self.target_display();
        let (width, height) = ((452.0 * d.ui).round() as i32, (196.0 * d.ui).round() as i32);
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

    /// "What's new" after an update (see announce_updated).
    pub fn show_whats_new_window(&mut self) {
        if self.whatsnew.win.is_some() || self.whatsnew.building {
            return;
        }
        self.whatsnew.building = true;
        self.ready.remove("whatsnew");
        own_window("whatsnew".into(), "whatsnew.html", (NEWS_WIDTH, 420.0), false, self.args.clone(), |c, made| {
            c.whatsnew.building = false;
            match made {
                Ok(w) => {
                    c.own_hwnds.push(w.hwnd);
                    c.whatsnew.win = Some(w);
                    if c.ready.contains("whatsnew") {
                        c.whats_new_window_ready(); // its page was quicker
                    }
                }
                Err(err) => log!("{err}"),
            }
        });
    }

    /// Its page is ready: send it the texts. It measures the card and asks for its size (below).
    pub fn whats_new_window_ready(&mut self) {
        if self.whatsnew.win.is_none() {
            return;
        }
        let now = rt::version();
        let sections = self.whats_new_sections();
        // one list, for this very release: "What's new" (the title above names the version)
        let only_now = sections.len() == 1 && sections[0].0 == now;
        let sections: Vec<Value> = sections
            .iter()
            .map(|(v, lines)| json!({ "title": if only_now { self.t("wn.heading") } else { self.build_name(v) }, "lines": lines }))
            .collect();
        let route = match self.updated_from() {
            Some(from) if i18n::build(&from).is_some() => format!("{} → {}", self.build_name(&from), self.build_name(&now)),
            _ => String::new(),
        };
        self.emit(
            "whatsnew",
            "whatsnew:show",
            json!([{
                "locale": i18n::locale(&self.lang),
                "title": self.tv("toast.updatedTitle", &[("version", self.build_name(&now))]),
                "route": route,
                "sections": sections,
                "ok": self.t("wn.ok"),
                "github": self.t("wn.github"),
                "close": self.t("toast.close"),
            }]),
        );
    }

    /// The page measured its card (CSS px): the window takes that height (at most 85% of the
    /// screen, the list scrolls then), in the middle of the monitor the mouse is on, and shows
    /// without taking the keyboard from whatever is in front.
    pub fn whats_new_size(&mut self, css_height: f64) {
        let Some(w) = self.whatsnew.win.clone() else { return };
        let shown = win32::is_visible(w.hwnd);
        let (x, y) = if shown {
            let r = win32::window_rect(w.hwnd);
            (r.x + r.w / 2, r.y + r.h / 2)
        } else {
            win32::cursor_pos()
        };
        let d = self.display_at(x, y).unwrap_or_else(|| self.target_display());
        let (w_px, h_px) = ((NEWS_WIDTH * d.ui).round() as i32, ((css_height.max(160.0) * d.ui).round() as i32).min(d.work.h * 85 / 100));
        let r = Rect { x: d.work.x + (d.work.w - w_px) / 2, y: d.work.y + (d.work.h - h_px) / 2, w: w_px, h: h_px };
        win32::set_bounds(w.hwnd, r);
        if !shown {
            win32::show_inactive(w.hwnd);
            win32::raise(w.hwnd);
            log!("what's new shown on {} at {},{} ({} x {})", d.id, r.x, r.y, r.w, r.h);
            whats_new_watch(self);
        }
    }

    /// "Got it", ✕ or Esc; or the GitHub link, which opens the releases page in the browser.
    pub fn close_whats_new(&mut self, releases: bool) {
        let Some(w) = self.whatsnew.win.take() else { return };
        let in_front = win32::foreground_window() == w.hwnd;
        self.own_hwnds.retain(|h| *h != w.hwnd);
        self.ready.remove("whatsnew");
        let _ = w.w.destroy();
        if releases {
            win32::open_url(&format!("{}/releases", crate::core::REPO_URL));
        } else if in_front {
            win32::restore_foreground(self.whatsnew.fg_before);
        }
        log!("what's new closed{}", if releases { " (releases page)" } else { "" });
    }
}
