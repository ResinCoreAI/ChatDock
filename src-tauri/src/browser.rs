//! The Browser tab: ChatDock's own small web browser (added from "+"). Its pages run in a WebView2
//! browser process of their own (data_dir/WebView2-browser), apart from the chats' logins, with
//! SmartScreen on. Its tab strip, navigation row and start page are drawn by the panel page
//! (ui/browser.js); the web pages are WebView2 views stacked under that bar.
//! WebView2 callbacks arrive on the main thread and hand their work to the app state with later().

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicU32, Ordering},
};

use serde_json::{json, Value};
use webview2_com::{Microsoft::Web::WebView2::Win32::*, *};
use windows::{
    core::{Interface, BOOL, HSTRING, PCWSTR, PWSTR},
    Win32::Foundation::RECT,
};

use crate::{
    apps,
    browser_tabs::Tabs,
    chats::{apply_settings, color, UiOnly},
    core::{later, timer, Core},
    log, rt, search,
    win32::{self, Rect},
};

pub const ID: &str = "browser";

/// The Browser's bar in the panel page (its tab strip 36 + navigation row 42), CSS px; a notice
/// under it (a refused download, a program link) adds NOTICE_H while it shows.
pub const BAR_H: f64 = 78.0;
pub const NOTICE_H: f64 = 36.0;

#[derive(Default, PartialEq, Clone, Copy, Debug)]
enum EnvState {
    #[default]
    NotStarted,
    Starting,
    Ready,
}

struct BView {
    controller: ICoreWebView2Controller,
    webview: ICoreWebView2,
    visible: bool,
    low_memory: bool,
    playing: bool,
    loading: bool,
    can_back: bool,
    can_forward: bool,
    icon: String,
}

#[derive(Default)]
pub struct Browser {
    pub tabs: Tabs,
    views: HashMap<u32, BView>,
    creating: HashSet<u32>,
    env: EnvState,
    /// a tab's page still to load once its view is made
    pending_nav: HashMap<u32, String>,
    /// the bar under the navigation row: Null, or {kind: "download" | "local" | "program", ...}
    pub notice: Value,
    /// a suggestion request in flight, and the newest text typed meanwhile
    suggest_busy: bool,
    suggest_next: Option<String>,
}

impl Browser {
    pub fn restore(&mut self, saved: &Value, now: i64) {
        let open = saved.get("open").and_then(Value::as_u64).unwrap_or(0) as usize;
        self.tabs = Tabs::restore(&saved["tabs"], open, now);
    }
}

thread_local! {
    static ENV: RefCell<Option<ICoreWebView2Environment>> = const { RefCell::new(None) };
    static MADE: RefCell<HashMap<u32, ICoreWebView2Controller>> = RefCell::new(HashMap::new());
    /// a page's new window waiting for the view that will become its tab
    static NEW_WINDOWS: RefCell<HashMap<u32, (ICoreWebView2NewWindowRequestedEventArgs, ICoreWebView2Deferral)>> = RefCell::new(HashMap::new());
    /// when each tab's page crashed lately (one that crashes on every load mustn't reload forever)
    static CRASHES: RefCell<HashMap<u32, Vec<i64>>> = RefCell::new(HashMap::new());
}

static PID: AtomicU32 = AtomicU32::new(0);

/// The Browser's own browser process (focus and auto-hide treat its windows as ChatDock's).
pub fn browser_pid() -> u32 {
    PID.load(Ordering::Relaxed)
}

/// Its Chromium switches: the chats' privacy ones, without what the chats turn off for comfort
/// (SmartScreen, the autoplay rule, process-per-site): the Browser opens any site.
fn browser_args() -> String {
    "--disable-features=WebAuthenticationUseNativeWinApi,WebRtcHideLocalIpsWithMdns,msWebOOUI,msPdfOOUI \
     --force-webrtc-ip-handling-policy=default_public_interface_only"
        .to_string()
}

