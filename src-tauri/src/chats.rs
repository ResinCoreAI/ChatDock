//! The chat sites, each in its own WebView2 inside the panel window. They run in the same WebView2
//! environment as ChatDock's own pages (one browser / GPU / network process for everything) and
//! each app has its own WebView2 *profile*, so its login, cookies and storage stay apart from the
//! other apps'. Measured with four apps: fewer processes and less RAM than the Electron build.
//! WebView2 callbacks arrive on the main thread; they decide what must be decided on the spot
//! (block a navigation, allow a permission) and hand the rest to the app state with later().

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
};

use webview2_com::{Microsoft::Web::WebView2::Win32::*, *};
use windows::{
    core::{Interface, BOOL, HSTRING, PWSTR},
    Win32::{
        Foundation::{CloseHandle, RECT},
        System::{
            Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS},
            ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX},
            Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        },
    },
};

use crate::{
    apps,
    core::{later, timer, Core, AUTO_RETRY_MS},
    log, rt,
    win32::{self, Rect},
};

/// Runs in every page and frame of the chat sites, before the site's own scripts.
/// 1. Passkey guard: login pages (Meta, the "Continue with Google" frame on X) ask for a passkey by
///    themselves, which pops a "Windows Security" dialog over the game. Passkeys are refused, so
///    the page falls back to password / QR login (the browser switch WebAuthenticationUseNativeWinApi
///    is also off).
/// 2. Sites that show notifications through their service worker (WhatsApp) show them as page
///    notifications instead, which ChatDock picks up like any other.
/// 3. WebSockets to this PC fail as if nothing listened there (WebView2 can't filter WebSockets
///    like other requests; Discord's page probes the desktop app on 127.0.0.1 and then nags
///    "Discord App Detected"). The socket goes to a name that never resolves instead.
const SITE_SCRIPT: &str = r#"(() => {
  if (window.WebSocket) {
    const Real = window.WebSocket;
    const local = /^wss?:\/\/(?:127(?:\.\d{1,3}){3}|localhost|\[::1\]|0\.0\.0\.0)(?::\d+)?(?:[\/?#]|$)/i;
    const WebSocket = function WebSocket(url, protocols) {
      if (!new.target) throw new TypeError("Failed to construct 'WebSocket': Please use the 'new' operator.");
      const target = local.test(String(url)) ? 'wss://local-blocked.chatdock.invalid/' : url;
      return protocols === undefined ? new Real(target) : new Real(target, protocols);
    };
    WebSocket.prototype = Real.prototype;
    Object.setPrototypeOf(WebSocket, Real);
    try { Object.defineProperty(Real.prototype, 'constructor', { value: WebSocket, configurable: true, writable: true }); } catch (e) {}
    window.WebSocket = WebSocket;
  }
  const report = (info) => { try { window.chrome.webview.postMessage(JSON.stringify({ type: 'passkey', ...info })); } catch (e) {} };
  const creds = navigator.credentials;
  if (creds) {
    const proto = Object.getPrototypeOf(creds);
    const guard = (original, kind) => function guarded(options) {
      if (options && options.publicKey) {
        report({ kind, origin: location.origin, mediation: String(options.mediation || '') });
        return Promise.reject(new DOMException('The operation is not allowed.', 'NotAllowedError'));
      }
      return original.call(this, options);
    };
    proto.get = guard(proto.get, 'get');
    proto.create = guard(proto.create, 'create');
  }
  if (window.PublicKeyCredential) {
    PublicKeyCredential.isConditionalMediationAvailable = () => Promise.resolve(false);
    PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable = () => Promise.resolve(false);
  }
  if (window.ServiceWorkerRegistration && window.Notification) {
    const sw = ServiceWorkerRegistration.prototype;
    sw.showNotification = function showNotification(title, options) {
      try { new Notification(title, options); } catch (e) {}
      return Promise.resolve();
    };
    sw.getNotifications = function getNotifications() { return Promise.resolve([]); };
  }
})();"#;

pub struct View {
    controller: ICoreWebView2Controller,
    webview: ICoreWebView2,
    playing: bool,
    visible: bool,
    low_memory: bool,
}

#[derive(Default)]
pub struct Chats {
    views: HashMap<String, View>,
    creating: HashSet<String>,
    env_ready: bool,
    waiting: Vec<String>,
    theme: (String, bool),
    memory: HashMap<String, u64>,
}

pub struct Memory {
    pub total: u64,
    pub per_app: HashMap<String, u64>,
    pub processes: u32,
}

