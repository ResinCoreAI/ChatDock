//! The app state and the rules that decide what happens. It lives on the main thread only;
//! everything else reaches it through later() / timer() (see rt.rs).
//!
//!   hidden --(cursor held on the dock edge)--> white tab --(click)--> panel slides in
//!   hidden --(global hotkey / tray click)-------------------------> panel slides in
//!   panel  --(click elsewhere / hotkey / Esc Esc / hide button)----> slides out, focus back to the game

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use serde_json::{json, Map, Value};
use tauri::{Emitter, WebviewWindow};

use crate::{apps, autostart, chats, edge, i18n, identify, log, rt, settings::Settings, toasts, tray, updater, win32};

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
/// How often an open call window is looked at (still open? sharing the screen?).
const CALL_WATCH_MS: u64 = 1000;
const COUNT_POPUP_GRACE_MS: i64 = 20_000;
pub const AUTO_RETRY_MS: u64 = 15_000;

// ---------------------------------------------------------------------------------------------
// Command line
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Debug)]
pub struct Args {
    pub data_dir: PathBuf,
    pub profile: bool, // --profile=<dir>: a separate data folder (tests); never touches "start with Windows" (see test_folder)
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
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        let own_dirs = ["ChatDock", "ChatDock-dev", TEST_PRODUCT].map(|n| base.join(n));
        let profile = test_folder(value("profile").map(PathBuf::from), &own_dirs);
        let selftest = has("selftest");
        let data_dir = profile.clone().unwrap_or_else(|| {
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

    /// Developer tools, browser keys and context menus in the pages: development builds only. In a
    /// released ChatDock --debug (or --selftest) only echoes the log, so no switch puts DevTools on
    /// the logged-in chats.
    pub fn dev_tools(&self) -> bool {
        self.debug && cfg!(debug_assertions)
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

/// --profile=<dir> as a test folder of its own. ChatDock's own data folders (and an empty value)
/// don't count: that is the real app, which keeps its one-instance lock, and never a place for the
/// self-test (it switches settings around and logs out of apps).
fn test_folder(profile: Option<PathBuf>, own_dirs: &[PathBuf]) -> Option<PathBuf> {
    profile.filter(|p| !p.as_os_str().is_empty() && !own_dirs.iter().any(|d| same_folder(p, d)))
}

/// The same folder, however it is written ("C:/x/", "c:\X", a relative path).
fn same_folder(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        let full = std::fs::canonicalize(p).or_else(|_| std::path::absolute(p)).unwrap_or_else(|_| p.to_path_buf());
        let s = full.to_string_lossy().replace('/', "\\").to_lowercase();
        s.strip_prefix(r"\\?\").unwrap_or(&s).trim_end_matches('\\').to_string()
    };
    norm(a) == norm(b)
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
    /// "What's new" after an update
    pub whatsnew: updater::WhatsNew,
    /// "Show numbers on the screens" (Settings → Monitors)
    pub identify: identify::Identify,
    pub ready: HashSet<String>,
    pub own_hwnds: Vec<isize>,
    /// what each page's volume script said when asked (self-test)
    pub volume_states: HashMap<String, String>,
    /// the chat's hotkey another program holds, already said
    pub hotkey_warned: String,
    /// Call windows the chats opened (Messenger, Instagram): (window, app)
    pub call_windows: Vec<(isize, String)>,
    /// Looking for the call window a chat just asked for: the app, the windows there were, until when
    pub call_window_search: Option<(String, Vec<isize>, i64)>,
    /// Apps sharing the screen from a call window (the "… is sharing your screen" bar says so)
    pub bar_shares: HashSet<String>,
    /// self-test: whose windows count as the chats' browser's
    pub test_browser_pid: Option<u32>,
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
    /// unread counts ChatDock shows: what the site counts minus what the user has already seen
    pub counts: HashMap<String, u32>,
    /// what each site counts right now (its page title, e.g. "(3) Instagram")
    pub site_counts: HashMap<String, u32>,
    /// apps whose current page has shown a number in its title. Until then what the site counts
    /// isn't known (a page that is still loading shows none), so "seen" isn't checked against it.
    pub counted_pages: HashSet<String>,
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
    /// A window of the chats' browser has the focus (the "… is sharing your screen" bar): watching
    /// where the focus goes next, since a click elsewhere from there never reaches the panel.
    pub focus_watch: bool,
    /// self-test: a pretend foreground window
    pub test_foreground: Option<isize>,
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
    /// the global hotkeys registered right now: (shortcut id, what it does)
    pub hotkeys: Vec<(u32, &'static str)>,
    pub autostart_cache: bool,
    pub dnd_timer: u64,
    pub save_timer: u64,
    pub quitting: bool,
}

thread_local! {
    static CORE: RefCell<Option<Core>> = const { RefCell::new(None) };
}

/// Windows is signing out or shutting down (set from the panel's window messages, and cleared again
/// when that is cancelled).
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
        hwnd != 0 && (self.is_own_window(hwnd) || self.is_chat_browser_window(hwnd))
    }

    /// ChatDock's own windows and the dialogs they own (a file picker, a menu).
    pub fn is_own_window(&self, hwnd: isize) -> bool {
        self.own_hwnds.contains(&hwnd) || self.own_hwnds.contains(&win32::root_owner(hwnd))
    }

    /// The chats' own WebView2 windows, e.g. the "… is sharing your screen" bar that takes the focus
    /// when a screen share starts: the chat must not hide for it.
    pub fn is_chat_browser_window(&self, hwnd: isize) -> bool {
        chats::browser_pid() != 0 && win32::window_pid(hwnd) == chats::browser_pid()
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
                    "whatsnew" => self.whats_new_window_ready(),
                    "toasts" => self.toasts_ready(),
                    label if label.starts_with("ident-") => self.identify_ready(label),
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
            ("panel:volume", "panel") => {
                let id = arg_str(&args, 0);
                if let Some(level) = args.get(1).and_then(Value::as_f64) {
                    self.set_app_volume(&id, level);
                }
            }
            ("panel:sounds-on", "panel") => {
                self.set_pref("muted", json!(false));
            }
            ("panel:sound", "panel") => {
                let id = arg_str(&args, 0);
                let on = args.get(1).and_then(Value::as_bool).unwrap_or(true);
                if apps::get(&id).is_some() {
                    self.set_app_pref(&id, "sound", on);
                }
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
                self.on_tab_open(&id, arg_str(&args, 1) == "call");
            }
            (c, "toasts") if c.starts_with("toast:") => self.on_toast_message(c, &args),
            ("whatsnew:size", "whatsnew") => {
                if let Some(h) = args.first().and_then(Value::as_f64) {
                    self.whats_new_size(h);
                }
            }
            ("whatsnew:close", "whatsnew") => self.close_whats_new(false),
            ("whatsnew:releases", "whatsnew") => self.close_whats_new(true),
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
            // the panel's window is on screen: its page keeps still while it isn't (panel.css)
            "shown": win32::is_visible(self.panel.hwnd),
            "calls": self.call_chips().iter().map(|(app, kind)| json!({ "app": app, "kind": kind })).collect::<Vec<_>>(),
            // each app's volume in ChatDock, and whether its sound is on (the header's speaker)
            "volumes": apps::ids().map(|id| (id.to_string(), json!(self.app_volume(id)))).collect::<Map<String, Value>>(),
            "soundOn": apps::ids().map(|id| (id.to_string(), json!(self.settings.app_pref(id, "sound")))).collect::<Map<String, Value>>(),
            "allMuted": self.settings.bool("muted"),
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

    /// The monitors for the map and the list in Settings, how the chat picks one ("auto", "one", or
    /// "missing": the picked one isn't connected), and a warning when the picked one's dock-side edge
    /// touches another monitor all the way (the mouse can't stop there).
    fn monitors_state(&self) -> (Vec<Value>, &'static str, Value) {
        let numbered = self.numbered_displays();
        let chat = self.target_display().id;
        let monitors = numbered
            .iter()
            .map(|(n, d)| {
                let ranges = |left: bool| self.outer_ranges(d, left).iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>();
                json!({
                    "key": d.key, "n": n, "label": self.monitor_label(d),
                    "x": d.bounds.x, "y": d.bounds.y, "w": d.bounds.w, "h": d.bounds.h, "hz": d.hz,
                    "primary": d.primary, "chat": d.id == chat,
                    "edges": { "left": ranges(true), "right": ranges(false) },
                })
            })
            .collect();
        let chosen = self.chosen_display();
        let picked = self.settings.get("displayId").as_str().is_some_and(|s| !s.is_empty());
        let mode = if chosen.is_some() {
            "one"
        } else if picked {
            "missing"
        } else {
            "auto"
        };
        let left = self.on_left();
        let seam = chosen
            .filter(|d| self.outer_share(d, left) < 0.1) // a sliver doesn't count
            .map(|d| {
                let number = |id: &str| numbered.iter().find(|(_, o)| o.id == id).map(|(n, _)| *n).unwrap_or(0);
                let beside = numbered.iter().find(|(_, o)| {
                    o.id != d.id
                        && (if left { o.bounds.right() == d.bounds.x } else { o.bounds.x == d.bounds.right() })
                        && o.bounds.y < d.bounds.bottom()
                        && o.bounds.bottom() > d.bounds.y
                });
                json!({
                    "n": number(&d.id),
                    "m": beside.map(|(n, _)| *n).unwrap_or(0),
                    "canSwitch": self.outer_share(&d, !left) >= 0.25,
                })
            })
            .unwrap_or(Value::Null);
        (monitors, mode, seam)
    }

    pub fn settings_state(&self) -> Value {
        let (monitors, mode, seam) = self.monitors_state();
        let mut prefs = Map::new();
        for key in PREF_KEYS {
            if key != "autostart" && key != "displayId" {
                prefs.insert(key.to_string(), self.settings.get(key).clone());
            }
        }
        prefs.insert("autostart".into(), json!(self.autostart_cache));
        // the picked monitor's key as the page knows it (a 1.5.x name that is still being matched
        // shows as that monitor), else what is saved, else "auto"
        let picked = self
            .chosen_display()
            .map(|d| d.key)
            .or_else(|| self.settings.get("displayId").as_str().filter(|s| !s.is_empty()).map(str::to_string))
            .unwrap_or_else(|| "auto".to_string());
        prefs.insert("displayId".into(), json!(picked));
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
        let version = rt::version();
        json!({
            "prefs": prefs,
            "catalog": catalog,
            "memory": memory.total,
            "langs": i18n::LANGS.iter().map(|(id, name)| json!({ "id": id, "name": name })).collect::<Vec<_>>(),
            "monitors": monitors,
            "monitorMode": mode,
            "chosenLabel": self.settings.str("displayLabel"),
            "seam": seam,
            "hotkeys": HOTKEYS.iter().map(|(acc, label)| json!({ "acc": acc, "label": label.replace("Ctrl", &self.t("key.ctrl")) })).collect::<Vec<_>>(),
            "hotkeyOk": self.hotkey_ok,
            "voiceKeys": VOICE_KEYS.iter().map(|(acc, label)| json!({ "acc": acc, "label": label.replace("Ctrl", &self.t("key.ctrl")) })).collect::<Vec<_>>(),
            "discord": self.discord_state(),
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
            "displayId" => v.as_str().is_some_and(|s| s == "auto" || win32::displays().iter().any(|d| d.key == s || d.id == s)),
            "popupDisplay" => one_of(&["chat", "mouse", "main"]),
            // a voice key from the list, not one of the other hotkeys
            "discordMuteKey" | "discordDeafenKey" => v.as_str().is_some_and(|s| {
                let other = if key == "discordMuteKey" { "discordDeafenKey" } else { "discordMuteKey" };
                s.is_empty()
                    || (VOICE_KEYS.iter().any(|(a, _)| *a == s) && s != self.settings.str("hotkey") && s != self.settings.str(other))
            }),
            "theme" => one_of(&["system", "dark", "light"]),
            "opacity" => v.as_f64().is_some_and(|n| (0.6..=1.0).contains(&n)),
            "hotkey" => v.as_str().is_some_and(|s| s.is_empty() || HOTKEYS.iter().any(|(a, _)| *a == s)),
            "popupPosition" => one_of(&["top-right", "bottom-right", "top-left", "bottom-left"]),
            "popupDuration" => num_in(&[0.0, 5.0, 8.0, 12.0, 20.0, 30.0]),
            "popupMax" => num_in(&[1.0, 2.0, 3.0, 4.0, 5.0]),
            "glow"
            | "edgeWheel"
            | "discordDms"
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
            self.broadcast_state(); // the page shows what it really is again
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
                let id = value.as_str().unwrap_or("");
                match win32::displays().into_iter().find(|d| id != "auto" && (d.key == id || d.id == id)) {
                    Some(d) => {
                        log!("chat monitor: {} ({})", d.id, self.monitor_label(&d));
                        self.set_setting("displayId", json!(d.key));
                        self.set_setting("displayLabel", json!(self.monitor_label(&d)));
                    }
                    None => {
                        log!("chat monitor: automatic");
                        self.set_setting("displayId", Value::Null);
                    }
                }
                self.dock_display.clear();
                self.on_displays_changed();
            }
            "popupDisplay" => {
                self.set_setting(key, value);
                self.toasts.display.clear();
                self.toasts_refresh();
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
            "hotkey" | "discordMuteKey" | "discordDeafenKey" => {
                self.set_setting(key, value);
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

    /// The self-test's way to a settings action.
    pub fn settings_action_test(&mut self, name: &str, arg: Value) {
        self.settings_action(name, arg);
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
                self.forget_electron_logins();
            }
            "reset-widths" => {
                self.set_setting("widths", json!({}));
                self.fit_panel_to_app();
            }
            "discord-server" => {
                // {name, on}: that server's pop-ups on this PC
                let name = arg.get("name").and_then(Value::as_str).unwrap_or("");
                if let Some(on) = arg.get("on").and_then(Value::as_bool).filter(|_| self.settings.get("discordServers").get(name).is_some())
                {
                    self.settings.set_in("discordServers", name, json!(on));
                    self.save_soon();
                    self.broadcast_state();
                }
            }
            "identify" => self.identify_all(),
            "identify-hover" => {
                let key = arg.as_str().unwrap_or("").chars().take(300).collect::<String>();
                self.identify_hover(&key);
            }
            "check-update" => self.check_update(),
            "download-update" => self.download_update(),
            "install-update" => self.install_update(),
            "open-releases" => win32::open_url(&format!("{REPO_URL}/releases")),
            "whats-new" => self.open_whats_new(),
            "discord-awake" => {
                self.set_app_pref("discord", "sleep", false);
                self.broadcast_state();
            }
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
        if id == "discord" {
            self.register_hotkey(); // its voice keys come and go with it
        }
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
            if (showing && active == id) || self.chats.playing_audio(id) || self.chats.in_call(id) {
                self.last_used.insert(id.to_string(), now); // on screen, playing sound, or in a call
                continue;
            }
            if now - self.last_used.get(id).copied().unwrap_or(now) >= SLEEP_AFTER_MS {
                self.sleep_app(id);
            }
        }
    }

    /// A call (a voice channel, a screen share) started or ended in an app's page. During one the
    /// app stays awake and keeps its memory, even when nobody talks. The edge tab shows a phone
    /// while it's in a call and a screen while it shares the screen.
    pub fn on_call(&mut self, id: &str, live: bool, call: bool, share: bool) {
        if self.chats.page_call_live(id) != live {
            self.chats.set_in_call(id, live);
            self.last_used.insert(id.to_string(), rt::epoch_ms());
            log!("call {id} {}", if live { "started" } else { "ended" });
        }
        if self.chats.call_state(id) != (call, share) {
            self.chats.set_call_state(id, call, share);
            log!("call {id}: in a call {call}, sharing the screen {share}");
            self.broadcast_state();
            self.refit_tab(); // taller or shorter
        }
    }

    fn calls_browser_pid(&self) -> u32 {
        self.test_browser_pid.unwrap_or_else(chats::browser_pid)
    }

    /// A chat opened a call window: Messenger and Instagram calls run in a window of their own,
    /// which WebView2 makes by itself (the page script doesn't run there). It is found as a new
    /// window of the chats' browser; while it's open the app is in a call.
    pub fn call_window_opening(&mut self, app: &str) {
        let before = win32::top_windows_of(self.calls_browser_pid());
        self.call_window_search = Some((app.to_string(), before, rt::epoch_ms() + 8000));
        timer(150, |c| c.find_call_window());
    }

    fn find_call_window(&mut self) {
        let Some((app, before, until)) = self.call_window_search.clone() else { return };
        let found = win32::top_windows_of(self.calls_browser_pid()).into_iter().find(|&h| {
            let r = win32::window_rect(h);
            !before.contains(&h) && !self.call_windows.iter().any(|(w, _)| *w == h) && win32::is_visible(h) && r.w >= 200 && r.h >= 150
        });
        match found {
            Some(h) => {
                self.call_window_search = None;
                let watching = !self.call_windows.is_empty();
                self.call_windows.push((h, app.clone()));
                log!("call window {app} open");
                self.call_windows_changed(&app);
                if !watching {
                    timer(CALL_WATCH_MS, |c| c.watch_call_windows());
                }
            }
            None if rt::epoch_ms() > until => {
                self.call_window_search = None;
                log!("call window {app}: none showed up");
            }
            None => {
                timer(150, |c| c.find_call_window());
            }
        }
    }

    /// Every second while a call window is open: closed, that call ended. And a small window of the
    /// chats' browser whose title starts with the site ("www.messenger.com is sharing your
    /// screen.") says the screen is shared from it.
    fn watch_call_windows(&mut self) {
        let closed: Vec<(isize, String)> =
            self.call_windows.iter().filter(|(h, _)| !win32::is_window(*h) || !win32::is_visible(*h)).cloned().collect();
        self.call_windows.retain(|w| !closed.contains(w));
        let mut changed: Vec<String> = closed.into_iter().map(|(_, app)| app).collect();
        for app in &changed {
            log!("call window {app} closed");
        }
        let shares = self.share_bars();
        if shares != self.bar_shares {
            log!("screen shared from a call window: {:?}", shares.iter().collect::<Vec<_>>());
            changed.extend(self.bar_shares.symmetric_difference(&shares).cloned());
            self.bar_shares = shares;
        }
        changed.sort();
        changed.dedup();
        for app in changed {
            self.call_windows_changed(&app);
        }
        if !self.call_windows.is_empty() {
            timer(CALL_WATCH_MS, |c| c.watch_call_windows());
        }
    }

    /// The apps with a call window whose site shares the screen right now.
    fn share_bars(&self) -> HashSet<String> {
        let mut sharing = HashSet::new();
        if self.call_windows.is_empty() {
            return sharing;
        }
        for h in win32::top_windows_of(self.calls_browser_pid()) {
            if !win32::is_visible(h) || win32::window_rect(h).h >= 150 || self.call_windows.iter().any(|(w, _)| *w == h) {
                continue;
            }
            let title = win32::window_title(h);
            let host = title.split_whitespace().next().unwrap_or("");
            if !host.contains('.') {
                continue;
            }
            for (_, app) in &self.call_windows {
                if apps::owns(app, &format!("https://{host}/")) {
                    sharing.insert(app.clone());
                }
            }
        }
        sharing
    }

    fn call_windows_changed(&mut self, app: &str) {
        let open = self.call_windows.iter().any(|(_, a)| a == app);
        self.chats.set_popup_call(app, open); // stays awake and keeps its memory, like any call
        self.last_used.insert(app.to_string(), rt::epoch_ms());
        self.broadcast_state();
        self.refit_tab(); // taller or shorter
    }

    /// The icons on the edge tab under the apps: (app, "call" | "share"), in the apps' order.
    pub fn call_chips(&self) -> Vec<(&'static str, &'static str)> {
        let mut chips = Vec::new();
        for id in self.enabled_apps() {
            let (page_call, page_share) = self.chats.call_state(id);
            let call = page_call || self.call_windows.iter().any(|(_, a)| a == id);
            let share = page_share || self.bar_shares.contains(id);
            if call {
                chips.push((id, "call"));
            }
            if share {
                chips.push((id, "share"));
            }
        }
        chips
    }

    /// This app's volume in ChatDock, 0-100 (on top of the site's own).
    pub fn app_volume(&self, id: &str) -> u8 {
        self.settings.get("volumes").get(id).and_then(Value::as_u64).map(|v| v.min(100) as u8).unwrap_or(100)
    }

    /// The speaker in the chat's header: how loud this app plays, without touching the others.
    pub fn set_app_volume(&mut self, id: &str, level: f64) {
        if apps::get(id).is_none() || !level.is_finite() {
            return;
        }
        let level = level.round().clamp(0.0, 100.0) as u8;
        if level == self.app_volume(id) {
            return;
        }
        let mut all = self.settings.get("volumes").as_object().cloned().unwrap_or_default();
        all.insert(id.to_string(), json!(level));
        self.set_setting("volumes", Value::Object(all));
        self.send_volume(id);
        self.broadcast_state();
    }

    /// Tell the app's page its volume (its script scales everything it plays).
    pub fn send_volume(&self, id: &str) {
        self.chats.post_json(id, &json!({ "type": "chatdock-volume", "level": self.app_volume(id) as f64 / 100.0 }));
    }

    pub fn apply_audio(&mut self, id: &str) {
        // "all chat sounds off" is about the chats: music plays on
        let muted = (self.settings.bool("muted") && id != "spotify") || !self.settings.app_pref(id, "sound");
        self.chats.set_muted(id, muted);
    }

    // -----------------------------------------------------------------------------------------
    // Unread counts (read from the page title, e.g. "(3) Instagram")
    // -----------------------------------------------------------------------------------------
    pub fn on_title(&mut self, id: &str, title: &str) {
        if id == "spotify" {
            return; // the song playing: no unread numbers there
        }
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
            self.set_site_count(id, n);
        } else if *self.site_counts.get(id).unwrap_or(&0) > 0 && !self.zero_timers.contains_key(id) {
            // Titles flash ("Name sent you a message" <-> "(1) Facebook"), so only trust a zero that sticks.
            let app = id.to_string();
            let t = timer(3000, move |c| {
                c.zero_timers.remove(&app);
                c.set_site_count(&app, 0);
            });
            self.zero_timers.insert(id.to_string(), t);
        }
    }

    /// The user can see this app's chat right now: the panel is out on it (not on Settings or the
    /// welcome screen).
    pub fn chat_in_view(&self, id: &str) -> bool {
        self.panel_state.showing() && !self.settings_mode && !self.help_mode && self.active() == id
    }

    fn seen_count(&self, id: &str) -> u32 {
        self.settings.get("seenCounts").get(id).and_then(Value::as_u64).unwrap_or(0) as u32
    }

    fn set_site_count(&mut self, id: &str, n: u32) {
        self.site_counts.insert(id.to_string(), n);
        self.refresh_count(id);
        if n > 0 {
            self.counted_pages.insert(id.to_string()); // (after: its first number is checked against "seen")
        }
    }

    /// Unread = what the site counts minus what the user has already seen. Whatever it counts while
    /// its chat is on screen has been seen (a message that arrives in the conversation you're reading,
    /// or the likes and follows a site adds to the same number), so it never pops up or shows as a
    /// number once you look away. Remembered across restarts (seen_after: a page ChatDock wasn't
    /// watching).
    pub fn refresh_count(&mut self, id: &str) {
        let site = *self.site_counts.get(id).unwrap_or(&0);
        let seen = self.seen_count(id);
        let seen_now = seen_after(seen, site, self.chat_in_view(id), self.counted_pages.contains(id));
        if seen_now != seen {
            self.settings.set_in("seenCounts", id, json!(seen_now));
            self.save_soon();
        }
        self.set_count(id, site.saturating_sub(seen_now));
    }

    /// The chat on screen changed (the panel came out, another app, back from Settings): what its
    /// site counts now has been seen.
    pub fn counts_seen_in_view(&mut self) {
        let active = self.active();
        if self.chat_in_view(&active) {
            self.refresh_count(&active);
        }
    }

    pub fn set_count(&mut self, id: &str, n: u32) {
        let before = *self.counts.get(id).unwrap_or(&0);
        if before == n {
            return;
        }
        self.counts.insert(id.to_string(), n);
        log!("unread {id} {before} -> {n} (the site counts {})", self.site_counts.get(id).copied().unwrap_or(0));
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
        // Discord says where a message is: "Name (#channel, Server)", or just "Name" for a direct
        // message. The pop-up shows the server and channel; a server's pop-ups can be switched off
        // here without touching Discord's own settings (those are the same on the phone). Read
        // before the title is shortened: a long server name would lose its ")".
        let place = if id == "discord" { discord_place(&clean_text(title, 400)) } else { None };
        let mut title = clean_text(title, 90);
        let mut meta = String::new();
        log!(
            "site notification {id} {{\"title\":{},\"body\":{}{}}}",
            title.chars().count(),
            body.chars().count(),
            if id != "discord" {
                ""
            } else if place.is_some() {
                ",\"from\":\"server\""
            } else {
                ",\"from\":\"direct\""
            }
        );
        if id == "discord" {
            match &place {
                Some((_, _, server)) => {
                    self.note_discord_server(server);
                    if !self.discord_server_on(server) {
                        return;
                    }
                }
                None if !self.settings.bool("discordDms") => return,
                None => {}
            }
        }
        if let Some((who, channel, server)) = place {
            title = clean_text(&who, 90);
            meta = clean_text(&format!("{server} · {channel}"), 90);
        }
        if !self.popup_allowed(Some(id)) || self.app_on_screen(id) {
            return;
        }
        let a = apps::get(id).unwrap();
        let show_text = self.settings.bool("popupText") && self.settings.app_pref(id, "preview");
        let fields = toasts::Fields {
            title: if title.is_empty() { a.name.to_string() } else { title },
            body: if show_text { clean_text(body, 300) } else { self.t("toast.sentYou") },
            icon: if self.settings.bool("popupAvatar") { safe_icon(icon) } else { String::new() },
            tag: if tag.is_empty() { String::new() } else { format!("{id}:{}", clean_text(tag, 80)) },
            source: key,
            meta,
            hint: String::new(),
        };
        self.popup(id, fields);
    }

    /// Settings' Discord page: is it on, the servers seen in its notifications (A-Z), and whether
    /// the voice keys work (another program can hold a key).
    fn discord_state(&self) -> Value {
        let mut servers: Vec<(String, bool)> = self
            .settings
            .get("discordServers")
            .as_object()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_bool().unwrap_or(true))).collect())
            .unwrap_or_default();
        servers.sort_by_key(|(name, _)| name.to_lowercase());
        let registered = |action: &str, key: &str| self.settings.str(key).is_empty() || self.hotkeys.iter().any(|(_, a)| *a == action);
        json!({
            "on": self.is_enabled("discord"),
            "servers": servers.iter().map(|(name, on)| json!({ "name": name, "on": on })).collect::<Vec<_>>(),
            "muteOk": registered("discord-mute", "discordMuteKey"),
            "deafenOk": registered("discord-deafen", "discordDeafenKey"),
            // asleep it sends nothing: the page offers to keep it awake
            "sleeps": self.settings.app_pref("discord", "sleep"),
        })
    }

    /// The servers in Discord's own sidebar: each gets its switch in Settings right away, not only
    /// after its first pop-up. (Servers inside a closed folder aren't drawn there; they come with
    /// their first pop-up.)
    pub fn read_discord_servers(&mut self) {
        if !self.chats.has("discord") {
            return;
        }
        let js = "JSON.stringify([...document.querySelectorAll('[data-list-item-id^=\"guildsnav___\"]')]\
            .filter((e) => /^guildsnav___\\d+$/.test(e.getAttribute('data-list-item-id')))\
            .map((e) => { const n = e.closest('[data-dnd-name]') || e.querySelector('[data-dnd-name]'); return (n && n.getAttribute('data-dnd-name')) || ''; })\
            .filter(Boolean))";
        self.chats.execute("discord", js, |r| {
            let names: Vec<String> =
                serde_json::from_str::<String>(&r).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
            later(move |c| {
                let before = c.settings.get("discordServers").as_object().map(|m| m.len()).unwrap_or(0);
                for name in &names {
                    c.note_discord_server(&clean_text(name, 100));
                }
                let after = c.settings.get("discordServers").as_object().map(|m| m.len()).unwrap_or(0);
                if after != before {
                    log!("discord servers from its sidebar: {} seen, {} new", names.len(), after - before);
                }
            });
        });
    }

    /// A Discord server seen in a notification: listed in Settings (pop-ups on at first).
    fn note_discord_server(&mut self, server: &str) {
        if server.is_empty() || self.settings.get("discordServers").get(server).is_some() {
            return;
        }
        if self.settings.get("discordServers").as_object().is_some_and(|m| m.len() >= 200) {
            return; // enough to choose from
        }
        self.settings.set_in("discordServers", server, json!(true));
        self.save_soon();
        if self.settings_mode {
            self.broadcast_state();
        }
    }

    fn discord_server_on(&self, server: &str) -> bool {
        self.settings.get("discordServers").get(server).and_then(Value::as_bool).unwrap_or(true)
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
            // Discord says who wrote only with its own desktop notifications on (a setting of each
            // browser, ChatDock's too): none from it yet, so the card says where to turn them on
            let hint = if app == "discord" && c.last_content_at.get(&app).copied().unwrap_or(0) == 0 {
                c.t("toast.discordWho")
            } else {
                String::new()
            };
            let fields = toasts::Fields {
                title: if flash.is_empty() { c.t("toast.newMessage") } else { flash },
                body: c.tv("toast.unread", &[("n", count.to_string())]),
                icon: String::new(),
                tag: format!("{app}:count"), // one "new messages" card per app, updated in place
                source: 0,
                meta: String::new(),
                hint,
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
            meta: String::new(),
            hint: String::new(),
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

    /// The version the user updated from ("older" = before 1.3, which didn't record it), if this
    /// start came right after an update to the version running now.
    pub fn updated_from(&self) -> Option<String> {
        let w = self.settings.get("whatsNew");
        if w.get("version").and_then(Value::as_str) != Some(rt::version().as_str()) {
            return None;
        }
        w.get("from").and_then(Value::as_str).filter(|f| !f.is_empty()).map(str::to_string)
    }

    /// What changed for whoever updated to the version running now: one list per release since the
    /// version they came from, newest first (after a fresh install, this release's). Our own
    /// translated texts ("whatsnew.<build>" in ui/i18n-data.js), else the release notes that came
    /// with the update. The hold-to-open edge (1.4) goes on top: it changes how ChatDock opens.
    pub fn whats_new_sections(&self) -> Vec<(String, Vec<String>)> {
        let now = rt::version();
        let from = self.updated_from();
        let wanted = |v: &str| match from.as_deref() {
            None => version_parts(v) == version_parts(&now),
            // 1.5.1 only fixed what 1.5.0 broke
            Some(f) => (f == "older" || version_less(f, v)) && !version_less(&now, v) && (v != "1.5.1" || f == "1.5.0"),
        };
        let mut found: Vec<(String, String)> = i18n::news_keys().into_iter().filter(|(v, _)| wanted(v)).collect();
        found.sort_by_key(|(v, _)| (v != "1.4.0", std::cmp::Reverse(version_parts(v))));
        let mut sections: Vec<(String, Vec<String>)> = found
            .into_iter()
            .map(|(v, key)| {
                let text = self.t(&key);
                (v, text.lines().map(|l| l.trim().trim_start_matches('•').trim().to_string()).filter(|l| !l.is_empty()).collect())
            })
            .filter(|(_, lines): &(String, Vec<String>)| !lines.is_empty())
            .collect();
        if sections.is_empty() && from.is_some() {
            let notes = note_lines(self.settings.get("whatsNew").get("notes").and_then(Value::as_str).unwrap_or(""));
            if !notes.is_empty() {
                sections.push((now, notes));
            }
        }
        sections
    }

    /// What's new for the settings screen (Updates): the same lists as one.
    pub fn whats_new_state(&self) -> Value {
        let sections = self.whats_new_sections();
        if sections.is_empty() {
            return Value::Null;
        }
        let now = rt::version();
        let text = sections.iter().flat_map(|(_, lines)| lines).map(|l| format!("• {l}")).collect::<Vec<_>>().join("\n");
        let at = self.settings.get("whatsNew").get("at").and_then(Value::as_i64).unwrap_or(0);
        json!({
            "title": self.tv("upd.notesFor", &[("version", self.build_name(&now))]),
            "text": text,
            "justUpdated": self.updated_from().is_some() && rt::epoch_ms() - at < 24 * 60 * 60 * 1000,
        })
    }

    /// Right after an update (which the user started a moment ago): "ChatDock is now on Beta Build
    /// 1.x ✓" and what's new since the version they had, in a window in the middle of the screen. It
    /// never takes the keyboard, and waits while a game runs in exclusive fullscreen or a
    /// presentation is on. Only a pop-up when there is nothing to list.
    pub fn announce_updated(&mut self) {
        if !self.whats_new_sections().is_empty() {
            if win32::nothing_may_show() {
                timer(5000, |c| c.announce_updated());
            } else {
                self.show_whats_new_window();
            }
            return;
        }
        let version = self.build_name(&rt::version());
        let item = toasts::Item::new_own(
            "chatdock",
            "ChatDock",
            "logo",
            "#22c55e",
            self.t("toast.updated"),
            self.tv("toast.updatedTitle", &[("version", version)]),
            self.t("toast.updatedBody"),
            String::new(),
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
        let discord = self.is_enabled("discord");
        let keys = [
            ("panel", acc.clone()),
            ("discord-mute", if discord { self.settings.str("discordMuteKey").to_string() } else { String::new() }),
            ("discord-deafen", if discord { self.settings.str("discordDeafenKey").to_string() } else { String::new() }),
        ];
        self.hotkeys = tray::register_hotkeys(&keys);
        self.hotkey_ok = self.hotkeys.iter().any(|(_, a)| *a == "panel");
        log!("hotkeys {:?}", self.hotkeys.iter().map(|(_, a)| *a).collect::<Vec<_>>());
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
        // said once per key: the Discord keys register again with it on their changes
        if self.hotkey_ok || acc.is_empty() {
            self.hotkey_warned.clear();
        } else if !self.args.selftest && self.hotkey_warned != acc {
            self.hotkey_warned = acc.clone();
            let body = self.tv("balloon.hotkeyBody", &[("hotkey", self.hotkey_label())]);
            let title = self.t("balloon.hotkeyTitle");
            self.notice(&title, &body);
        }
    }

    /// A global hotkey was pressed (its shortcut id).
    pub fn on_shortcut(&mut self, id: u32) {
        match self.hotkeys.iter().find(|(k, _)| *k == id).map(|(_, a)| *a) {
            Some("panel") => self.on_hotkey(),
            Some("discord-mute") => self.discord_voice(0),
            Some("discord-deafen") => self.discord_voice(1),
            _ => {}
        }
    }

    /// Mute / deafen in Discord from anywhere, a game included, like the Discord app's own keys:
    /// presses Discord's own switch in its page (0 = the mic, 1 = the sound), then says how it is
    /// now in a short pop-up.
    pub fn discord_voice(&mut self, which: usize) {
        if !self.chats.has("discord") {
            let title = self.t("dc.notOpen");
            self.notice_brief(&title, "", 2500);
            return;
        }
        // Discord's user panel (bottom left): its last two switches are the mic and the sound, on
        // the account row under anything a voice channel adds.
        let js = format!(
            "(() => {{ const s = document.querySelectorAll('section[class*=\"panels\"] button[role=\"switch\"]'); const b = s[s.length - 2 + {which}]; \
             if (!b) return 'none'; b.click(); return 'clicked'; }})()"
        );
        self.chats.execute("discord", &js, move |r| {
            if r.contains("none") {
                later(move |c| c.discord_voice_keys(which)); // no switch found: Discord's own keys
                return;
            }
            timer(250, move |c| c.discord_voice_said(which));
        });
    }

    /// Discord's own shortcuts (Ctrl+Shift+M, Ctrl+Shift+D), typed into its page.
    fn discord_voice_keys(&mut self, which: usize) {
        let (key, code, vk) = if which == 0 { ("M", "KeyM", 77) } else { ("D", "KeyD", 68) };
        for kind in ["rawKeyDown", "keyUp"] {
            let params = json!({ "type": kind, "modifiers": 2 | 8, "key": key, "code": code, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk });
            self.chats.cdp("discord", "Input.dispatchKeyEvent", &params.to_string(), |_| {});
        }
        log!("discord voice key {which}: no switch found, sent Discord's shortcut");
        let title = self.t(if which == 0 { "dc.muteToggled" } else { "dc.deafenToggled" });
        self.notice_brief(&title, "", 1800);
    }

    fn discord_voice_said(&mut self, which: usize) {
        let js = format!(
            "(() => {{ const s = document.querySelectorAll('section[class*=\"panels\"] button[role=\"switch\"]'); const b = s[s.length - 2 + {which}]; \
             return b ? b.getAttribute('aria-checked') : 'none'; }})()"
        );
        self.chats.execute("discord", &js, move |r| {
            let on = r.contains("true");
            later(move |c| {
                let key = match (which, on) {
                    (0, true) => "dc.muted",
                    (0, false) => "dc.unmuted",
                    (_, true) => "dc.deafened",
                    (_, false) => "dc.undeafened",
                };
                log!("discord voice {key}");
                let title = c.t(key);
                c.notice_brief(&title, "", 1800);
            });
        });
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
            pages.extend(c.whatsnew.win.clone());
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

/// Keys for Discord's mute / deafen (the panel's hotkeys are in HOTKEYS).
pub const VOICE_KEYS: [(&str, &str); 6] = [
    ("Control+Alt+M", "Ctrl + Alt + M"),
    ("Control+Alt+D", "Ctrl + Alt + D"),
    ("Control+Alt+Shift+M", "Ctrl + Alt + Shift + M"),
    ("Control+Alt+Shift+D", "Ctrl + Alt + Shift + D"),
    ("Alt+F9", "Alt + F9"),
    ("Alt+F10", "Alt + F10"),
];

pub const PREF_KEYS: [&str; 28] = [
    "lang",
    "side",
    "edgeMode",
    "edgeHold",
    "edgeWheel",
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
    "popupDisplay",
    "discordDms",
    "discordMuteKey",
    "discordDeafenKey",
    "updateAutoCheck",
    "updateAutoDownload",
];

/// "1.5.2" -> [1, 5, 2] (anything unreadable counts as 0)
pub fn version_parts(v: &str) -> Vec<u64> {
    v.split(['.', '-', '+']).take(3).map(|p| p.parse().unwrap_or(0)).collect()
}

/// "1.3.0" < "1.4.0" (numbers compared part by part)
pub fn version_less(a: &str, b: &str) -> bool {
    version_parts(a) < version_parts(b)
}

/// The points of a release's notes (Markdown from GitHub): its "- " lines, without the formatting.
fn note_lines(notes: &str) -> Vec<String> {
    notes
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix("- ").or_else(|| l.trim_start().strip_prefix("* ")))
        .map(|l| clean_text(&l.replace(['*', '`'], ""), 300))
        .filter(|l| !l.is_empty())
        .take(12)
        .collect()
}

/// Where a Discord notification's message is: "Name (#channel, Server)" -> (name, "#channel",
/// server). None for a direct message ("Name") or a group ("Name (Group)").
pub fn discord_place(title: &str) -> Option<(String, String, String)> {
    let t = title.trim();
    let inner = t.strip_suffix(')')?;
    let open = inner.find(" (#")?;
    let (who, rest) = (inner[..open].trim(), &inner[open + 2..]);
    let (channel, server) = rest.split_once(", ")?;
    let (channel, server) = (channel.trim(), server.trim());
    if who.is_empty() || channel.len() < 2 || server.is_empty() {
        return None;
    }
    Some((who.to_string(), channel.to_string(), server.to_string()))
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

/// How much of what a site counts the user has already seen, now that it counts `site`.
/// `counted`: this page has already shown a number, so `seen` has been checked against it.
fn seen_after(seen: u32, site: u32, in_view: bool, counted: bool) -> u32 {
    if site == 0 && !counted {
        seen // no number yet: the page may still be loading
    } else if in_view {
        site // on screen: everything it counts has been seen
    } else if counted || site >= seen {
        // watched all along, it went down (read elsewhere); or the same or more: the seen part stays seen
        seen.min(site)
    } else {
        // it went down while ChatDock wasn't watching (closed, the app asleep or off), then new
        // messages came: old and new can't be told apart, so none of them is hidden
        0
    }
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{same_folder, seen_after, test_folder};

    #[test]
    fn a_folder_is_the_same_however_it_is_written() {
        let tmp = std::env::temp_dir();
        let shouted = PathBuf::from(tmp.to_string_lossy().to_uppercase().replace('\\', "/") + "/");
        assert!(same_folder(&tmp, &shouted));
        assert!(same_folder(Path::new("."), &std::env::current_dir().unwrap()));
        assert!(!same_folder(&tmp, &tmp.join("chatdock-other")));
    }

    #[test]
    fn the_self_test_never_gets_chatdocks_own_data_folder() {
        let base = std::env::temp_dir().join("chatdock-appdata-test");
        let own = ["ChatDock", "ChatDock-dev", "ChatDockUpdTest"].map(|n| base.join(n));
        assert_eq!(test_folder(Some(base.join("ChatDock")), &own), None);
        let written_otherwise = base.join("chatdock-dev").to_string_lossy().replace('\\', "/") + "/";
        assert_eq!(test_folder(Some(PathBuf::from(written_otherwise)), &own), None);
        assert_eq!(test_folder(Some(PathBuf::new()), &own), None); // "--profile="
        assert_eq!(test_folder(None, &own), None);
        let test_dir = base.join("selftest-profile");
        assert_eq!(test_folder(Some(test_dir.clone()), &own), Some(test_dir));
    }

    // seen_after(seen, site, in_view, counted)

    #[test]
    fn on_screen_everything_counted_is_seen() {
        assert_eq!(seen_after(0, 3, true, true), 3);
        assert_eq!(seen_after(5, 2, true, false), 2);
    }

    #[test]
    fn watched_all_along_seen_follows_the_count_down() {
        assert_eq!(seen_after(5, 1, false, true), 1); // read elsewhere, e.g. on the phone
        assert_eq!(seen_after(5, 6, false, true), 5); // one new message
        assert_eq!(seen_after(3, 0, false, true), 0); // a zero that stuck
    }

    #[test]
    fn a_page_without_a_number_yet_keeps_what_was_seen() {
        // a reload or a wake: the title has no "(N)" while the page loads
        assert_eq!(seen_after(3, 0, false, false), 3);
        assert_eq!(seen_after(3, 0, true, false), 3);
    }

    #[test]
    fn a_new_page_with_the_same_or_a_higher_number_keeps_what_was_seen() {
        assert_eq!(seen_after(5, 5, false, false), 5); // the likes and follows seen before a restart
        assert_eq!(seen_after(5, 7, false, false), 5); // and two new messages
    }

    #[test]
    fn a_new_page_with_a_lower_number_shows_all_of_it() {
        // it went down while ChatDock wasn't watching (closed, the app asleep or off), then new
        // messages came: they can't be told apart from the old ones, so none of them is hidden
        assert_eq!(seen_after(5, 2, false, false), 0);
    }
}