/// ChatDock's tabs in order: the chat apps, then the Browser when it's on.
pub fn tab_order(mut apps: Vec<&'static str>, browser_on: bool) -> Vec<&'static str> {
    if browser_on {
        apps.push(ID);
    }
    apps
}

impl Core {
    pub fn browser_on(&self) -> bool {
        self.settings.app_on(ID)
    }

    /// Switched on from "+" / off from its tab's menu. Off closes its pages and forgets its tabs
    /// (shortcuts, engine and recent searches stay).
    pub fn set_browser_enabled(&mut self, on: bool) {
        if self.browser_on() == on {
            return;
        }
        self.settings.set_in("apps", ID, json!(on));
        if !on {
            self.browser_close_all();
            self.browser.tabs = Tabs::default();
            self.browser_save();
            if self.active() == ID {
                let first = self.enabled_apps()[0];
                self.set_active(first, false);
            }
        }
        self.save_soon();
        log!("browser {}", if on { "on" } else { "off" });
        self.layout_views();
        self.broadcast_state();
        self.browser_emit();
    }

    /// The Browser's tabs and open tab into settings.json.
    pub fn browser_save(&mut self) {
        let mut b = self.settings.get(ID).clone();
        b["tabs"] = self.browser.tabs.saved();
        b["open"] = json!(self.browser.tabs.open_index());
        self.settings.set(ID, b);
        self.save_soon();
    }

    // -----------------------------------------------------------------------------------------
    // Its own WebView2 environment
    // -----------------------------------------------------------------------------------------
    fn browser_env(&mut self) -> Option<ICoreWebView2Environment> {
        match self.browser.env {
            EnvState::Ready => return ENV.with(|e| e.borrow().clone()),
            EnvState::Starting => return None,
            EnvState::NotStarted => {}
        }
        self.browser.env = EnvState::Starting;
        let dir = self.args.data_dir.join("WebView2-browser");
        let result = unsafe {
            {
                let options = CoreWebView2EnvironmentOptions::default();
                options.set_additional_browser_arguments(browser_args());
                let options: ICoreWebView2EnvironmentOptions = options.into();
                CreateCoreWebView2EnvironmentWithOptions(
                    PCWSTR::null(),
                    &HSTRING::from(dir.as_os_str()),
                    &options,
                    &CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(|err, env| {
                        match (err, env) {
                            (Ok(()), Some(env)) => {
                                ENV.with(|e| *e.borrow_mut() = Some(env));
                                later(|c| c.browser_env_ready());
                            }
                            (err, _) => {
                                log!("browser environment could not start: {err:?}");
                                later(|c| c.browser.env = EnvState::NotStarted);
                            }
                        }
                        Ok(())
                    })),
                )
            }
        };
        if let Err(err) = result {
            log!("browser environment: {err}");
            self.browser.env = EnvState::NotStarted;
        }
        None
    }

    fn browser_env_ready(&mut self) {
        self.browser.env = EnvState::Ready;
        if let Some(env5) = ENV.with(|e| e.borrow().clone()).and_then(|e| e.cast::<ICoreWebView2Environment5>().ok()) {
            let mut token = 0i64;
            unsafe {
                let _ = env5.add_BrowserProcessExited(
                    &BrowserProcessExitedEventHandler::create(Box::new(|_, _| {
                        later(|c| c.browser_process_gone());
                        Ok(())
                    })),
                    &mut token,
                );
            }
        }
        log!("browser environment ready");
        // the tabs that waited for it (the open one, a page's new window)
        let mut waiting: Vec<u32> = self.browser.pending_nav.keys().copied().collect();
        waiting.extend(NEW_WINDOWS.with(|n| n.borrow().keys().copied().collect::<Vec<_>>()));
        let current = self.browser.tabs.current().clone();
        if !current.is_start() {
            waiting.push(current.id);
        }
        waiting.sort_unstable();
        waiting.dedup();
        for id in waiting {
            self.browser_view(id);
        }
    }

    /// The Browser's process ended (killed, crashed, a runtime update): its views are gone, its
    /// tabs keep their addresses, and the open one comes back when it is shown again.
    fn browser_process_gone(&mut self) {
        log!("the Browser's process stopped");
        for (_, v) in self.browser.views.drain() {
            unsafe {
                let _ = v.controller.Close();
            }
        }
        self.browser.creating.clear();
        ENV.with(|e| *e.borrow_mut() = None);
        PID.store(0, Ordering::Relaxed);
        self.browser.env = EnvState::NotStarted;
        self.layout_views();
        self.browser_emit();
    }

    // -----------------------------------------------------------------------------------------
    // Views
    // -----------------------------------------------------------------------------------------
    /// Make the view for a Browser tab (its page loads when it's made: pending_nav or its url).
    fn browser_view(&mut self, id: u32) {
        if self.browser.views.contains_key(&id) || self.browser.creating.contains(&id) {
            return;
        }
        let Some(env) = self.browser_env() else { return }; // browser_env_ready comes back for it
        self.browser.creating.insert(id);
        let dark = self.chats.dark();
        let result = unsafe {
            (|| -> windows::core::Result<()> {
                let env10: ICoreWebView2Environment10 = env.cast()?;
                let opts = env10.CreateCoreWebView2ControllerOptions()?;
                opts.SetProfileName(&HSTRING::from("browser"))?;
                if let Ok(o3) = opts.cast::<ICoreWebView2ControllerOptions3>() {
                    let _ = o3.SetDefaultBackgroundColor(color(dark));
                }
                env10.CreateCoreWebView2ControllerWithOptions(
                    win32::h(self.panel.hwnd),
                    &opts,
                    &CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |err, ctrl| {
                        match (err, ctrl) {
                            (Ok(()), Some(ctrl)) => {
                                MADE.with(|m| m.borrow_mut().insert(id, ctrl));
                                later(move |c| c.browser_view_made(id));
                            }
                            (err, _) => {
                                log!("browser view {id} could not be made: {err:?}");
                                later(move |c| {
                                    c.browser.creating.remove(&id);
                                    c.browser_window_failed(id);
                                });
                            }
                        }
                        Ok(())
                    })),
                )
            })()
        };
        if let Err(err) = result {
            log!("browser view {id}: {err}");
            self.browser.creating.remove(&id);
            self.browser_window_failed(id);
        }
    }

    fn browser_view_made(&mut self, id: u32) {
        self.browser.creating.remove(&id);
        let Some(controller) = MADE.with(|m| m.borrow_mut().remove(&id)) else { return };
        let close = |c: &ICoreWebView2Controller| unsafe {
            let _ = c.Close();
        };
        // switched off, or the tab closed meanwhile
        if !self.browser_on() || self.browser.tabs.get(id).is_none() {
            close(&controller);
            self.browser_window_failed(id);
            return;
        }
        let Ok(webview) = (unsafe { controller.CoreWebView2() }) else {
            close(&controller);
            self.browser_window_failed(id);
            return;
        };
        if let Err(err) = unsafe { configure(&controller, &webview, id, self.args.dev_tools()) } {
            // without all of its handlers (navigation, windows, downloads) a page mustn't run
            log!("browser view {id} setup: {err}");
            close(&controller);
            self.browser_window_failed(id);
            return;
        }
        unsafe {
            let _ = controller.SetIsVisible(false);
        }
        self.browser.views.insert(
            id,
            BView {
                controller,
                webview: webview.clone(),
                visible: false,
                low_memory: false,
                playing: false,
                loading: false,
                can_back: false,
                can_forward: false,
                icon: String::new(),
            },
        );
        // a page's new window: this view becomes it (the opener's window.opener keeps working)
        if let Some((args, deferral)) = NEW_WINDOWS.with(|n| n.borrow_mut().remove(&id)) {
            unsafe {
                let _ = args.SetNewWindow(&webview);
                let _ = deferral.Complete();
            }
        } else {
            let url = self.browser.pending_nav.remove(&id).or_else(|| self.browser.tabs.get(id).map(|t| t.url.clone()));
            if let Some(url) = url.filter(|u| search::web_url(u)) {
                unsafe {
                    let _ = webview.Navigate(&HSTRING::from(url));
                }
            }
        }
        self.layout_views();
        self.browser_emit();
    }

    /// A view that couldn't be made: a page's new window waiting for it is refused, and its tab goes.
    fn browser_window_failed(&mut self, id: u32) {
        if let Some((args, deferral)) = NEW_WINDOWS.with(|n| n.borrow_mut().remove(&id)) {
            unsafe {
                let _ = args.SetHandled(true);
                let _ = deferral.Complete();
            }
            self.browser.tabs.close(id, rt::epoch_ms());
            self.browser_save();
            self.browser_emit();
        }
    }

    fn browser_drop_view(&mut self, id: u32) {
        if let Some(v) = self.browser.views.remove(&id) {
            unsafe {
                let _ = v.controller.Close();
            }
        }
    }

    pub fn browser_close_all(&mut self) {
        let ids: Vec<u32> = self.browser.views.keys().copied().collect();
        for id in ids {
            self.browser_drop_view(id);
        }
        for id in NEW_WINDOWS.with(|n| n.borrow().keys().copied().collect::<Vec<_>>()) {
            self.browser_window_failed(id);
        }
        self.browser.pending_nav.clear();
        self.browser.notice = Value::Null;
    }

    // -----------------------------------------------------------------------------------------
    // Going places
    // -----------------------------------------------------------------------------------------
    /// Typed text from the start page or the address field: open an address, or search.
    pub fn browser_go(&mut self, text: &str) {
        match search::classify(text) {
            None => {}
            Some(search::Input::Open(url)) => {
                if search::blocked(&url) {
                    self.browser_notice(json!({ "kind": "local" }));
                } else {
                    self.browser_open(&url, false);
                }
            }
            Some(search::Input::Search(q)) => {
                let engine = self.settings.get(ID)["engine"].as_str().unwrap_or("google").to_string();
                self.browser_remember(&q, &engine);
                let url = search::search_url(&engine, &q, &self.lang);
                self.browser_open(&url, false);
            }
        }
    }

    /// A web page in the open tab (or a new tab next to it).
    pub fn browser_open(&mut self, url: &str, new_tab: bool) {
        if !search::web_url(url) || search::blocked(url) || url.len() > search::URL_MAX {
            return;
        }
        let now = rt::epoch_ms();
        let id = if new_tab {
            match self.browser.tabs.add(url, now) {
                Some(id) => id,
                None => return, // 10 open: the page's "+" says so
            }
        } else {
            let id = self.browser.tabs.current().id;
            let title = self.browser.tabs.current().title.clone();
            self.browser.tabs.set_page(id, url, &title);
            id
        };
        self.browser.notice = Value::Null;
        match self.browser.views.get(&id) {
            Some(v) => unsafe {
                let _ = v.webview.Navigate(&HSTRING::from(url));
            },
            None => {
                self.browser.pending_nav.insert(id, url.to_string());
                self.browser_view(id);
            }
        }
        self.browser_save();
        self.layout_views();
        self.browser_emit();
    }

    pub fn browser_new(&mut self) {
        if self.browser.tabs.add("", rt::epoch_ms()).is_some() {
            self.browser.notice = Value::Null;
            self.browser_save();
            self.layout_views();
            self.browser_emit();
            self.emit("panel", "browser:focus-search", json!([]));
        }
    }

    pub fn browser_select(&mut self, id: u32) {
        if !self.browser.tabs.select(id, rt::epoch_ms()) {
            return;
        }
        self.browser.notice = Value::Null;
        self.browser_save();
        self.layout_views(); // (a dropped or restored tab's page is made there)
        self.browser_emit();
    }

    pub fn browser_close(&mut self, id: u32) {
        self.browser_drop_view(id);
        self.browser.pending_nav.remove(&id);
        self.browser.tabs.close(id, rt::epoch_ms());
        self.browser.notice = Value::Null;
        self.browser_save();
        self.layout_views();
        self.browser_emit();
    }

    fn browser_current_view(&self) -> Option<&BView> {
        self.browser.views.get(&self.browser.tabs.current().id)
    }

    pub fn browser_back(&mut self) {
        if let Some(v) = self.browser_current_view() {
            unsafe {
                let _ = v.webview.GoBack();
            }
        }
    }

    pub fn browser_forward(&mut self) {
        if let Some(v) = self.browser_current_view() {
            unsafe {
                let _ = v.webview.GoForward();
            }
        }
    }

    pub fn browser_reload(&mut self) {
        let current = self.browser.tabs.current().clone();
        match self.browser.views.get(&current.id) {
            Some(v) => unsafe {
                let _ = v.webview.Reload();
            },
            None if !current.is_start() => self.browser_view(current.id),
            None => {}
        }
    }

    /// Home: this tab shows the start page again (its page is dropped).
    pub fn browser_home(&mut self) {
        let id = self.browser.tabs.current().id;
        self.browser_drop_view(id);
        self.browser.pending_nav.remove(&id);
        self.browser.tabs.set_page(id, "", "");
        self.browser.notice = Value::Null;
        self.browser_save();
        self.layout_views();
        self.browser_emit();
        self.emit("panel", "browser:focus-search", json!([]));
    }

    /// "Open in the computer's browser": the open page (or a refused download's address).
    pub fn browser_outside(&mut self, url: Option<String>) {
        let url = url.unwrap_or_else(|| self.browser.tabs.current().url.clone());
        if search::web_url(&url) && !search::blocked(&url) && url.len() <= search::URL_MAX {
            win32::open_url(&url);
        }
    }

    fn browser_loading(&mut self, id: u32, on: bool) {
        if let Some(v) = self.browser.views.get_mut(&id) {
            v.loading = on;
        }
        self.browser_emit();
    }

    fn browser_source(&mut self, id: u32, url: &str) {
        if search::web_url(url) {
            let title = self.browser.tabs.get(id).map(|t| t.title.clone()).unwrap_or_default();
            self.browser.tabs.set_page(id, url, &title);
            self.browser_save();
            self.browser_emit();
        }
    }

    /// A page tried to go where the Browser doesn't: this PC / the LAN (said so), or a program
    /// link (asked about when clicked; never opened without a click).
    fn browser_refused(&mut self, id: u32, uri: &str, clicked: bool) {
        if search::web_url(uri) {
            if id == self.browser.tabs.current().id {
                self.browser_notice(json!({ "kind": "local" }));
            }
            return;
        }
        if clicked && apps::app_scheme(uri) && !self.args.selftest {
            let scheme = uri.split(':').next().unwrap_or("").chars().take(30).collect::<String>();
            self.browser_notice(json!({ "kind": "program", "scheme": scheme, "url": uri }));
        }
    }

    pub fn browser_notice(&mut self, n: Value) {
        self.browser.notice = n;
        self.layout_views(); // the bar pushes the page down
        self.browser_emit();
    }

    fn browser_new_window(&mut self, (args, deferral): (ICoreWebView2NewWindowRequestedEventArgs, ICoreWebView2Deferral), uri: &str) {
        let url = if uri == "about:blank" { "" } else { uri };
        let Some(id) = self.browser.tabs.add(url, rt::epoch_ms()) else {
            unsafe {
                let _ = args.SetHandled(true);
                let _ = deferral.Complete();
            }
            log!("browser: a window refused (10 tabs open)");
            return;
        };
        NEW_WINDOWS.with(|n| n.borrow_mut().insert(id, (args, deferral)));
        self.browser.notice = Value::Null;
        self.browser_view(id);
        self.browser_save();
        self.layout_views();
        self.browser_emit();
    }

    fn browser_page_crashed(&mut self, id: u32) {
        let now = rt::epoch_ms();
        let recent = CRASHES.with(|m| {
            let mut m = m.borrow_mut();
            let list = m.entry(id).or_default();
            list.retain(|t| now - t < 120_000);
            list.push(now);
            list.len()
        });
        if recent >= 3 {
            log!("browser tab {id} crashed {recent} times: its page is dropped until clicked");
            self.browser_drop_view(id);
            self.layout_views();
        } else {
            timer(1500, move |c| {
                if let Some(v) = c.browser.views.get(&id) {
                    unsafe {
                        let _ = v.webview.Reload();
                    }
                }
            });
        }
        self.browser_emit();
    }

    fn browser_remember(&mut self, q: &str, engine: &str) {
        let mut b = self.settings.get(ID).clone();
        if b["remember"] != json!(true) {
            return;
        }
        let Some(entry) = crate::settings::recent_ok(&json!({ "q": q.trim(), "e": engine })) else { return };
        let mut list: Vec<Value> = b["recent"].as_array().cloned().unwrap_or_default();
        list.retain(|r| r["q"] != entry["q"]);
        list.insert(0, entry);
        list.truncate(crate::settings::BROWSER_RECENT_MAX);
        b["recent"] = Value::Array(list);
        self.settings.set(ID, b);
        self.save_soon();
    }

    // -----------------------------------------------------------------------------------------
    // Layout, focus, memory
    // -----------------------------------------------------------------------------------------
    /// The Browser's pages go in `area` (under its bar); only the open tab's, only when it shows a
    /// web page and `show` (the Browser tab on screen, no Settings / welcome over it).
    pub fn layout_browser(&mut self, area: Rect, show: bool) {
        let scale = self.target_display().ui;
        let bar = ((BAR_H + if self.browser.notice.is_null() { 0.0 } else { NOTICE_H }) * scale).round() as i32;
        let r = Rect { x: area.x, y: area.y + bar, w: area.w, h: (area.h - bar).max(1) };
        let current = self.browser.tabs.current().clone();
        if show && !current.is_start() && !self.browser.views.contains_key(&current.id) {
            self.browser_view(current.id); // after a restart, a crash, or a drop
        }
        for (id, v) in self.browser.views.iter_mut() {
            let on = show && *id == current.id && !current.is_start();
            unsafe {
                let _ = v.controller.SetBounds(RECT { left: r.x, top: r.y, right: r.x + r.w, bottom: r.y + r.h });
                if v.visible != on {
                    v.visible = on;
                    let _ = v.controller.SetIsVisible(on);
                }
            }
        }
        self.browser_memory();
    }

    /// Hidden pages give memory back, unless they play sound.
    fn browser_memory(&mut self) {
        for v in self.browser.views.values_mut() {
            let low = !v.visible && !v.playing;
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

    /// Keyboard into the open page. False when none is on screen.
    pub fn browser_focus(&self) -> bool {
        match self.browser_current_view() {
            Some(v) if v.visible => unsafe { v.controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC).is_ok() },
            _ => false,
        }
    }

    /// Tabs unused for 30 minutes drop their page (never the open one, or one playing sound).
    pub fn browser_drop_idle(&mut self) {
        let playing: HashSet<u32> = self.browser.views.iter().filter(|(_, v)| v.playing).map(|(id, _)| *id).collect();
        for id in self.browser.tabs.idle(rt::epoch_ms(), |id| playing.contains(&id)) {
            if self.browser.views.contains_key(&id) {
                log!("browser tab {id} unused for 30 min: page dropped");
                self.browser_drop_view(id);
            }
        }
    }
}

