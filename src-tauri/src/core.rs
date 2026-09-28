//! The app state and the rules that decide what happens. It lives on the main thread only;
//! everything else reaches it through later() / timer() (see rt.rs).
//!
//!   hidden --(cursor held on the dock edge)--> white tab --(click)--> panel slides in
//!   hidden --(global hotkey / tray click)-------------------------> panel slides in
//!   panel  --(click elsewhere / hotkey / Esc Esc / hide button)----> slides out, focus back to the game

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use serde_json::{json, Map, Value};
use tauri::{Emitter, WebviewWindow};

use crate::{apps, autostart, chats, edge, i18n, log, rt, settings::Settings, toasts, tray, updater, win32};

pub const REPO_URL: &str = "https://github.com/ResinCoreAI/ChatDock";
/// A throw-away copy of ChatDock for testing installs and updates ("ChatDockUpdTest.exe"): its own
/// data folder, "start with Windows" entry and single-instance lock, so it never touches the real one.
pub const TEST_PRODUCT: &str = "ChatDockUpdTest";

pub fn test_product() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().eq_ignore_ascii_case(TEST_PRODUCT)))
        .unwrap_or(false)
}
pub const HOTKEYS: [(&str, &str); 4] = [
    ("Control+Alt+C", "Ctrl + Alt + C"),
    ("Control+Alt+Space", "Ctrl + Alt + Space"),
    ("Control+Shift+Space", "Ctrl + Shift + Space"),
    ("Control+Alt+Z", "Ctrl + Alt + Z"),
];
pub const ZOOM_STEPS: [f64; 8] = [0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5];
const SLEEP_AFTER_MS: i64 = 10 * 60 * 1000;
const COUNT_POPUP_GRACE_MS: i64 = 20_000;
pub const AUTO_RETRY_MS: u64 = 15_000;

// ---------------------------------------------------------------------------------------------
// Command line
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Debug)]
pub struct Args {
    pub data_dir: PathBuf,
    pub profile: bool, // --profile=<dir>: a separate data folder (tests); never touches "start with Windows"
    pub selftest: bool,
    pub selftest_only: Option<String>,
    pub shots: Option<PathBuf>,
    pub hidden: bool, // launched by "start with Windows"
    pub debug: bool,
    pub no_occlusion: bool,
}

impl Args {
    pub fn parse() -> Self {
        let argv: Vec<String> = std::env::args().skip(1).collect();
        let value = |name: &str| {
            let prefix = format!("--{name}=");
            argv.iter().find_map(|a| a.strip_prefix(&prefix).map(str::to_string))
        };
        let has = |name: &str| argv.iter().any(|a| a == &format!("--{name}"));
        let profile = value("profile").map(PathBuf::from);
        let selftest = has("selftest");
        let data_dir = profile.clone().unwrap_or_else(|| {
            let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            // development builds and the test copy never touch a real install's data
            base.join(if test_product() {
                TEST_PRODUCT
            } else if cfg!(debug_assertions) {
                "ChatDock-dev"
            } else {
                "ChatDock"
            })
        });
        Args {
            data_dir,
            profile: profile.is_some(),
            selftest,
            selftest_only: value("selftest-only"),
            shots: value("shots").map(PathBuf::from),
            hidden: has("hidden"),
            debug: selftest || has("debug") || cfg!(debug_assertions),
            no_occlusion: has("no-occlusion"),
        }
    }

    pub fn webview_dir(&self) -> PathBuf {
        self.data_dir.join("WebView2")
    }