thread_local! {
    static ENV: RefCell<Option<ICoreWebView2Environment>> = const { RefCell::new(None) };
    static CREATED: RefCell<HashMap<String, ICoreWebView2Controller>> = RefCell::new(HashMap::new());
    static NOTES: RefCell<HashMap<u64, (String, ICoreWebView2Notification)>> = RefCell::new(HashMap::new());
    static NOTE_SEQ: Cell<u64> = const { Cell::new(0) };
    /// Apps with a navigation ChatDock started itself (their home page, a retry): allowed wherever it goes.
    static OWN_NAV: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

fn env() -> Option<ICoreWebView2Environment> {
    ENV.with(|e| e.borrow().clone())
}

fn pwstr_string(p: PWSTR) -> String {
    take_pwstr(p)
}

fn color(dark: bool) -> COREWEBVIEW2_COLOR {
    if dark {
        COREWEBVIEW2_COLOR { A: 255, R: 24, G: 25, B: 29 }
    } else {
        COREWEBVIEW2_COLOR { A: 255, R: 255, G: 255, B: 255 }
    }
}

impl Chats {
    pub fn has(&self, id: &str) -> bool {
        self.views.contains_key(id)
    }

    pub fn ids(&self) -> Vec<String> {
        self.views.keys().cloned().collect()
    }

    pub fn playing_audio(&self, id: &str) -> bool {
        self.views.get(id).is_some_and(|v| v.playing)
    }

    pub fn is_visible(&self, id: &str) -> bool {
        self.views.get(id).is_some_and(|v| v.visible)
    }

    /// Where the page sits inside the panel window (physical pixels).
    pub fn bounds(&self, id: &str) -> Option<Rect> {
        let v = self.views.get(id)?;
        let mut r = RECT::default();
        unsafe { v.controller.Bounds(&mut r).ok()? };
        Some(Rect { x: r.left, y: r.top, w: r.right - r.left, h: r.bottom - r.top })
    }

    pub fn webview(&self, id: &str) -> Option<ICoreWebView2> {
        self.views.get(id).map(|v| v.webview.clone())
    }

    pub fn popups_open(&self, _id: &str) -> u32 {
        0
    }

    pub fn set_bounds(&self, id: &str, r: Rect) {
        if let Some(v) = self.views.get(id) {
            unsafe {
                let _ = v.controller.SetBounds(RECT { left: r.x, top: r.y, right: r.x + r.w, bottom: r.y + r.h });
            }
        }
    }

    /// Shown or not (a hidden page counts as hidden for the site too, like a background tab).
    /// Apps that are not the one in use also give memory back; they still get their messages.
    pub fn set_visible(&mut self, id: &str, on: bool, in_use: bool) {
        if let Some(v) = self.views.get_mut(id) {
            if v.visible != on {
                v.visible = on;
                unsafe {
                    let _ = v.controller.SetIsVisible(on);
                }
            }
            let low = !on && !in_use;
            if v.low_memory != low {
                v.low_memory = low;
                if let Ok(wv19) = v.webview.cast::<ICoreWebView2_19>() {
                    let level =
                        if low { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW } else { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL };
                    unsafe {
                        let _ = wv19.SetMemoryUsageTargetLevel(level);
                    }
                }
            }
        }
    }

    /// Keyboard into the chat page. False when it isn't showing.
    pub fn focus(&self, id: &str) -> bool {
        match self.views.get(id) {
            Some(v) if v.visible => unsafe { v.controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC).is_ok() },
            _ => false,
        }
    }

    pub fn navigate(&self, id: &str, url: &str) {
        if let Some(v) = self.views.get(id) {
            OWN_NAV.with(|n| n.borrow_mut().insert(id.to_string()));
            unsafe {
                if v.webview.Navigate(&HSTRING::from(url)).is_err() {
                    OWN_NAV.with(|n| n.borrow_mut().remove(id));
                }
            }
        }
    }

    pub fn reload(&self, id: &str) {
        if let Some(v) = self.views.get(id) {
            unsafe {
                let _ = v.webview.Reload();
            }
        }
    }

    pub fn source(&self, id: &str) -> String {
        match self.views.get(id) {
            Some(v) => unsafe {
                let mut p = PWSTR::null();
                if v.webview.Source(&mut p).is_ok() {
                    pwstr_string(p)
                } else {
                    String::new()
                }
            },
            None => String::new(),
        }
    }

    pub fn go_back(&self, id: &str) {
        if let Some(v) = self.views.get(id) {
            unsafe {
                let mut can = BOOL(0);
                if v.webview.CanGoBack(&mut can).is_ok() && can.as_bool() {
                    let _ = v.webview.GoBack();
                }
            }
        }
    }

    pub fn go_forward(&self, id: &str) {
        if let Some(v) = self.views.get(id) {
            unsafe {
                let mut can = BOOL(0);
                if v.webview.CanGoForward(&mut can).is_ok() && can.as_bool() {
                    let _ = v.webview.GoForward();
                }
            }
        }
    }

    pub fn set_zoom(&self, id: &str, factor: f64) {
        if let Some(v) = self.views.get(id) {
            unsafe {
                let _ = v.controller.SetZoomFactor(factor);
            }
        }
    }

    pub fn set_muted(&self, id: &str, muted: bool) {
        if let Some(v) = self.views.get(id) {
            if let Ok(wv8) = v.webview.cast::<ICoreWebView2_8>() {
                unsafe {
                    let _ = wv8.SetIsMuted(muted);
                }
            }
        }
    }

    /// Chat pages follow ChatDock's theme (their prefers-color-scheme), like the panel.
    pub fn set_theme(&mut self, theme: &str, dark: bool) {
        self.theme = (theme.to_string(), dark);
        for v in self.views.values() {
            apply_theme(v, theme, dark);
        }
    }

    /// After the panel moved: pop-ups inside the pages (menus, IME) follow it.
    pub fn notify_moved(&self) {
        for v in self.views.values() {
            unsafe {
                let _ = v.controller.NotifyParentWindowPositionChanged();
            }
        }
    }

    pub fn execute(&self, id: &str, js: &str, done: impl FnOnce(String) + 'static) {
        if let Some(v) = self.views.get(id) {
            let mut done = Some(done);
            unsafe {
                let _ = v.webview.ExecuteScript(
                    &HSTRING::from(js),
                    &ExecuteScriptCompletedHandler::create(Box::new(move |_, result| {
                        if let Some(f) = done.take() {
                            f(result);
                        }
                        Ok(())
                    })),
                );
            }
        }
    }

    fn close(&mut self, id: &str) {
        OWN_NAV.with(|n| n.borrow_mut().remove(id));
        if let Some(v) = self.views.remove(id) {
            unsafe {
                let _ = v.controller.Close();
            }
        }
        NOTES.with(|n| n.borrow_mut().retain(|_, (app, _)| app != id));
    }

    /// RAM in MB: all of ChatDock (this process and every WebView2 process it started), and the
    /// last measured share of each app (see refresh_memory).
    pub fn memory(&self) -> Memory {
        let tree = process_tree(std::process::id());
        let total: u64 = tree.iter().map(|pid| private_mb(*pid)).sum();
        Memory { total, per_app: self.memory.clone(), processes: tree.len() as u32 }
    }
}