/// What the panel page draws: the tabs, the open one's address and history, the start page's
/// settings and any notice. `view(id)` = (icon, loading, can_back, can_forward) of a made view.
pub fn state_json(tabs: &Tabs, view: &dyn Fn(u32) -> Option<(String, bool, bool, bool)>, saved: &Value, notice: &Value) -> Value {
    let list: Vec<Value> = tabs
        .list()
        .iter()
        .map(|t| {
            let (icon, loading, _, _) = view(t.id).unwrap_or_default();
            json!({ "id": t.id, "title": t.title, "url": t.url, "icon": icon, "loading": loading })
        })
        .collect();
    let cur = tabs.current();
    let (_, _, back, fwd) = view(cur.id).unwrap_or_default();
    json!({
        "tabs": list,
        "open": cur.id,
        "url": cur.url,
        "start": cur.is_start(),
        "canBack": back,
        "canForward": fwd,
        "full": tabs.full(),
        "engine": saved.get("engine").and_then(Value::as_str).unwrap_or("google"),
        "suggest": saved.get("suggest").and_then(Value::as_bool).unwrap_or(true),
        "remember": saved.get("remember").and_then(Value::as_bool).unwrap_or(true),
        "recent": saved.get("recent").cloned().unwrap_or_else(|| json!([])),
        "shortcuts": saved.get("shortcuts").cloned().unwrap_or_else(|| json!([])),
        "notice": notice,
    })
}