    /// Chromium switches for every WebView2 in ChatDock (they must be the same for all of them):
    /// no Windows passkey dialog, no WebRTC mDNS (it trips a firewall prompt) and local addresses
    /// never offered to WebRTC, less RAM (one process per site, no spare renderer, no back/forward
    /// cache), sound without a click first, and Edge's own extra UI and SmartScreen lookups off.
    pub fn browser_args(&self) -> String {
        let mut off = vec![
            "WebAuthenticationUseNativeWinApi",
            "WebRtcHideLocalIpsWithMdns",
            "SpareRendererForSitePerProcess",
            "BackForwardCache",
            "msWebOOUI",
            "msPdfOOUI",
            "msSmartScreenProtection",
        ];
        if self.no_occlusion {
            off.push("CalculateNativeWinOcclusion");
        }
        format!(
            "--disable-features={} --process-per-site --force-webrtc-ip-handling-policy=default_public_interface_only --autoplay-policy=no-user-gesture-required",
            off.join(",")
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PanelState {
    Hidden,
    Opening,
    Open,
    Closing,
}

impl PanelState {
    pub fn as_str(self) -> &'static str {
        match self {
            PanelState::Hidden => "hidden",
            PanelState::Opening => "opening",
            PanelState::Open => "open",
            PanelState::Closing => "closing",
        }
    }
    pub fn showing(self) -> bool {
        matches!(self, PanelState::Open | PanelState::Opening)
    }
}

#[derive(Clone)]
pub struct Win {
    pub w: WebviewWindow,
    pub hwnd: isize,
}

pub struct Core {
    pub args: Args,
    pub settings: Settings,
    pub lang: String,
    pub panel: Win,
    pub tab: Win,
    pub glow: Win,
    pub edgewin: Win,
    pub toastwin: Win,
    pub update_win: Option<Win>,
    pub ready: HashSet<String>,
    pub own_hwnds: Vec<isize>,
    /// the panel page's own child windows (Tauri's WebView): kept under the chat views
    pub panel_page_hwnds: Vec<isize>,
    /// chat views with a reload coming after their renderer crashed
    pub reload_pending: HashSet<String>,
    /// ChatDock is starting itself again (WebView2 stopped)
    pub recovering: bool,
    /// "Automatic" monitor: the one the dock is on right now (where the edge was used last, or where
    /// the mouse was when the hotkey was pressed); "" = the outermost monitor on the dock side
    pub dock_display: String,
    /// monitor scale and page scale the layout was last made for (to notice a Text size change)
    pub applied_scale: (f64, f64),
    /// ChatDock's own pages are being loaded again after their renderer crashed
    pub ui_reload_pending: bool,
    pub ui_crashes: Vec<i64>,
    /// the panel's current see-through level (255 = solid)
    pub panel_alpha: u8,
    /// while the resize grip is dragged: cursor x minus the panel's inner edge, physical px
    pub resize_grab: Option<i32>,
    pub chats: chats::Chats,
    // per app
    pub counts: HashMap<String, u32>,
    pub load_state: HashMap<String, &'static str>,
    pub first_shown: HashMap<String, bool>,
    pub asleep: HashMap<String, bool>,
    pub last_used: HashMap<String, i64>,
    pub zero_timers: HashMap<String, u64>,
    pub retry_timers: HashMap<String, u64>,
    pub fallback_timers: HashMap<String, u64>,
    pub last_flash: HashMap<String, (String, i64)>,
    pub last_content_at: HashMap<String, i64>,
    pub load_started_at: HashMap<String, i64>,
    // panel
    pub panel_state: PanelState,
    pub help_mode: bool,
    pub settings_mode: bool,
    pub banner_shown: bool,
    pub banner_dismissed: bool,
    pub prev_foreground: isize,
    pub last_auto_hide_at: i64,
    pub blurred_while_opening: bool,
    pub last_esc_at: i64,
    pub anim_gen: u64,
    pub anim_frames: u32,
    pub settings_timer: u64,
    pub toast_foreground: isize,
    // edge, pop-ups, updates
    pub edge: edge::EdgeState,
    pub toasts: toasts::Toasts,
    pub upd: updater::UpdState,
    pub update_announced: String,
    pub pending_update_to: String,
    pub hotkey_ok: bool,
    pub autostart_cache: bool,
    pub dnd_timer: u64,
    pub save_timer: u64,
    pub quitting: bool,
}

thread_local! {
    static CORE: RefCell<Option<Core>> = const { RefCell::new(None) };
}

/// Windows is signing out or shutting down (set from the panel's window messages).
pub static SESSION_ENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn install(core: Core) {
    CORE.with(|c| *c.borrow_mut() = Some(core));
}

/// Run f with the app state, right now (main thread only). None if it is already in use: code that
/// can run inside another handler queues its work with later() instead.
pub fn with<R>(f: impl FnOnce(&mut Core) -> R) -> Option<R> {
    CORE.with(|cell| match cell.try_borrow_mut() {
        Ok(mut guard) => guard.as_mut().map(f),
        Err(_) => None,
    })
}

type Job = Box<dyn FnOnce(&mut Core) + Send>;

fn run_job(f: Job) {
    let not_now = CORE.with(|cell| match cell.try_borrow_mut() {
        Ok(mut guard) => match guard.as_mut() {
            Some(c) => {
                f(c);
                None
            }
            None => Some(f), // a page that loaded before the app state was set up
        },
        // It arrived through a message loop running inside other work on the app state (a menu, a
        // window message sent while that work moved a window): run it as soon as that is done.
        Err(_) => Some(f),
    });
    if let Some(f) = not_now {
        rt::after(5, move || run_job(f));
    }
}

/// Queue work for the app state, from any thread and from inside any callback.
pub fn later(f: impl FnOnce(&mut Core) + Send + 'static) {
    let job: Job = Box::new(f);
    rt::post(move || run_job(job));
}

/// Run work for the app state in `ms` milliseconds (returns an id for rt::cancel).
pub fn timer(ms: u64, f: impl FnOnce(&mut Core) + Send + 'static) -> u64 {
    let job: Job = Box::new(f);
    rt::after(ms, move || run_job(job))
}

/// A page sent a message (the Electron build's ipcRenderer.send channels, unchanged).
#[tauri::command]
pub fn ui_send(webview: tauri::Webview, channel: String, args: Vec<Value>) {
    let from = webview.label().to_string();
    later(move |c| c.on_ui(&from, &channel, args));
}

fn arg_str(args: &[Value], i: usize) -> String {
    args.get(i).and_then(Value::as_str).unwrap_or("").to_string()
}

pub fn clean_text(value: &str, max: usize) -> String {
    let s: String = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() > max {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    } else {
        s
    }
}

impl Core {
    pub fn t(&self, key: &str) -> String {
        i18n::t(&self.lang, key, &[])
    }

    pub fn tv(&self, key: &str, vars: &[(&str, String)]) -> String {
        i18n::t(&self.lang, key, vars)
    }

    pub fn build_name(&self, version: &str) -> String {
        i18n::build_name(&self.lang, version)
    }

    pub fn ui_lang(&self) -> String {
        let chosen = self.settings.str("lang");
        if i18n::is_lang(chosen) {
            chosen.to_string()
        } else {
            i18n::system_lang()
        }
    }

    pub fn emit(&self, label: &str, channel: &str, args: Value) {
        let _ = rt::app().emit_to(label, channel, args);
    }

    /// Settings are written shortly after the last change.
    pub fn save_soon(&mut self) {
        if self.save_timer != 0 || !self.settings.dirty {
            return;
        }
        self.save_timer = timer(400, |c| {
            c.save_timer = 0;
            c.settings.flush();
        });
    }

    pub fn set_setting(&mut self, key: &str, value: Value) {
        self.settings.set(key, value);
        self.save_soon();
    }

    pub fn enabled_apps(&self) -> Vec<&'static str> {
        apps::CATALOG.iter().filter(|a| self.settings.app_on(a.id)).map(|a| a.id).collect()
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        apps::get(id).is_some() && self.settings.app_on(id)
    }

    pub fn active(&self) -> String {
        self.settings.str("active").to_string()
    }

    pub fn is_ours(&self, hwnd: isize) -> bool {
        hwnd != 0 && (self.own_hwnds.contains(&hwnd) || self.own_hwnds.contains(&win32::root_owner(hwnd)))
    }

    // -----------------------------------------------------------------------------------------
    // Messages from the pages
    // -----------------------------------------------------------------------------------------
    pub fn on_ui(&mut self, from: &str, channel: &str, args: Vec<Value>) {
        let panel = from == "panel";
        match (channel, from) {
            ("ui:ready", _) => {
                self.ready.insert(from.to_string());
                match from {
                    "panel" | "tab" | "glow" => self.emit(from, "state", json!([self.ui_state()])),
                    "update" => self.update_window_ready(),
                    "toasts" => self.toasts_ready(),
                    _ => {}
                }
                if from == "panel" {
                    self.panel_page_ready();
                }
            }
            ("app:select", "panel") => {
                let id = arg_str(&args, 0);
                if !self.is_enabled(&id) {
                    return;
                }
                if self.settings_mode {
                    // an app tab leaves the settings screen (set_active wakes a sleeping app)
                    self.set_active(&id, false);
                    self.close_settings();
                } else if self.help_mode {
                    // clicking an app on the welcome / help screen means "take me there"
                    self.set_active(&id, false);
                    self.finish_onboarding(None);
                } else if id == self.active() && self.view_showable(&id) {
                    self.go_home(&id);
                } else {
                    self.set_active(&id, true);
                }
            }
            ("panel:reload" | "panel:retry", "panel") => {
                let id = self.active();
                self.reload_app(&id, false);
            }
            ("panel:pin", "panel") => {
                let pinned = !self.settings.bool("pinned");
                self.set_pref("pinned", json!(pinned));
            }
            ("panel:hide", "panel") => self.close_panel(true, "hide button"),
            ("panel:settings", "panel") => {
                if self.settings_mode {
                    self.close_settings()
                } else {
                    self.open_settings("", "header")
                }
            }
            ("panel:update", "panel") => {
                if self.upd.status == "ready" {
                    self.install_update();
                } else {
                    self.open_settings("updates", "header");
                }
            }
            ("settings:set", "panel") => {
                let key = arg_str(&args, 0);
                self.set_pref(&key, args.get(1).cloned().unwrap_or(Value::Null));
            }
            ("settings:app", "panel") => {
                if let Some(on) = args.get(1).and_then(Value::as_bool) {
                    self.set_app_enabled(&arg_str(&args, 0), on);
                }
            }
            ("settings:app-pref", "panel") => {
                if let Some(v) = args.get(2).and_then(Value::as_bool) {
                    self.set_app_pref(&arg_str(&args, 0), &arg_str(&args, 1), v);
                }
            }
            ("settings:action", "panel") => {
                let name = arg_str(&args, 0);
                self.settings_action(&name, args.get(1).cloned().unwrap_or(Value::Null));
            }
            ("panel:resize-start", "panel") => self.resize_start(),
            ("panel:resize", "panel") => self.resize_to(),
            ("panel:resize-end", "panel") => {
                self.resize_to();
                self.resize_grab = None;
            }
            ("panel:zoom-reset", "panel") => {
                let id = self.active();
                self.zoom_step(&id, 0);
            }
            ("onboarding:done", "panel") => {
                let autostart = args.first().and_then(|o| o.get("autostart")).and_then(Value::as_bool);
                self.finish_onboarding(autostart);
            }
            ("banner:dismiss", "panel") => {
                self.banner_shown = false;
                self.banner_dismissed = true;
                self.layout_views();
                self.broadcast_state();
            }
            ("tab:log", "tab") => log!("tab renderer: {}", clean_text(&arg_str(&args, 0), 200)),
            ("tab:open", "tab") => {
                let id = arg_str(&args, 0);
                self.on_tab_open(&id);
            }
            (c, "toasts") if c.starts_with("toast:") => self.on_toast_message(c, &args),
            _ => {
                if !panel {
                    log!("ignored message {channel} from {from}");
                }
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // State for the pages
    // -----------------------------------------------------------------------------------------
    pub fn hotkey_label(&self) -> String {
        let acc = self.settings.str("hotkey");
        if acc.is_empty() {
            return String::new();
        }
        let label = HOTKEYS
            .iter()
            .find(|(a, _)| *a == acc)
            .map(|(_, l)| l.to_string())
            .unwrap_or_else(|| acc.replace("Control", "Ctrl").split('+').collect::<Vec<_>>().join(" + "));
        label.replace("Ctrl", &self.t("key.ctrl"))
    }

    pub fn shown_count(&self, id: &str) -> u32 {
        if self.settings.app_pref(id, "badge") {
            *self.counts.get(id).unwrap_or(&0)
        } else {
            0
        }
    }

    pub fn total_unread(&self) -> u32 {
        self.enabled_apps().iter().map(|id| self.shown_count(id)).sum()
    }

    pub fn preferred_app(&self) -> String {
        let with_unread: Vec<_> = self.enabled_apps().into_iter().filter(|id| self.shown_count(id) > 0).collect();
        if with_unread.len() == 1 {
            with_unread[0].to_string()
        } else {
            self.active()
        }
    }

    pub fn zoom_of(&self, id: &str) -> f64 {
        self.settings.get("zoom").get(id).and_then(Value::as_f64).unwrap_or(1.0)
    }

    pub fn ui_state(&self) -> Value {
        let active = self.active();
        let apps: Vec<Value> = self
            .enabled_apps()
            .iter()
            .map(|id| {
                let a = apps::get(id).unwrap();
                json!({ "id": a.id, "name": a.name, "icon": a.icon, "asleep": *self.asleep.get(*id).unwrap_or(&false) })
            })
            .collect();
        let counts: Map<String, Value> = apps::ids().map(|id| (id.to_string(), json!(self.shown_count(id)))).collect();
        let load: Map<String, Value> =
            apps::ids().map(|id| (id.to_string(), json!(self.load_state.get(id).copied().unwrap_or("loading")))).collect();
        let first: Map<String, Value> =
            apps::ids().map(|id| (id.to_string(), json!(*self.first_shown.get(id).unwrap_or(&false)))).collect();
        json!({
            "apps": apps,
            "active": active,
            "counts": counts,
            "lang": self.lang,
            "langPref": self.settings.str("lang"),
            "load": load,
            "firstShown": first,
            "pinned": self.settings.bool("pinned"),
            "help": self.help_mode,
            "settingsOpen": self.settings_mode,
            "onboarded": self.settings.bool("onboarded"),
            "banner": self.banner_shown,
            "hotkey": if self.hotkey_ok { self.hotkey_label() } else { String::new() },
            "zoom": self.zoom_of(&active),
            "canAutostart": self.can_autostart(),
            "autostart": self.autostart_cache,
            "panel": self.panel_state.as_str(),
            "side": self.settings.str("side"),
            "dnd": self.dnd_active(),
            "update": {
                "status": self.upd.status,
                "version": self.upd.version,
                "percent": self.upd.percent,
                "build": i18n::build(&self.upd.version),
                "name": self.build_name(&self.upd.version),
            },
        })
    }

    pub fn settings_state(&self) -> Value {
        let mut prefs = Map::new();
        for key in PREF_KEYS {
            if key != "autostart" && key != "displayId" {
                prefs.insert(key.to_string(), self.settings.get(key).clone());
            }
        }
        prefs.insert("autostart".into(), json!(self.autostart_cache));
        prefs.insert("displayId".into(), json!(if self.auto_display() { "auto".to_string() } else { self.target_display().id }));
        let memory = self.memory_stats();
        let catalog: Vec<Value> = apps::CATALOG
            .iter()
            .map(|a| {
                json!({
                    "id": a.id, "name": a.name, "icon": a.icon, "on": self.is_enabled(a.id),
                    "asleep": *self.asleep.get(a.id).unwrap_or(&false),
                    "mb": memory.per_app.get(a.id).copied().unwrap_or(0),
                    "prefs": self.settings.app_prefs(a.id),
                })
            })
            .collect();
        let mut displays: Vec<Value> = win32::displays()
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let primary = if d.primary { self.t("display.primary") } else { String::new() };
                json!({ "id": d.id, "label": format!("{}{} — {}×{}", self.tv("display.label", &[("n", (i + 1).to_string())]), primary, d.bounds.w, d.bounds.h) })
            })
            .collect();
        if displays.len() > 1 {
            displays.insert(0, json!({ "id": "auto", "label": self.t("display.auto") }));
        }
        let version = rt::version();
        json!({
            "prefs": prefs,
            "catalog": catalog,
            "memory": memory.total,
            "langs": i18n::LANGS.iter().map(|(id, name)| json!({ "id": id, "name": name })).collect::<Vec<_>>(),
            "displays": displays,
            "hotkeys": HOTKEYS.iter().map(|(acc, label)| json!({ "acc": acc, "label": label.replace("Ctrl", &self.t("key.ctrl")) })).collect::<Vec<_>>(),
            "hotkeyOk": self.hotkey_ok,
            "dndUntil": self.settings.get("dndUntil"),
            "update": self.update_state_json(),
            "version": version,
            "build": self.build_name(&version),
            "updateName": self.build_name(&self.upd.version),
            "whatsNew": self.whats_new_state(),
            "packaged": !cfg!(debug_assertions),
            "autostartAvailable": self.can_autostart(),
            "cookieEncryption": true,
            "cookiesMigrated": true,
            "repo": REPO_URL,
        })
    }

    pub fn broadcast_state(&mut self) {
        let s = self.ui_state();
        if self.ready.contains("panel") {
            let mut p = s.clone();
            if self.settings_mode {
                p["settings"] = self.settings_state();
            }
            self.emit("panel", "state", json!([p]));
        }
        if self.ready.contains("tab") {
            self.emit("tab", "state", json!([s]));
        }
        self.update_tray();
    }

    // -----------------------------------------------------------------------------------------
    // Settings: every value the settings screen may change, checked before it is used
    // -----------------------------------------------------------------------------------------
    fn pref_ok(&self, key: &str, v: &Value) -> bool {
        let one_of = |list: &[&str]| v.as_str().is_some_and(|s| list.contains(&s));
        let num_in = |list: &[f64]| v.as_f64().is_some_and(|n| list.iter().any(|x| (x - n).abs() < 1e-9));
        match key {
            "lang" => v.as_str().is_some_and(|s| s == "auto" || i18n::is_lang(s)),
            "side" => one_of(&["right", "left"]),
            "edgeMode" => one_of(&["always", "no-fullscreen", "off"]),
            "edgeHold" => num_in(&[0.0, 0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0]),
            "displayId" => v.as_str().is_some_and(|s| s == "auto" || win32::displays().iter().any(|d| d.id == s)),
            "theme" => one_of(&["system", "dark", "light"]),
            "opacity" => v.as_f64().is_some_and(|n| (0.6..=1.0).contains(&n)),
            "hotkey" => v.as_str().is_some_and(|s| s.is_empty() || HOTKEYS.iter().any(|(a, _)| *a == s)),
            "popupPosition" => one_of(&["top-right", "bottom-right", "top-left", "bottom-left"]),
            "popupDuration" => num_in(&[0.0, 5.0, 8.0, 12.0, 20.0, 30.0]),
            "popupMax" => num_in(&[1.0, 2.0, 3.0, 4.0, 5.0]),
            "glow"
            | "pinned"
            | "muted"
            | "autostart"
            | "hideFromCapture"
            | "popups"
            | "popupText"
            | "popupAvatar"
            | "popupSound"
            | "popupQuietFullscreen"
            | "updateAutoCheck"
            | "updateAutoDownload" => v.is_boolean(),
            _ => false,
        }
    }

    pub fn set_pref(&mut self, key: &str, value: Value) -> bool {
        if !self.pref_ok(key, &value) {
            log!("setting rejected {}", clean_text(key, 40));
            return false;
        }
        match key {
            "lang" => {
                self.set_setting("lang", value);
                self.lang = self.ui_lang();
                self.toasts_refresh();
                log!("language {} -> {}", self.settings.str("lang"), self.lang);
            }
            "side" => self.set_side(value.as_str().unwrap_or("right")),
            "edgeMode" => {
                self.set_setting("edgeMode", value);
                if self.settings.str("edgeMode") == "off" {
                    self.hide_tab(true);
                }
            }
            "edgeHold" => {
                self.set_setting("edgeHold", value);
                self.stop_hold(false);
            }
            "displayId" => {
                let id = value.as_str().unwrap_or("").to_string();
                self.set_setting("displayId", if id == "auto" { Value::Null } else { json!(id) });
                self.dock_display.clear();
                self.on_displays_changed();
            }
            "glow" => {
                self.set_setting("glow", value);
                self.update_glow(false);
            }
            "theme" => {
                self.set_setting("theme", value);
                self.apply_theme();
            }
            "opacity" => {
                self.set_setting("opacity", value);
                if self.panel_state == PanelState::Open {
                    let a = self.panel_opacity();
                    self.set_panel_alpha(a);
                }
            }
            "muted" => {
                self.set_setting("muted", value);
                for id in apps::ids() {
                    self.apply_audio(id);
                }
            }
            "hotkey" => {
                self.set_setting("hotkey", value);
                self.register_hotkey();
            }
            "autostart" => self.set_open_at_login(value.as_bool().unwrap_or(false)),
            "hideFromCapture" => {
                self.set_setting("hideFromCapture", value);
                self.apply_capture_protection();
            }
            "popups" => {
                self.set_setting("popups", value);
                if !self.settings.bool("popups") {
                    self.toasts_dismiss_all();
                }
            }
            "popupPosition" | "popupMax" => {
                self.set_setting(key, value);
                self.toasts_refresh();
            }
            "updateAutoCheck" => {
                self.set_setting(key, value);
                self.schedule_update_checks();
            }
            _ => self.set_setting(key, value),
        }
        self.broadcast_state();
        true
    }

    fn settings_action(&mut self, name: &str, arg: Value) {
        match name {
            "close" => self.close_settings(),
            "test-popup" => self.test_popup(),
            "dnd" => {
                if let Some(m) = arg.as_i64().filter(|m| [0, 30, 60, 120, 480, -1].contains(m)) {
                    self.set_dnd(m);
                }
            }
            "clear-app" => {
                if let Some(id) = arg.as_str().filter(|id| apps::get(id).is_some()) {
                    self.clear_app_data(id);
                }
            }
            "clear-all" => {
                for id in apps::ids() {
                    self.clear_app_data(id);
                }
            }
            "reset-widths" => {
                self.set_setting("widths", json!({}));
                self.fit_panel_to_app();
            }
            "check-update" => self.check_update(),
            "download-update" => self.download_update(),
            "install-update" => self.install_update(),
            "open-releases" => win32::open_url(&format!("{REPO_URL}/releases")),
            "open-repo" => win32::open_url(REPO_URL),
            "open-license" => win32::open_url(&format!("{REPO_URL}/blob/main/LICENSE")),
            "open-logs" => {
                if let Some(p) = log::path() {
                    win32::show_in_folder(p);
                }
            }
            "help" => self.show_help(),
            "quit" => self.quit(),
            _ => log!("unknown settings action {}", clean_text(name, 40)),
        }
    }

    // -----------------------------------------------------------------------------------------
    // Apps on/off, per-app switches, sleep (the RAM saver)
    // -----------------------------------------------------------------------------------------
    pub fn set_app_enabled(&mut self, id: &str, on: bool) {
        if apps::get(id).is_none() || self.is_enabled(id) == on {
            return;
        }
        if !on && self.enabled_apps().len() <= 1 {
            return; // keep at least one app
        }
        self.settings.set_in("apps", id, json!(on));
        self.save_soon();
        self.asleep.insert(id.to_string(), false);
        if on {
            self.create_view(id);
        } else {
            self.destroy_view(id);
            self.toasts_dismiss_app(id);
            if self.active() == id {
                let first = self.enabled_apps()[0];
                self.set_setting("active", json!(first));
            }
        }
        log!("app {id} {} {:?}", if on { "on" } else { "off" }, self.enabled_apps());
        if self.edge.tab_shown {
            let cy = self.edge.tab_center_y;
            self.place_tab(cy); // the tab grows / shrinks with the number of apps
        }
        self.layout_views();
        self.broadcast_state();
        self.update_glow(false);
    }

    pub fn set_app_pref(&mut self, id: &str, key: &str, value: bool) -> bool {
        if apps::get(id).is_none() || !crate::settings::APP_PREF_KEYS.contains(&key) {
            return false;
        }
        self.settings.set_app_pref(id, key, value);
        self.save_soon();
        if key == "popups" && !value {
            self.toasts_dismiss_app(id);
        }
        if key == "sound" {
            self.apply_audio(id);
        }
        self.last_used.insert(id.to_string(), rt::epoch_ms()); // a changed setting starts the idle clock over
        if *self.asleep.get(id).unwrap_or(&false) && !self.sleep_eligible(id) {
            self.wake_app(id); // it has notifications to deliver again
        }
        log!("app setting {id} {key} {value}");
        self.broadcast_state();
        self.update_glow(false);
        true
    }

    /// An app sleeps (its page is unloaded, which frees its RAM) when the user asked for it, or
    /// when it couldn't alert them anyway: pop-ups and unread count both switched off.
    pub fn sleep_eligible(&self, id: &str) -> bool {
        self.settings.app_pref(id, "sleep") || (!self.settings.app_pref(id, "popups") && !self.settings.app_pref(id, "badge"))
    }

    pub fn sleep_app(&mut self, id: &str) {
        if *self.asleep.get(id).unwrap_or(&false) || !self.chats.has(id) {
            return;
        }
        self.destroy_view(id);
        self.asleep.insert(id.to_string(), true);
        self.toasts_dismiss_app(id);
        log!("app sleeping {id}");
        self.broadcast_state();
        self.update_glow(false);
    }

    pub fn wake_app(&mut self, id: &str) {
        if !*self.asleep.get(id).unwrap_or(&false) || !self.is_enabled(id) {
            return;
        }
        self.asleep.insert(id.to_string(), false);
        self.last_used.insert(id.to_string(), rt::epoch_ms());
        self.create_view(id);
        log!("app awake {id}");
        self.broadcast_state();
    }

    pub fn sleep_check(&mut self) {
        let now = rt::epoch_ms();
        let showing = self.panel_state.showing();
        let active = self.active();
        for id in self.enabled_apps() {
            if *self.asleep.get(id).unwrap_or(&false) || !self.chats.has(id) || !self.sleep_eligible(id) {
                continue;
            }
            if (showing && active == id) || self.chats.playing_audio(id) || self.chats.popups_open(id) > 0 {
                self.last_used.insert(id.to_string(), now); // on screen, playing sound, or in a call
                continue;
            }
            if now - self.last_used.get(id).copied().unwrap_or(now) >= SLEEP_AFTER_MS {
                self.sleep_app(id);
            }
        }
    }

    pub fn apply_audio(&mut self, id: &str) {
        let muted = self.settings.bool("muted") || !self.settings.app_pref(id, "sound");
        self.chats.set_muted(id, muted);
    }

    // -----------------------------------------------------------------------------------------
    // Unread counts (read from the page title, e.g. "(3) Instagram")
    // -----------------------------------------------------------------------------------------
    pub fn on_title(&mut self, id: &str, title: &str) {
        let n = parse_count(title);
        if n.is_none() {
            let text = title.trim();
            if !text.is_empty() && !apps::plain_title(id, text) && message_words(text) {
                self.last_flash.insert(id.to_string(), (text.to_string(), rt::epoch_ms()));
            }
        }
        let n = n.unwrap_or(0);
        if n > 0 {
            if let Some(t) = self.zero_timers.remove(id) {
                rt::cancel(t);
            }
            self.set_count(id, n);
        } else if *self.counts.get(id).unwrap_or(&0) > 0 && !self.zero_timers.contains_key(id) {
            // Titles flash ("Name sent you a message" <-> "(1) Facebook"), so only trust a zero that sticks.
            let app = id.to_string();
            let t = timer(3000, move |c| {
                c.zero_timers.remove(&app);
                c.set_count(&app, 0);
            });
            self.zero_timers.insert(id.to_string(), t);
        }
    }

    pub fn set_count(&mut self, id: &str, n: u32) {
        let before = *self.counts.get(id).unwrap_or(&0);
        if before == n {
            return;
        }
        self.counts.insert(id.to_string(), n);
        log!("unread {id} {before} -> {n}");
        self.broadcast_state();
        let badge = self.settings.app_pref(id, "badge");
        self.update_glow(n > before && badge);
        if n > before {
            self.schedule_count_popup(id);
        } else if n == 0 {
            self.toasts_dismiss_app(id); // read elsewhere (e.g. on the phone)
        }
    }

    // -----------------------------------------------------------------------------------------
    // Message pop-ups: who may pop up when
    // -----------------------------------------------------------------------------------------
    pub fn dnd_active(&self) -> bool {
        let until = self.settings.i64("dndUntil");
        until == -1 || (until > 0 && until > rt::epoch_ms())
    }

    pub fn set_dnd(&mut self, minutes: i64) {
        let until = if minutes == -1 {
            -1
        } else if minutes > 0 {
            rt::epoch_ms() + minutes * 60_000
        } else {
            0
        };
        self.set_setting("dndUntil", json!(until));
        if until != 0 {
            self.toasts_dismiss_all();
        }
        self.schedule_dnd_end();
        self.broadcast_state();
        log!(
            "do not disturb {}",
            if until == -1 {
                "on".into()
            } else if until > 0 {
                format!("for {minutes} min")
            } else {
                "off".into()
            }
        );
    }

    pub fn schedule_dnd_end(&mut self) {
        rt::cancel(self.dnd_timer);
        self.dnd_timer = 0;
        let until = self.settings.i64("dndUntil");
        if until <= 0 {
            return;
        }
        let ms = (until - rt::epoch_ms()).max(0) as u64;
        self.dnd_timer = timer(ms, |c| {
            c.dnd_timer = 0;
            c.set_setting("dndUntil", json!(0));
            c.broadcast_state();
            log!("do not disturb over");
        });
    }

    /// Every "should this pop up?" rule the user can set, except "already reading that chat".
    pub fn popup_allowed(&self, id: Option<&str>) -> bool {
        if !self.settings.bool("popups") || self.dnd_active() {
            return false;
        }
        if let Some(id) = id {
            if !self.settings.app_pref(id, "popups") {
                return false;
            }
        }
        !(self.settings.bool("popupQuietFullscreen") && win32::is_fullscreen_app_active())
    }

    /// Is the user already looking at this app's chat?
    pub fn app_on_screen(&self, id: &str) -> bool {
        self.panel_state == PanelState::Open && !self.settings_mode && self.active() == id && win32::foreground_window() == self.panel.hwnd
    }

    /// A site raised a web notification (WebView2 NotificationReceived): who wrote, what, and their picture.
    pub fn on_site_notification(&mut self, id: &str, key: u64, title: &str, body: &str, icon: &str, tag: &str) {
        self.last_content_at.insert(id.to_string(), rt::epoch_ms());
        log!("site notification {id} {{\"title\":{},\"body\":{}}}", title.chars().count(), body.chars().count());
        if !self.popup_allowed(Some(id)) || self.app_on_screen(id) {
            return;
        }
        let a = apps::get(id).unwrap();
        let show_text = self.settings.bool("popupText") && self.settings.app_pref(id, "preview");
        let title = clean_text(title, 90);
        let fields = toasts::Fields {
            title: if title.is_empty() { a.name.to_string() } else { title },
            body: if show_text { clean_text(body, 300) } else { self.t("toast.sentYou") },
            icon: if self.settings.bool("popupAvatar") { safe_icon(icon) } else { String::new() },
            tag: if tag.is_empty() { String::new() } else { format!("{id}:{}", clean_text(tag, 80)) },
            source: key,
        };
        self.popup(id, fields);
    }

    /// The unread count went up. If the site didn't say who wrote, still pop something up.
    pub fn schedule_count_popup(&mut self, id: &str) {
        if let Some(t) = self.fallback_timers.remove(id) {
            rt::cancel(t);
        }
        let app = id.to_string();
        let t = timer(2500, move |c| {
            c.fallback_timers.remove(&app);
            let count = *c.counts.get(&app).unwrap_or(&0);
            if !c.popup_allowed(Some(&app)) || c.app_on_screen(&app) || count == 0 {
                return;
            }
            let now = rt::epoch_ms();
            if now - c.last_content_at.get(&app).copied().unwrap_or(0) < 8000 {
                return; // already shown with name + text
            }
            if now - c.load_started_at.get(&app).copied().unwrap_or(0) < COUNT_POPUP_GRACE_MS {
                return; // page just (re)loaded: old unread counts are not new messages
            }
            let flash = c.last_flash.get(&app).filter(|(_, at)| now - at < 15_000).map(|(t, _)| clean_text(t, 120)).unwrap_or_default();
            let fields = toasts::Fields {
                title: if flash.is_empty() { c.t("toast.newMessage") } else { flash },
                body: c.tv("toast.unread", &[("n", count.to_string())]),
                icon: String::new(),
                tag: format!("{app}:count"), // one "new messages" card per app, updated in place
                source: 0,
            };
            c.popup(&app, fields);
        });
        self.fallback_timers.insert(id.to_string(), t);
    }

    pub fn test_popup(&mut self) {
        let id = self.active();
        let avatar = format!("data:image/png;base64,{}", toasts::base64(include_bytes!("../../assets/icon.png")));
        let fields = toasts::Fields {
            title: self.t("toast.testTitle"),
            body: if self.settings.bool("popupText") { self.t("toast.testBody") } else { self.t("toast.sentYou") },
            icon: if self.settings.bool("popupAvatar") { avatar } else { String::new() },
            tag: "test".into(),
            source: 0,
        };
        self.popup(&id, fields);
    }

    // -----------------------------------------------------------------------------------------
    // After an update: say so once, and offer what's new
    // -----------------------------------------------------------------------------------------
    /// Did this start come right after an update? Returns the version we came from (or 'older').
    pub fn detect_update(&mut self) -> Option<String> {
        let now = rt::version();
        let pending = self.settings.get("pendingUpdate").clone();
        let last = self.settings.str("lastVersion").to_string();
        self.set_setting("lastVersion", json!(now));
        if !pending.is_null() {
            self.set_setting("pendingUpdate", Value::Null);
        }
        let pending_to = pending.get("to").and_then(Value::as_str).unwrap_or("");
        let from = if pending_to == now {
            pending.get("from").and_then(Value::as_str).map(str::to_string)
        } else if !last.is_empty() && last != now {
            Some(last)
        } else if last.is_empty() && self.settings.bool("onboarded") {
            Some("older".into()) // builds before 1.3 didn't record their version
        } else {
            None
        };
        let from = from?;
        let notes = if pending_to == now { pending.get("notes").and_then(Value::as_str).unwrap_or("").to_string() } else { String::new() };
        self.set_setting("whatsNew", json!({ "version": now, "from": from, "notes": notes, "at": rt::epoch_ms() }));
        log!("updated {from} -> {now}");
        Some(from)
    }

    /// What's new in the version running now, for whoever updated to it: this release's list, and
    /// what changed on the way from the version they came from (the hold-to-open edge from 1.4, the
    /// move to Tauri in 1.5). Our own translated texts; else the release notes.
    pub fn whats_new_state(&self) -> Value {
        let now = rt::version();
        let w = self.settings.get("whatsNew");
        let mine = w.get("version").and_then(Value::as_str) == Some(now.as_str());
        let from = if mine { w.get("from").and_then(Value::as_str).unwrap_or("") } else { "" };
        let before = |v: &str| from == "older" || (!from.is_empty() && version_less(from, v));
        let mut parts: Vec<String> = Vec::new();
        let mut add = |key: &str| {
            let text = self.t(key);
            if text != key && !text.is_empty() {
                parts.push(text);
            }
        };
        if before("1.4.0") {
            add("whatsnew.hold"); // first: it changes how ChatDock opens
        }
        if from == "1.5.0" {
            add("whatsnew.fix151");
        }
        if let Some(n) = i18n::build(&now) {
            add(&format!("whatsnew.{n}"));
        }
        if before("1.5.0") {
            add("whatsnew.1.5");
        }
        let text =
            if parts.is_empty() && mine { w.get("notes").and_then(Value::as_str).unwrap_or("").to_string() } else { parts.join("\n") };
        if text.is_empty() {
            return Value::Null;
        }
        let at = w.get("at").and_then(Value::as_i64).unwrap_or(0);
        json!({
            "title": self.tv("upd.notesFor", &[("version", self.build_name(&now))]),
            "text": text,
            "justUpdated": mine && rt::epoch_ms() - at < 24 * 60 * 60 * 1000,
        })
    }

    pub fn announce_updated(&mut self) {
        let version = self.build_name(&rt::version());
        let news = self.whats_new_state();
        let first_line =
            news.get("text").and_then(Value::as_str).map(|t| t.lines().next().unwrap_or("").trim_start_matches('•').trim().to_string());
        let item = toasts::Item::new_own(
            "chatdock",
            "ChatDock",
            "logo",
            "#22c55e",
            self.t("toast.updated"),
            self.tv("toast.updatedTitle", &[("version", version)]),
            first_line.clone().unwrap_or_else(|| self.t("toast.updatedBody")),
            if first_line.is_some() { self.t("toast.updatedBody") } else { String::new() },
            "chatdock:updated",
            "whatsnew",
        );
        self.toasts_push(item);
    }

    // -----------------------------------------------------------------------------------------
    // RAM figures for the settings screen
    // -----------------------------------------------------------------------------------------
    pub fn memory_stats(&self) -> chats::Memory {
        self.chats.memory()
    }

    pub fn can_autostart(&self) -> bool {
        !cfg!(debug_assertions) && !self.args.profile
    }

    pub fn set_open_at_login(&mut self, on: bool) {
        if !self.can_autostart() {
            return;
        }
        autostart::set(on);
        self.autostart_cache = autostart::get();
        self.broadcast_state();
    }

    pub fn register_hotkey(&mut self) {
        let acc = self.settings.str("hotkey").to_string();
        self.hotkey_ok = tray::register_hotkey(&acc);
        log!(
            "hotkey {acc} {}",
            if self.hotkey_ok {
                "registered"
            } else if acc.is_empty() {
                "off"
            } else {
                "FAILED"
            }
        );
        if !self.hotkey_ok && !acc.is_empty() && !self.args.selftest {
            let body = self.tv("balloon.hotkeyBody", &[("hotkey", self.hotkey_label())]);
            let title = self.t("balloon.hotkeyTitle");
            self.notice(&title, &body);
        }
    }

    pub fn on_hotkey(&mut self) {
        log!("hotkey {}", self.snap());
        if self.panel_state == PanelState::Open && win32::foreground_window() != self.panel.hwnd {
            // pinned panel sitting behind the game -> bring it forward for typing
            self.remember_foreground("hotkey");
            self.focus_panel();
            return;
        }
        self.toggle_panel("hotkey");
    }

    /// WebView2's browser process is gone: every page is dead and can't be brought back from here.
    /// Start ChatDock again (a clean exit first, so the single-instance lock is free).
    pub fn restart_after_crash(&mut self, why: &str) {
        if self.recovering || self.quitting {
            return;
        }
        self.recovering = true;
        if SESSION_ENDING.load(std::sync::atomic::Ordering::SeqCst) || win32::shutting_down() {
            log!("{why} while Windows signs out: quitting");
            self.quit();
            return;
        }
        // The new copy inherits this: a browser that crashes again right away is left alone
        // (the tray still works, Quit and start ChatDock again), instead of restarting forever.
        const KEY: &str = "CHATDOCK_CRASH_RESTART_AT";
        let now = rt::epoch_ms();
        let last = std::env::var(KEY).ok().and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        if now - last < 120_000 {
            log!("{why} again, within 2 minutes of the last restart: not restarting");
            return;
        }
        std::env::set_var(KEY, now.to_string());
        log!("{why}: starting ChatDock again");
        self.settings.flush();
        rt::app().request_restart();
    }

    /// The renderer that draws ChatDock's own pages crashed (they share one, so every page reports
    /// it): load them all again, once, a moment later; slower each time it happens again.
    pub fn reload_ui_pages(&mut self) {
        if self.recovering || self.quitting || self.ui_reload_pending {
            return;
        }
        let now = rt::epoch_ms();
        self.ui_crashes.retain(|t| now - t < 60_000);
        self.ui_crashes.push(now);
        let n = self.ui_crashes.len() as u32;
        if n > 4 {
            self.restart_after_crash("ChatDock's own pages keep crashing");
            return;
        }
        self.ui_reload_pending = true;
        let delay = (300 * 3u64.pow(n - 1)).min(10_000);
        log!("ChatDock's own pages crashed: loading them again in {delay} ms");
        timer(delay, |c| {
            c.ui_reload_pending = false;
            if c.recovering || c.quitting {
                return;
            }
            let mut pages = vec![c.panel.clone(), c.tab.clone(), c.glow.clone(), c.edgewin.clone(), c.toastwin.clone()];
            pages.extend(c.update_win.clone());
            for w in pages {
                c.ready.remove(w.w.label());
                let _ = w.w.reload();
            }
        });
    }

    pub fn quit(&mut self) {
        self.quitting = true;
        self.settings.flush();
        log!("quit");
        rt::app().exit(0);
    }
}

pub const PREF_KEYS: [&str; 23] = [
    "lang",
    "side",
    "edgeMode",
    "edgeHold",
    "displayId",
    "glow",
    "theme",
    "opacity",
    "pinned",
    "muted",
    "hotkey",
    "autostart",
    "hideFromCapture",
    "popups",
    "popupText",
    "popupAvatar",
    "popupSound",
    "popupPosition",
    "popupDuration",
    "popupMax",
    "popupQuietFullscreen",
    "updateAutoCheck",
    "updateAutoDownload",
];

/// "1.3.0" < "1.4.0" (numbers compared part by part; anything unreadable counts as 0)
pub fn version_less(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> { v.split(['.', '-', '+']).take(3).map(|p| p.parse().unwrap_or(0)).collect() };
    parts(a) < parts(b)
}

/// "(3) Instagram" -> Some(3); no count in front -> None
pub fn parse_count(title: &str) -> Option<u32> {
    let t = title.trim_start();
    let rest = t.strip_prefix('(')?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let after = &rest[digits.len()..];
    let after = after.strip_prefix('+').unwrap_or(after);
    if !after.starts_with(')') {
        return None;
    }
    digits.parse().ok()
}

/// Titles like "Somchai sent you a message" / "สมชาย ส่งข้อความถึงคุณ" (not the site's normal title)
pub fn message_words(title: &str) -> bool {
    let t = title.to_lowercase();
    ["messag", "sent", "wrote", "replied", "mention", "ส่ง", "ข้อความ", "ทัก", "ตอบกลับ", "กล่าวถึง"].iter().any(|w| t.contains(w))
}

pub fn safe_icon(url: &str) -> String {
    if url.len() > 200_000 {
        return String::new();
    }
    let lower = url.to_ascii_lowercase();
    let data_ok = ["data:image/png;", "data:image/jpeg;", "data:image/jpg;", "data:image/gif;", "data:image/webp;"]
        .iter()
        .any(|p| lower.starts_with(p));
    if lower.starts_with("https://") || data_ok {
        url.to_string()
    } else {
        String::new()
    }
}