fn apply_theme(v: &View, theme: &str, dark: bool) {
    unsafe {
        if let Ok(c2) = v.controller.cast::<ICoreWebView2Controller2>() {
            let _ = c2.SetDefaultBackgroundColor(color(dark));
        }
        if let Ok(wv13) = v.webview.cast::<ICoreWebView2_13>() {
            if let Ok(profile) = wv13.Profile() {
                let scheme = match theme {
                    "dark" => COREWEBVIEW2_PREFERRED_COLOR_SCHEME_DARK,
                    "light" => COREWEBVIEW2_PREFERRED_COLOR_SCHEME_LIGHT,
                    _ => COREWEBVIEW2_PREFERRED_COLOR_SCHEME_AUTO,
                };
                let _ = profile.SetPreferredColorScheme(scheme);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Processes and their RAM
// ---------------------------------------------------------------------------------------------
fn process_tree(root: u32) -> Vec<u32> {
    let mut parents: Vec<(u32, u32)> = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return vec![root] };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snap, &mut e).is_ok() {
            loop {
                parents.push((e.th32ProcessID, e.th32ParentProcessID));
                if Process32NextW(snap, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    let mut tree = vec![root];
    let mut i = 0;
    while i < tree.len() {
        let p = tree[i];
        for (pid, parent) in &parents {
            if *parent == p && !tree.contains(pid) {
                tree.push(*pid);
            }
        }
        i += 1;
    }
    tree
}

fn private_mb(pid: u32) -> u64 {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return 0 };
        let mut c = PROCESS_MEMORY_COUNTERS_EX { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
        let ok = K32GetProcessMemoryInfo(h, &mut c as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS, c.cb).as_bool();
        let _ = CloseHandle(h);
        if ok {
            (c.PrivateUsage / (1024 * 1024)) as u64
        } else {
            0
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The app state's side: making, loading, sleeping and clearing the chat views
// ---------------------------------------------------------------------------------------------
impl Core {
    pub fn init_chats(&mut self) {
        let debug = self.args.debug;
        let _ = self.panel.w.with_webview(move |pw| {
            ENV.with(|e| *e.borrow_mut() = Some(pw.environment()));
            // the shortcuts work in the panel's own page too (Esc Esc, Ctrl+1..9, ...)
            hook_keys(&pw.controller(), None);
            if let Ok(wv) = unsafe { pw.controller().CoreWebView2() } {
                if let Ok(s) = unsafe { wv.Settings() } {
                    unsafe {
                        let _ = s.SetAreDefaultContextMenusEnabled(debug);
                        let _ = s.SetIsStatusBarEnabled(false);
                    }
                }
            }
            later(|c| {
                // every child of the panel window so far belongs to the panel page itself
                c.panel_page_hwnds = win32::children(c.panel.hwnd);
                log!("panel page windows {:?}", c.panel_page_hwnds.iter().map(|w| win32::class_name(*w)).collect::<Vec<_>>());
                c.chats_ready();
            });
        });
    }

    pub fn chats_ready(&mut self) {
        self.chats.env_ready = true;
        for id in self.enabled_apps() {
            if self.sleep_eligible(id) {
                self.asleep.insert(id.to_string(), true); // loads when it is first opened
            } else {
                self.create_view(id);
            }
        }
        for id in std::mem::take(&mut self.chats.waiting) {
            self.create_view(&id);
        }
        timer(5000, |c| c.refresh_memory());
    }

    pub fn create_view(&mut self, id: &str) {
        if self.chats.has(id) || self.chats.creating.contains(id) || apps::get(id).is_none() {
            return;
        }
        if !self.chats.env_ready {
            self.chats.waiting.push(id.to_string());
            return;
        }
        let Some(env) = env() else { return };
        self.chats.creating.insert(id.to_string());
        let app = id.to_string();
        let result = unsafe {
            (|| -> windows::core::Result<()> {
                let env10: ICoreWebView2Environment10 = env.cast()?;
                let opts = env10.CreateCoreWebView2ControllerOptions()?;
                opts.SetProfileName(&HSTRING::from(id))?;
                opts.SetIsInPrivateModeEnabled(false)?;
                if let Ok(o3) = opts.cast::<ICoreWebView2ControllerOptions3>() {
                    let _ = o3.SetDefaultBackgroundColor(color(self.chats.theme.1));
                }
                env10.CreateCoreWebView2ControllerWithOptions(
                    win32::h(self.panel.hwnd),
                    &opts,
                    &CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |err, controller| {
                        match (err, controller) {
                            (Ok(()), Some(ctrl)) => {
                                CREATED.with(|m| m.borrow_mut().insert(app.clone(), ctrl));
                                let a = app.clone();
                                later(move |c| c.view_created(&a));
                            }
                            (err, _) => {
                                log!("chat view {app} could not be made: {err:?}");
                                let a = app.clone();
                                later(move |c| {
                                    c.chats.creating.remove(&a);
                                });
                            }
                        }
                        Ok(())
                    })),
                )
            })()
        };
        if let Err(err) = result {
            log!("chat view {id}: {err}");
            self.chats.creating.remove(id);
        }
    }

    pub fn view_created(&mut self, id: &str) {
        self.chats.creating.remove(id);
        let Some(controller) = CREATED.with(|m| m.borrow_mut().remove(id)) else { return };
        if !self.is_enabled(id) || *self.asleep.get(id).unwrap_or(&false) {
            unsafe {
                let _ = controller.Close(); // switched off or put to sleep meanwhile
            }
            return;
        }
        let webview = match unsafe { controller.CoreWebView2() } {
            Ok(wv) => wv,
            Err(err) => {
                log!("chat view {id}: {err}");
                return;
            }
        };
        if let Err(err) = unsafe { configure(&controller, &webview, id, self.args.debug) } {
            log!("chat view {id} setup: {err}");
        }
        let view = View { controller, webview, playing: false, visible: true, low_memory: false };
        let (theme, dark) = self.chats.theme.clone();
        apply_theme(&view, &theme, dark);
        self.chats.views.insert(id.to_string(), view);
        let in_use = self.active() == id;
        self.chats.set_visible(id, false, in_use);
        self.chats.set_zoom(id, self.zoom_of(id));
        self.apply_audio(id);
        self.load_state.insert(id.to_string(), "loading");
        self.first_shown.insert(id.to_string(), false);
        self.layout_views();
        self.load_home(id);
    }

    pub fn destroy_view(&mut self, id: &str) {
        if !self.chats.has(id) {
            return;
        }
        self.chats.close(id);
        for timers in [&mut self.retry_timers, &mut self.zero_timers, &mut self.fallback_timers] {
            if let Some(t) = timers.remove(id) {
                rt::cancel(t);
            }
        }
        self.counts.insert(id.to_string(), 0);
        self.load_state.insert(id.to_string(), "loading");
        self.first_shown.insert(id.to_string(), false);
    }

    pub fn load_home(&mut self, id: &str) {
        if let Some(a) = apps::get(id) {
            self.chats.navigate(id, a.home);
        }
    }

    pub fn reload_app(&mut self, id: &str, _hard: bool) {
        if !self.chats.has(id) {
            return;
        }
        if self.load_state.get(id).copied() == Some("error") {
            self.load_home(id);
        } else {
            self.chats.reload(id);
        }
    }

    pub fn go_home(&mut self, id: &str) {
        let Some(a) = apps::get(id) else { return };
        let path = tauri::Url::parse(&self.chats.source(id)).map(|u| u.path().to_string()).unwrap_or_default();
        if !path.starts_with(a.home_path) {
            self.load_home(id);
        }
    }

    fn set_load(&mut self, id: &str, value: &'static str) {
        if self.load_state.get(id).copied() == Some(value) {
            return;
        }
        self.load_state.insert(id.to_string(), value);
        if value != "error" {
            if let Some(t) = self.retry_timers.remove(id) {
                rt::cancel(t);
            }
        }
        self.layout_views();
        self.broadcast_state();
    }

    fn on_load_start(&mut self, id: &str) {
        self.load_started_at.insert(id.to_string(), rt::epoch_ms());
        self.set_load(id, "loading");
    }

    fn on_load_done(&mut self, id: &str, ok: bool, status: i32) {
        if ok {
            if self.load_state.get(id).copied() == Some("loading") {
                self.set_load(id, "ready");
            }
            return;
        }
        // cancelled = normal when navigating away or when a link opened in the browser instead
        if status == COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED.0
            || status == COREWEBVIEW2_WEB_ERROR_STATUS_VALID_AUTHENTICATION_CREDENTIALS_REQUIRED.0
        {
            if self.load_state.get(id).copied() == Some("loading") {
                self.set_load(id, "ready");
            }
            return;
        }
        log!("load failed {id} status {status}");
        self.set_load(id, "error");
        if let Some(t) = self.retry_timers.remove(id) {
            rt::cancel(t);
        }
        let app = id.to_string();
        let t = timer(AUTO_RETRY_MS, move |c| {
            c.retry_timers.remove(&app);
            if c.load_state.get(&app).copied() == Some("error") {
                c.reload_app(&app, false);
            }
        });
        self.retry_timers.insert(id.to_string(), t);
    }

    fn on_dom_ready(&mut self, id: &str) {
        let z = self.zoom_of(id);
        self.chats.set_zoom(id, z);
        if !*self.first_shown.get(id).unwrap_or(&false) {
            self.first_shown.insert(id.to_string(), true);
            self.layout_views();
            self.broadcast_state();
        }
    }

    pub fn open_external(&self, url: &str) {
        win32::open_url(&apps::unshim(url));
    }

    /// Wipe everything an app stored on this PC (login, cookies, cache) and start it fresh.
    pub fn clear_app_data(&mut self, id: &str) {
        self.toasts_dismiss_app(id);
        self.set_count(id, 0);
        let Some(v) = self.chats.views.get(id) else {
            log!("data cleared {id} (not loaded)");
            return;
        };
        let app = id.to_string();
        unsafe {
            let result =
                v.webview.cast::<ICoreWebView2_13>().and_then(|w| w.Profile()).and_then(|p| p.cast::<ICoreWebView2Profile2>()).and_then(
                    |p2| {
                        p2.ClearBrowsingDataAll(&ClearBrowsingDataCompletedHandler::create(Box::new(move |_| {
                            let a = app.clone();
                            later(move |c| {
                                c.load_home(&a);
                                log!("data cleared {a}");
                            });
                            Ok(())
                        })))
                    },
                );
            if let Err(err) = result {
                log!("clear data failed {id}: {err}");
            }
        }
    }

    /// A pop-up for a site notification was clicked: the site's own click handler runs, so the
    /// site opens that conversation.
    pub fn click_notification(&mut self, key: u64) {
        if let Some((_, n)) = NOTES.with(|m| m.borrow_mut().remove(&key)) {
            unsafe {
                let _ = n.ReportClicked();
            }
        }
    }

    /// How much RAM each app uses: WebView2 lists its processes with the frames they draw; each
    /// renderer counts for the app whose page (or frame inside it) it shows.
    pub fn refresh_memory(&mut self) {
        let Some(env) = env() else { return };
        let pages: Vec<(String, String)> = self.chats.views.keys().map(|id| (id.clone(), self.chats.source(id))).collect();
        let Ok(env13) = env.cast::<ICoreWebView2Environment13>() else { return };
        unsafe {
            let _ = env13.GetProcessExtendedInfos(&GetProcessExtendedInfosCompletedHandler::create(Box::new(move |_, list| {
                let mut per_app: HashMap<String, u64> = HashMap::new();
                if let Some(list) = list {
                    let mut count = 0u32;
                    let _ = list.Count(&mut count);
                    for i in 0..count {
                        let Ok(info) = list.GetValueAtIndex(i) else { continue };
                        let Ok(p) = info.ProcessInfo() else { continue };
                        let mut kind = COREWEBVIEW2_PROCESS_KIND::default();
                        let _ = p.Kind(&mut kind);
                        if kind != COREWEBVIEW2_PROCESS_KIND_RENDERER {
                            continue;
                        }
                        let mut pid = 0i32;
                        let _ = p.ProcessId(&mut pid);
                        let Some(app) = renderer_app(&info, &pages) else { continue };
                        *per_app.entry(app).or_default() += private_mb(pid as u32);
                    }
                }
                later(move |c| c.chats.memory = per_app);
                Ok(())
            })));
        }
        if self.settings_mode {
            timer(4000, |c| c.refresh_memory());
        }
    }
}

fn renderer_app(info: &ICoreWebView2ProcessExtendedInfo, pages: &[(String, String)]) -> Option<String> {
    unsafe {
        let frames = info.AssociatedFrameInfos().ok()?;
        let it = frames.GetIterator().ok()?;
        let mut has = BOOL(0);
        while it.HasCurrent(&mut has).is_ok() && has.as_bool() {
            if let Ok(frame) = it.GetCurrent() {
                let mut top = frame.clone();
                if let Ok(f2) = frame.cast::<ICoreWebView2FrameInfo2>() {
                    let mut cur = f2;
                    while let Ok(parent) = cur.ParentFrameInfo() {
                        top = parent.clone();
                        match parent.cast::<ICoreWebView2FrameInfo2>() {
                            Ok(p2) => cur = p2,
                            Err(_) => break,
                        }
                    }
                }
                let mut src = PWSTR::null();
                if top.Source(&mut src).is_ok() {
                    let src = take_pwstr(src);
                    if let Some((id, _)) = pages.iter().find(|(_, page)| !page.is_empty() && *page == src) {
                        return Some(id.clone());
                    }
                }
            }
            let mut more = BOOL(0);
            if it.MoveNext(&mut more).is_err() || !more.as_bool() {
                break;
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------------------------
// Setting up one chat view
// ---------------------------------------------------------------------------------------------
fn allowed_permission(kind: COREWEBVIEW2_PERMISSION_KIND) -> bool {
    [
        COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS,
        COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
        COREWEBVIEW2_PERMISSION_KIND_CAMERA,
        COREWEBVIEW2_PERMISSION_KIND_AUTOPLAY,
        COREWEBVIEW2_PERMISSION_KIND_MULTIPLE_AUTOMATIC_DOWNLOADS,
    ]
    .contains(&kind)
}

unsafe fn configure(controller: &ICoreWebView2Controller, wv: &ICoreWebView2, id: &str, debug: bool) -> windows::core::Result<()> {
    let s = wv.Settings()?;
    s.SetAreDevToolsEnabled(debug)?;
    s.SetIsStatusBarEnabled(false)?;
    s.SetIsZoomControlEnabled(true)?;
    s.SetIsBuiltInErrorPageEnabled(false)?; // a failed load shows ChatDock's own "can't connect" screen
    if let Ok(s4) = s.cast::<ICoreWebView2Settings4>() {
        s4.SetIsPasswordAutosaveEnabled(false)?;
        s4.SetIsGeneralAutofillEnabled(false)?;
    }
    if let Ok(s6) = s.cast::<ICoreWebView2Settings6>() {
        s6.SetIsSwipeNavigationEnabled(false)?;
    }
    if let Ok(s8) = s.cast::<ICoreWebView2Settings8>() {
        s8.SetIsReputationCheckingRequired(false)?;
    }
    wv.AddScriptToExecuteOnDocumentCreated(
        &HSTRING::from(SITE_SCRIPT),
        &AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(|_, _| Ok(()))),
    )?;
    // The app's own pages may show notifications from the start (like a site you allowed in a browser)
    if let Ok(profile) = wv.cast::<ICoreWebView2_13>().and_then(|w| w.Profile()) {
        if let Ok(p4) = profile.cast::<ICoreWebView2Profile4>() {
            for origin in apps::notification_origins(id) {
                let _ = p4.SetPermissionState(
                    COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS,
                    &HSTRING::from(origin),
                    COREWEBVIEW2_PERMISSION_STATE_ALLOW,
                    &SetPermissionStateCompletedHandler::create(Box::new(|_| Ok(()))),
                );
            }
        }
    }
    // Chat sites have no business talking to programs on this PC. (Discord's page probes the
    // desktop app on localhost and then nags "Discord App Detected".)
    for f in ["http://127.0.0.1*", "https://127.0.0.1*", "http://localhost*", "https://localhost*", "http://[::1]*", "https://[::1]*"] {
        wv.AddWebResourceRequestedFilter(&HSTRING::from(f), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?;
    }
    let mut token = 0i64;
    wv.add_WebResourceRequested(
        &WebResourceRequestedEventHandler::create(Box::new(|_, args| {
            if let (Some(args), Some(env)) = (args, env()) {
                let resp = env.CreateWebResourceResponse(None, 403, &HSTRING::from("Blocked"), &HSTRING::from(""))?;
                args.SetResponse(&resp)?;
            }
            Ok(())
        })),
        &mut token,
    )?;

    let app = id.to_string();
    wv.add_NavigationStarting(
        &NavigationStartingEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut p = PWSTR::null();
            args.Uri(&mut p)?;
            let uri = take_pwstr(p);
            let own = OWN_NAV.with(|n| n.borrow_mut().remove(&app));
            if own || uri.starts_with("about:") || apps::keep_inside(&app, &uri) {
                let a = app.clone();
                later(move |c| c.on_load_start(&a));
            } else {
                args.SetCancel(true)?; // anything else opens in the normal browser
                later(move |c| c.open_external(&uri));
            }
            Ok(())
        })),
        &mut token,
    )?;

    let app = id.to_string();
    wv.add_NavigationCompleted(
        &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut ok = BOOL(0);
            args.IsSuccess(&mut ok)?;
            let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
            args.WebErrorStatus(&mut status)?;
            let (a, ok, st) = (app.clone(), ok.as_bool(), status.0);
            later(move |c| c.on_load_done(&a, ok, st));
            Ok(())
        })),
        &mut token,
    )?;

    let app = id.to_string();
    wv.cast::<ICoreWebView2_2>()?.add_DOMContentLoaded(
        &DOMContentLoadedEventHandler::create(Box::new(move |_, _| {
            let a = app.clone();
            later(move |c| c.on_dom_ready(&a));
            Ok(())
        })),
        &mut token,
    )?;

    let app = id.to_string();
    wv.add_DocumentTitleChanged(
        &DocumentTitleChangedEventHandler::create(Box::new(move |sender, _| {
            if let Some(sender) = sender {
                let mut p = PWSTR::null();
                sender.DocumentTitle(&mut p)?;
                let (a, title) = (app.clone(), take_pwstr(p));
                later(move |c| c.on_title(&a, &title));
            }
            Ok(())
        })),
        &mut token,
    )?;

    let app = id.to_string();
    wv.add_NewWindowRequested(
        &NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut p = PWSTR::null();
            args.Uri(&mut p)?;
            let uri = take_pwstr(p);
            // Voice / video calls and "Sign in with Google/Apple" need their own small window
            if uri == "about:blank" || apps::is_call_url(&app, &uri) || apps::is_auth_popup(&app, &uri) {
                return Ok(());
            }
            args.SetHandled(true)?;
            later(move |c| c.open_external(&uri));
            Ok(())
        })),
        &mut token,
    )?;

    let app = id.to_string();
    wv.add_PermissionRequested(
        &PermissionRequestedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
            args.PermissionKind(&mut kind)?;
            let mut p = PWSTR::null();
            args.Uri(&mut p)?;
            let uri = take_pwstr(p);
            let allow = allowed_permission(kind) && apps::owns(&app, &uri);
            args.SetState(if allow { COREWEBVIEW2_PERMISSION_STATE_ALLOW } else { COREWEBVIEW2_PERMISSION_STATE_DENY })?;
            if !allow {
                log!("permission {} refused for {app}", kind.0);
            }
            Ok(())
        })),
        &mut token,
    )?;

    if let Ok(wv24) = wv.cast::<ICoreWebView2_24>() {
        let app = id.to_string();
        wv24.add_NotificationReceived(
            &NotificationReceivedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                args.SetHandled(true)?; // no Windows toast: ChatDock shows its own pop-up
                let n = args.Notification()?;
                let text = |f: &dyn Fn(&mut PWSTR) -> windows::core::Result<()>| {
                    let mut p = PWSTR::null();
                    if f(&mut p).is_ok() {
                        take_pwstr(p)
                    } else {
                        String::new()
                    }
                };
                let title = text(&|p| n.Title(p));
                let body = text(&|p| n.Body(p));
                let tag = text(&|p| n.Tag(p));
                let icon = text(&|p| n.IconUri(p));
                let _ = n.ReportShown();
                let key = NOTE_SEQ.with(|s| {
                    s.set(s.get() + 1);
                    s.get()
                });
                // the site closed it itself (read elsewhere): the pop-up goes too
                let mut t = 0i64;
                let a = app.clone();
                let _ = n.add_CloseRequested(
                    &NotificationCloseRequestedEventHandler::create(Box::new(move |_, _| {
                        NOTES.with(|m| m.borrow_mut().remove(&key));
                        let a = a.clone();
                        later(move |c| c.toasts_dismiss_source(&a, key));
                        Ok(())
                    })),
                    &mut t,
                );
                NOTES.with(|m| m.borrow_mut().insert(key, (app.clone(), n)));
                let a = app.clone();
                later(move |c| c.on_site_notification(&a, key, &title, &body, &icon, &tag));
                Ok(())
            })),
            &mut token,
        )?;
    }

    let app = id.to_string();
    wv.add_ProcessFailed(
        &ProcessFailedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
            args.ProcessFailedKind(&mut kind)?;
            log!("renderer gone {app} kind {}", kind.0);
            if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED
                || kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE
            {
                let a = app.clone();
                timer(1500, move |c| c.chats.reload(&a));
            }
            Ok(())
        })),
        &mut token,
    )?;

    if let Ok(wv8) = wv.cast::<ICoreWebView2_8>() {
        let app = id.to_string();
        wv8.add_IsDocumentPlayingAudioChanged(
            &IsDocumentPlayingAudioChangedEventHandler::create(Box::new(move |sender, _| {
                if let Some(sender) = sender {
                    let mut on = BOOL(0);
                    if let Ok(s8) = sender.cast::<ICoreWebView2_8>() {
                        let _ = s8.IsDocumentPlayingAudio(&mut on);
                    }
                    let (a, on) = (app.clone(), on.as_bool());
                    later(move |c| {
                        if let Some(v) = c.chats.views.get_mut(&a) {
                            v.playing = on;
                        }
                        c.last_used.insert(a, rt::epoch_ms()); // playing sound counts as in use
                    });
                }
                Ok(())
            })),
            &mut token,
        )?;
    }

    let app = id.to_string();
    wv.add_WebMessageReceived(
        &WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
            if let Some(args) = args {
                let mut p = PWSTR::null();
                if args.TryGetWebMessageAsString(&mut p).is_ok() {
                    let msg = take_pwstr(p);
                    if msg.contains("\"passkey\"") {
                        log!("passkey request blocked {app} {}", crate::core::clean_text(&msg, 200));
                    }
                }
            }
            Ok(())
        })),
        &mut token,
    )?;

    let app = id.to_string();
    controller.add_ZoomFactorChanged(
        &ZoomFactorChangedEventHandler::create(Box::new(move |sender, _| {
            if let Some(sender) = sender {
                let mut z = 1.0f64;
                sender.ZoomFactor(&mut z)?;
                let a = app.clone();
                later(move |c| c.zoom_changed(&a, z));
            }
            Ok(())
        })),
        &mut token,
    )?;

    hook_keys(controller, Some(id));
    Ok(())
}