impl Core {
    pub fn browser_emit(&mut self) {
        if !self.ready.contains("panel") || !self.browser_on() {
            return;
        }
        let views = &self.browser.views;
        let view = |id: u32| views.get(&id).map(|v| (v.icon.clone(), v.loading, v.can_back, v.can_forward));
        let mut s = state_json(&self.browser.tabs, &view, self.settings.get(ID), &self.browser.notice);
        s["lang"] = json!(self.lang);
        self.emit("panel", "browser:state", json!([s]));
    }

    /// A message from the panel page's Browser (ui/browser.js).
    pub fn browser_ui(&mut self, channel: &str, args: &[Value]) {
        if !self.browser_on() {
            return;
        }
        let s = |i: usize| args.get(i).and_then(Value::as_str).unwrap_or("").to_string();
        let n = |i: usize| args.get(i).and_then(Value::as_i64).unwrap_or(-1);
        let id = |i: usize| args.get(i).and_then(Value::as_u64).map(|v| v as u32).unwrap_or(0);
        match channel {
            "browser:go" => self.browser_go(&s(0)),
            "browser:open" => {
                let new_tab = args.get(1).and_then(Value::as_bool).unwrap_or(false);
                self.browser_open(&s(0), new_tab)
            }
            "browser:tab" => self.browser_select(id(0)),
            "browser:close" => self.browser_close(id(0)),
            "browser:new" => self.browser_new(),
            "browser:back" => self.browser_back(),
            "browser:forward" => self.browser_forward(),
            "browser:reload" => self.browser_reload(),
            "browser:home" => self.browser_home(),
            "browser:outside" => self.browser_outside(Some(s(0)).filter(|u| !u.is_empty())),
            "browser:suggest" => self.browser_suggest(&s(0)), // Task 8; empty fn until then
            "browser:engine" => self.browser_set("engine", json!(s(0))),
            "browser:recent-remove" => self.browser_edit_list("recent", n(0), None),
            "browser:recent-clear" => self.browser_set("recent", json!([])),
            "browser:shortcut" => {
                let i = n(0);
                match search::classify(&s(2)) {
                    Some(search::Input::Open(u)) if !search::blocked(&u) => {
                        let entry = json!({ "n": s(1), "u": u });
                        if crate::settings::shortcut_ok(&entry).is_some() {
                            self.browser_edit_list("shortcuts", i, Some(entry));
                        } else {
                            self.emit("panel", "browser:shortcut-bad", json!([]));
                        }
                    }
                    _ => self.emit("panel", "browser:shortcut-bad", json!([])),
                }
            }
            "browser:shortcut-delete" => self.browser_edit_list("shortcuts", n(0), None),
            "browser:notice-close" => self.browser_notice(Value::Null),
            "browser:program-open" => {
                let url = self.browser.notice["url"].as_str().unwrap_or("").to_string();
                if self.browser.notice["kind"] == "program" && crate::apps::app_scheme(&url) {
                    win32::open_url(&url);
                }
                self.browser_notice(Value::Null);
            }
            _ => {}
        }
    }

    /// One Browser setting the page changed (engine, recent cleared, the two switches).
    pub fn browser_set(&mut self, key: &str, v: Value) {
        let mut b = self.settings.get(ID).clone();
        b[key] = v;
        if key == "remember" && b["remember"] == json!(false) {
            b["recent"] = json!([]); // off also forgets them
        }
        let clean = crate::settings::clean_browser(Some(&b));
        self.settings.set(ID, clean);
        self.save_soon();
        self.browser_emit();
    }

    /// Replace (i ≥ 0 with Some), add (i = -1 with Some, at the end) or delete (None) an entry of
    /// `recent` or `shortcuts`; clean_browser keeps the limits.
    fn browser_edit_list(&mut self, key: &str, i: i64, entry: Option<Value>) {
        let mut list: Vec<Value> = self.settings.get(ID)[key].as_array().cloned().unwrap_or_default();
        match (i, entry) {
            (-1, Some(e)) => list.push(e),
            (i, Some(e)) if i >= 0 && (i as usize) < list.len() => list[i as usize] = e,
            (i, None) if i >= 0 && (i as usize) < list.len() => {
                list.remove(i as usize);
            }
            _ => return,
        }
        self.browser_set(key, Value::Array(list));
    }
}

/// The suggestion address for typed text, or None: switched off, empty, an address (never sent
/// anywhere), or too long.
pub fn suggest_request(saved: &Value, text: &str, lang: &str) -> Option<String> {
    if saved.get("suggest").and_then(Value::as_bool) != Some(true) || text.trim().chars().count() > 200 {
        return None;
    }
    match search::classify(text)? {
        search::Input::Search(q) => Some(search::suggest_url(saved["engine"].as_str().unwrap_or("google"), &q, lang)),
        search::Input::Open(_) => None,
    }
}