/// ChatDock's keyboard shortcuts inside a page (a chat, or the panel's own page when app is None).
fn hook_keys(controller: &ICoreWebView2Controller, app: Option<&str>) {
    let host = app.is_none();
    let mut token = 0i64;
    unsafe {
        let _ = controller.add_AcceleratorKeyPressed(
            &AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
                args.KeyEventKind(&mut kind)?;
                if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN {
                    return Ok(());
                }
                let mut status = COREWEBVIEW2_PHYSICAL_KEY_STATUS::default();
                args.PhysicalKeyStatus(&mut status)?;
                if status.WasKeyDown.as_bool() {
                    return Ok(()); // held down: act once
                }
                let mut vk = 0u32;
                args.VirtualKey(&mut vk)?;
                let (ctrl, alt, shift) = win32::modifiers();
                let ours = match vk {
                    0x1B => false, // Esc: the page still gets it (closes its own pop-ups)
                    0x31..=0x39 | 0x61..=0x69 => ctrl && !alt && !shift,
                    0x09 => ctrl,
                    0x52 | 0x57 => ctrl && !alt,
                    0x74 => !alt,
                    0xBB | 0x6B | 0xBD | 0x6D | 0x30 | 0x60 => ctrl && !alt,
                    0x25 | 0x27 => alt && !ctrl,
                    _ => return Ok(()),
                };
                if ours {
                    args.SetHandled(true)?;
                }
                later(move |c| c.on_key(vk, ctrl, alt, shift, host));
                Ok(())
            })),
            &mut token,
        );
    }
}