impl Core {
    pub fn browser_suggest(&mut self, text: &str) {
        let text: String = text.chars().take(200).collect();
        if self.browser.suggest_busy {
            self.browser.suggest_next = Some(text); // asked when the one in flight answers
            return;
        }
        let Some(url) = suggest_request(self.settings.get(ID), &text, &self.lang) else {
            self.emit("panel", "browser:suggest", json!([{ "text": text, "items": [] }]));
            return;
        };
        self.browser.suggest_busy = true;
        let started = std::thread::Builder::new().name("chatdock-suggest".into()).spawn(move || {
            let items = crate::suggest::get(&url, 2000).map(|b| search::parse_suggestions(&b)).unwrap_or_default();
            later(move |c| {
                c.browser.suggest_busy = false;
                c.emit("panel", "browser:suggest", json!([{ "text": text, "items": items }]));
                if let Some(next) = c.browser.suggest_next.take() {
                    c.browser_suggest(&next);
                }
            });
        });
        if started.is_err() {
            self.browser.suggest_busy = false;
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum KeyAction {
    NewTab,
    CloseTab,
    Address,
    Back,
    Forward,
    Reload,
    Step(i32),
}

/// A key while the Browser tab is open (others go on to ChatDock's usual keys).
pub fn browser_key_action(vk: u32, ctrl: bool, alt: bool, _shift: bool) -> Option<KeyAction> {
    Some(match (vk, ctrl, alt) {
        (0x54, true, false) => KeyAction::NewTab,
        (0x57, true, false) => KeyAction::CloseTab,
        (0x4C, true, false) => KeyAction::Address,
        (0x25, false, true) => KeyAction::Back,
        (0x27, false, true) => KeyAction::Forward,
        (0x52, true, false) | (0x74, _, false) => KeyAction::Reload,
        (0x21, true, false) => KeyAction::Step(-1),
        (0x22, true, false) => KeyAction::Step(1),
        _ => return None,
    })
}

impl Core {
    /// True when the key was the Browser's.
    pub fn browser_key(&mut self, vk: u32, ctrl: bool, alt: bool, shift: bool) -> bool {
        let Some(a) = browser_key_action(vk, ctrl, alt, shift) else { return false };
        match a {
            KeyAction::NewTab => self.browser_new(),
            KeyAction::CloseTab => {
                let id = self.browser.tabs.current().id;
                self.browser_close(id);
            }
            KeyAction::Address => {
                crate::chats::focus_panel_page();
                self.emit("panel", "browser:focus-address", json!([]));
            }
            KeyAction::Back => self.browser_back(),
            KeyAction::Forward => self.browser_forward(),
            KeyAction::Reload => self.browser_reload(),
            KeyAction::Step(d) => {
                self.browser.tabs.step(d, rt::epoch_ms());
                let id = self.browser.tabs.current().id;
                self.browser_select(id);
            }
        }
        true
    }
}

/// The Browser's rules for one of its pages (not the chats': any site, its own pop-up rule, no
/// page script, nothing on this PC or the LAN).
unsafe fn configure(controller: &ICoreWebView2Controller, wv: &ICoreWebView2, id: u32, debug: bool) -> windows::core::Result<()> {
    let mut pid = 0u32;
    if wv.BrowserProcessId(&mut pid).is_ok() && pid != 0 {
        PID.store(pid, Ordering::Relaxed);
    }
    apply_settings(wv, debug, false)?; // the browser's own error pages: any site can fail to load
    if let Ok(s8) = wv.Settings()?.cast::<ICoreWebView2Settings8>() {
        let _ = s8.SetIsReputationCheckingRequired(true); // SmartScreen on (apply_settings turned it off)
    }
    refuse_local(wv)?;
    let mut token = 0i64;

    // top-level navigation: any web address but this PC or the LAN; other schemes ask first
    wv.add_NavigationStarting(
        &NavigationStartingEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut p = PWSTR::null();
            args.Uri(&mut p)?;
            let uri = take_pwstr(p);
            if uri.starts_with("about:") {
                return Ok(());
            }
            if search::web_url(&uri) && !search::blocked(&uri) {
                later(move |c| c.browser_loading(id, true));
                return Ok(());
            }
            args.SetCancel(true)?;
            let mut clicked = BOOL(0);
            let _ = args.IsUserInitiated(&mut clicked);
            let clicked = clicked.as_bool();
            later(move |c| c.browser_refused(id, &uri, clicked));
            Ok(())
        })),
        &mut token,
    )?;

    wv.add_NavigationCompleted(
        &NavigationCompletedEventHandler::create(Box::new(move |_, _| {
            later(move |c| c.browser_loading(id, false));
            Ok(())
        })),
        &mut token,
    )?;

    wv.add_SourceChanged(
        &SourceChangedEventHandler::create(Box::new(move |sender, _| {
            if let Some(sender) = sender {
                let mut p = PWSTR::null();
                sender.Source(&mut p)?;
                let url = take_pwstr(p);
                later(move |c| c.browser_source(id, &url));
            }
            Ok(())
        })),
        &mut token,
    )?;

    wv.add_DocumentTitleChanged(
        &DocumentTitleChangedEventHandler::create(Box::new(move |sender, _| {
            if let Some(sender) = sender {
                let mut p = PWSTR::null();
                sender.DocumentTitle(&mut p)?;
                let title = take_pwstr(p);
                later(move |c| {
                    c.browser.tabs.set_title(id, &title);
                    c.browser_save();
                    c.browser_emit();
                });
            }
            Ok(())
        })),
        &mut token,
    )?;

    wv.add_HistoryChanged(
        &HistoryChangedEventHandler::create(Box::new(move |sender, _| {
            if let Some(sender) = sender {
                let (mut b, mut f) = (BOOL(0), BOOL(0));
                let _ = sender.CanGoBack(&mut b);
                let _ = sender.CanGoForward(&mut f);
                let (b, f) = (b.as_bool(), f.as_bool());
                later(move |c| {
                    if let Some(v) = c.browser.views.get_mut(&id) {
                        v.can_back = b;
                        v.can_forward = f;
                    }
                    c.browser_emit();
                });
            }
            Ok(())
        })),
        &mut token,
    )?;

    if let Ok(wv15) = wv.cast::<ICoreWebView2_15>() {
        wv15.add_FaviconChanged(
            &FaviconChangedEventHandler::create(Box::new(move |sender, _| {
                if let Some(s15) = sender.and_then(|s| s.cast::<ICoreWebView2_15>().ok()) {
                    let mut p = PWSTR::null();
                    s15.FaviconUri(&mut p)?;
                    let icon = take_pwstr(p);
                    // https only: the panel page shows it as a picture
                    let icon = if icon.starts_with("https://") && icon.len() <= search::URL_MAX { icon } else { String::new() };
                    later(move |c| {
                        if let Some(v) = c.browser.views.get_mut(&id) {
                            v.icon = icon;
                        }
                        c.browser_emit();
                    });
                }
                Ok(())
            })),
            &mut token,
        )?;
    }

    // new windows after a click: a new Browser tab (handed the window, so sign-in pop-ups work)
    wv.add_NewWindowRequested(
        &NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut p = PWSTR::null();
            args.Uri(&mut p)?;
            let uri = take_pwstr(p);
            let mut clicked = BOOL(0);
            let _ = args.IsUserInitiated(&mut clicked);
            if !clicked.as_bool() || !(uri == "about:blank" || (search::web_url(&uri) && !search::blocked(&uri))) {
                args.SetHandled(true)?;
                later(move |_| log!("browser: a window not opened (no click, or not a web page)"));
                return Ok(());
            }
            let deferral = args.GetDeferral()?;
            let held = UiOnly((args.clone(), deferral));
            later(move |c| c.browser_new_window(held.take(), &uri));
            Ok(())
        })),
        &mut token,
    )?;

    // permissions: none in 1.8.2 (no prompt UI yet)
    wv.add_PermissionRequested(
        &PermissionRequestedEventHandler::create(Box::new(|_, args| {
            if let Some(args) = args {
                args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
            }
            Ok(())
        })),
        &mut token,
    )?;

    // downloads: refused, with a bar offering the computer's browser
    if let Ok(wv4) = wv.cast::<ICoreWebView2_4>() {
        wv4.add_DownloadStarting(
            &DownloadStartingEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                args.SetCancel(true)?;
                args.SetHandled(true)?;
                let (mut u, mut f) = (PWSTR::null(), PWSTR::null());
                if let Ok(op) = args.DownloadOperation() {
                    let _ = op.Uri(&mut u);
                }
                let _ = args.ResultFilePath(&mut f);
                let (url, path) = (take_pwstr(u), take_pwstr(f));
                let name: String = std::path::Path::new(&path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
                    .chars()
                    .take(80)
                    .collect();
                let url = if search::web_url(&url) && url.len() <= search::URL_MAX { url } else { String::new() };
                later(move |c| c.browser_notice(json!({ "kind": "download", "name": name, "url": url })));
                Ok(())
            })),
            &mut token,
        )?;
    }

    // a frame's program link (steam://, mailto:): asked about, never opened by itself
    if let Ok(wv18) = wv.cast::<ICoreWebView2_18>() {
        let _ = wv18.add_LaunchingExternalUriScheme(
            &LaunchingExternalUriSchemeEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                args.SetCancel(true)?;
                let mut p = PWSTR::null();
                args.Uri(&mut p)?;
                let uri = take_pwstr(p);
                let mut clicked = BOOL(0);
                let _ = args.IsUserInitiated(&mut clicked);
                let clicked = clicked.as_bool();
                later(move |c| c.browser_refused(id, &uri, clicked));
                Ok(())
            })),
            &mut token,
        );
    }

    if let Ok(wv8) = wv.cast::<ICoreWebView2_8>() {
        wv8.add_IsDocumentPlayingAudioChanged(
            &IsDocumentPlayingAudioChangedEventHandler::create(Box::new(move |sender, _| {
                if let Some(s8) = sender.and_then(|s| s.cast::<ICoreWebView2_8>().ok()) {
                    let mut on = BOOL(0);
                    let _ = s8.IsDocumentPlayingAudio(&mut on);
                    let on = on.as_bool();
                    later(move |c| {
                        if let Some(v) = c.browser.views.get_mut(&id) {
                            v.playing = on;
                        }
                        c.browser_memory();
                    });
                }
                Ok(())
            })),
            &mut token,
        )?;
    }

    wv.add_ProcessFailed(
        &ProcessFailedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
            args.ProcessFailedKind(&mut kind)?;
            if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED {
                later(move |c| c.browser_page_crashed(id));
            }
            Ok(())
        })),
        &mut token,
    )?;

    own_page_menu(wv, id);
    crate::chats::hook_keys(controller, Some(ID));
    Ok(())
}

unsafe fn own_page_menu(_wv: &ICoreWebView2, _id: u32) {} // Task 10

/// The same rule as chats::block_this_pc, plus the LAN (any site may try to reach a router): the
/// filters catch every way these can be written, the handler decides by the parsed host.
unsafe fn refuse_local(wv: &ICoreWebView2) -> windows::core::Result<()> {
    let wv22 = wv.cast::<ICoreWebView2_22>().ok();
    let filters = [
        "*://127.*",
        "*://localhost*",
        "*://*.localhost*",
        "*://[::1]*",
        "*://[::]*",
        "*://0.0.0.0*",
        "*://[::ffff:*",
        "*://10.*",
        "*://192.168.*",
        "*://172.*",
        "*://169.254.*",
        "*://[f*",
        "http://*", // a plain http name may point at this PC: looked up
    ];
    for f in filters {
        match &wv22 {
            Some(w) => w.AddWebResourceRequestedFilterWithRequestSourceKinds(
                &HSTRING::from(f),
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
                COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
            )?,
            None => wv.AddWebResourceRequestedFilter(&HSTRING::from(f), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?,
        }
    }
    let mut token = 0i64;
    wv.add_WebResourceRequested(
        &WebResourceRequestedEventHandler::create(Box::new(|_, args| {
            let (Some(args), Some(env)) = (args, ENV.with(|e| e.borrow().clone())) else { return Ok(()) };
            let mut p = PWSTR::null();
            args.Request()?.Uri(&mut p)?;
            let uri = take_pwstr(p);
            if search::blocked(&uri) {
                return refuse(&env, &args);
            }
            let host = match apps::http_check(&uri) {
                apps::HttpCheck::Allow => return Ok(()),
                apps::HttpCheck::Block => return refuse(&env, &args),
                apps::HttpCheck::Resolve(host) => host,
            };
            match apps::cached_local(&host) {
                Some(true) => return refuse(&env, &args),
                Some(false) => return Ok(()),
                None => {}
            }
            // looked up off the UI thread; the request waits for the answer
            let deferral = args.GetDeferral()?;
            let held = UiOnly((env, args.clone(), deferral));
            let started = std::thread::Builder::new().name("chatdock-lookup".into()).spawn(move || {
                let local = apps::resolves_local(&host);
                rt::post(move || {
                    let (env, args, deferral) = held.take();
                    if local {
                        let _ = refuse(&env, &args);
                        log!("browser: a request to {host} (this PC) refused");
                    }
                    let _ = deferral.Complete();
                });
            });
            if started.is_err() {
                log!("browser lookup: no thread");
            }
            Ok(())
        })),
        &mut token,
    )?;
    Ok(())
}

fn refuse(env: &ICoreWebView2Environment, args: &ICoreWebView2WebResourceRequestedEventArgs) -> windows::core::Result<()> {
    let resp = unsafe { env.CreateWebResourceResponse(None, 403, &HSTRING::from("Blocked"), &HSTRING::from(""))? };
    unsafe { args.SetResponse(&resp)? };
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_state_tells_the_page_what_to_draw() {
        let tabs = crate::browser_tabs::Tabs::restore(
            &serde_json::json!([{ "url": "https://a.com/", "title": "A" }, { "url": "", "title": "" }]),
            1,
            0,
        );
        let views =
            |id: u32| if id == tabs.list()[0].id { Some(("https://a.com/favicon.ico".to_string(), true, true, false)) } else { None };
        let s = super::state_json(&tabs, &views, &serde_json::json!({ "engine": "ddg", "suggest": false }), &serde_json::Value::Null);
        assert_eq!(s["open"], tabs.list()[1].id);
        assert_eq!(s["start"], true);
        assert_eq!(s["tabs"][0]["icon"], "https://a.com/favicon.ico");
        assert_eq!(s["tabs"][0]["loading"], true);
        assert_eq!(s["tabs"][1]["title"], "");
        assert_eq!(s["canBack"], false); // the open tab (the start page) has no view
        assert_eq!((s["engine"].as_str(), s["suggest"].as_bool(), s["full"].as_bool()), (Some("ddg"), Some(false), Some(false)));
    }

    #[test]
    fn suggestions_are_asked_only_when_they_may_be() {
        let on = serde_json::json!({ "engine": "bing", "suggest": true });
        let off = serde_json::json!({ "engine": "bing", "suggest": false });
        assert_eq!(super::suggest_request(&on, "elden", "en").as_deref(), Some("https://api.bing.com/osjson.aspx?query=elden"));
        assert_eq!(super::suggest_request(&off, "elden", "en"), None);
        assert_eq!(super::suggest_request(&on, "   ", "en"), None);
        assert_eq!(super::suggest_request(&on, "youtube.com", "en"), None); // an address goes nowhere
        assert_eq!(super::suggest_request(&on, &"a".repeat(201), "en"), None);
    }

    #[test]
    fn browser_keys_only_while_the_browser_is_open() {
        use super::{browser_key_action as k, KeyAction::*};
        assert_eq!(k(0x54, true, false, false), Some(NewTab)); // Ctrl+T
        assert_eq!(k(0x57, true, false, false), Some(CloseTab)); // Ctrl+W
        assert_eq!(k(0x4C, true, false, false), Some(Address)); // Ctrl+L
        assert_eq!(k(0x25, false, true, false), Some(Back)); // Alt+Left
        assert_eq!(k(0x27, false, true, false), Some(Forward));
        assert_eq!(k(0x52, true, false, false), Some(Reload)); // Ctrl+R
        assert_eq!(k(0x74, false, false, false), Some(Reload)); // F5
        assert_eq!(k(0x21, true, false, false), Some(Step(-1))); // Ctrl+PgUp
        assert_eq!(k(0x22, true, false, false), Some(Step(1)));
        assert_eq!(k(0x09, true, false, false), None); // Ctrl+Tab stays ChatDock's
        assert_eq!(k(0x31, true, false, false), None); // Ctrl+1 too
        assert_eq!(k(0x54, true, true, false), None); // Ctrl+Alt+T: not ours
    }

    #[test]
    fn the_browser_comes_after_the_chats() {
        assert_eq!(super::tab_order(vec!["instagram", "discord"], true), ["instagram", "discord", "browser"]);
        assert_eq!(super::tab_order(vec!["instagram"], false), ["instagram"]);
    }
}
