//! The chat panel: its window (and all of ChatDock's other windows), where it goes, sliding in and
//! out, focus (taking it, and handing it straight back to the game), hiding when you click
//! elsewhere, the settings / welcome screens, resizing, and the keyboard shortcuts.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};
use tauri::{window::Color, WebviewUrl, WebviewWindowBuilder};
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Shell::{DefSubclassProc, SetWindowSubclass},
        WindowsAndMessaging::{
            SPI_SETWORKAREA, WA_INACTIVE, WM_ACTIVATE, WM_CLOSE, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ENDSESSION, WM_QUERYENDSESSION,
            WM_SETTINGCHANGE,
        },
    },
};

use crate::{
    apps,
    core::{self, later, timer, Args, Core, PanelState, Win},
    frames, log, rt,
    settings::Settings,
    win32::{self, Display, Rect},
};

// Layout, in DIP (keep in sync with --header-h / --banner-h / --grip-w in ui/panel.css)
pub const HEADER_H: f64 = 48.0;
pub const BANNER_H: f64 = 52.0;
pub const GRIP_W: f64 = 6.0;
pub const MIN_W: f64 = 340.0;
/// How often to look where the focus went while the screen-share bar has it.
const FOCUS_WATCH_MS: u64 = 150;
const OPEN_MS: f64 = 260.0;
const CLOSE_MS: f64 = 170.0;

fn ease_out_quint(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(5) // quick start, long soft landing
}

fn ease_in_cubic(t: f64) -> f64 {
    t * t * t
}

#[derive(Clone, Copy)]
pub enum AnimKind {
    SlideX {
        from: i32,
        to: i32,
    },
    /// no slide (another monitor right against the dock edge): fade in / out instead
    Fade {
        from: u8,
        to: u8,
    },
}

#[derive(Clone, Copy, PartialEq)]
pub enum AnimDone {
    Opened,
    Closed,
    Moved,
}

pub struct Anim {
    gen: u64,
    t0: Option<f64>,
    ms: f64,
    ease: fn(f64) -> f64,
    kind: AnimKind,
    done: AnimDone,
    last_x: i32,
}

thread_local! {
    static ANIM: std::cell::RefCell<Option<Anim>> = const { std::cell::RefCell::new(None) };
}

fn window(
    app: &tauri::App,
    label: &str,
    page: &str,
    args: &Args,
    transparent: bool,
    focusable: bool,
    bg: Option<Color>,
) -> tauri::Result<Win> {
    let mut b = WebviewWindowBuilder::new(app, label, WebviewUrl::App(page.into()))
        .title("ChatDock")
        .visible(false)
        .decorations(false)
        .resizable(false)
        .minimizable(false)
        .maximizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .shadow(false)
        .focused(false)
        .focusable(focusable)
        .transparent(transparent)
        .devtools(args.dev_tools())
        .zoom_hotkeys_enabled(false)
        .data_directory(args.webview_dir())
        .additional_browser_args(&args.browser_args())
        .inner_size(120.0, 120.0)
        .position(-30000.0, -30000.0);
    if let Some(c) = bg {
        b = b.background_color(c);
    }
    let w = b.build()?;
    let hwnd = w.hwnd()?.0 as isize;
    lock_down_page(&w, args.dev_tools());
    Ok(Win { w, hwnd })
}

/// One of ChatDock's own pages: no browser context menu (Back / Refresh / Print... on the tab or a
/// pop-up) and no browser keys (Ctrl+P, Ctrl+F, F5 on ChatDock's own pages); if the renderer that
/// draws them crashes, they are loaded again.
pub fn lock_down_page(w: &tauri::WebviewWindow, debug: bool) {
    use webview2_com::{Microsoft::Web::WebView2::Win32::*, ProcessFailedEventHandler};
    use windows::core::Interface;
    let _ = w.with_webview(move |pw| unsafe {
        let Ok(wv) = pw.controller().CoreWebView2() else { return };
        if let Ok(s) = wv.Settings() {
            let _ = s.SetAreDefaultContextMenusEnabled(debug);
            if let Ok(s3) = s.cast::<ICoreWebView2Settings3>() {
                let _ = s3.SetAreBrowserAcceleratorKeysEnabled(debug);
            }
        }
        let mut token = 0i64;
        let _ = wv.add_ProcessFailed(
            &ProcessFailedEventHandler::create(Box::new(|_, args| {
                let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                if let Some(args) = args {
                    let _ = args.ProcessFailedKind(&mut kind);
                }
                if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED {
                    later(|c| c.reload_ui_pages());
                } else if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED {
                    later(|c| c.restart_after_crash("the WebView2 browser process stopped"));
                }
                Ok(())
            })),
            &mut token,
        );
    });
}

// Window messages Tauri doesn't forward: the panel losing activation (click elsewhere), the tab or
// the pop-ups being clicked (focus has to go back to the game), displays and theme changing.
unsafe extern "system" fn subclass_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, which: usize) -> LRESULT {
    match msg {
        WM_ACTIVATE => {
            let inactive = (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE;
            match (which, inactive) {
                (1, true) => later(|c| c.on_panel_blur()),
                (1, false) => {
                    // Windows gives the window focus, and wry then hands the keyboard to the panel
                    // page. After Alt+Tab, a closed file picker or a click from another window the
                    // chat must get it back (Electron did this by itself).
                    let r = DefSubclassProc(hwnd, msg, wparam, lparam);
                    later(|c| c.on_panel_activated());
                    return r;
                }
                (2, false) => later(|c| c.on_tab_focus()),
                (3, false) => later(|c| c.on_toast_focus()),
                _ => {}
            }
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED if which == 1 => {
            let r = DefSubclassProc(hwnd, msg, wparam, lparam);
            win32::forget_displays();
            later(|c| c.on_displays_changed());
            return r;
        }
        WM_SETTINGCHANGE if which == 1 => {
            win32::forget_displays();
            if wparam.0 == SPI_SETWORKAREA.0 as usize {
                later(|c| c.on_displays_changed()); // the taskbar moved or changed size
            } else {
                later(|c| {
                    c.apply_theme(); // light / dark
                    let d = c.target_display();
                    if (d.scale - c.applied_scale.0).abs() > 1e-6 || (d.ui - c.applied_scale.1).abs() > 1e-6 {
                        c.on_displays_changed(); // Text size (or the monitor scale) changed
                    }
                });
            }
        }
        WM_QUERYENDSESSION if which == 1 => {
            core::SESSION_ENDING.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        WM_ENDSESSION if which == 1 => {
            // FALSE: the sign-out or shutdown was cancelled (an app wouldn't close), ChatDock carries on
            core::SESSION_ENDING.store(wparam.0 != 0, std::sync::atomic::Ordering::SeqCst);
        }
        WM_CLOSE if which == 1 => {
            later(|c| c.close_panel(true, "alt-f4"));
            return LRESULT(0);
        }
        WM_CLOSE => return LRESULT(0), // the tab and the pop-ups can't be closed (Alt+F4)
        _ => {}
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

fn subclass(hwnd: isize, which: usize) {
    unsafe {
        let _ = SetWindowSubclass(win32::h(hwnd), Some(subclass_proc), 0xC4A7_D0C0 + which, which);
    }
}

/// Everything ChatDock needs at start: its windows, the app state, and then the rest (see start()).
pub fn init(app: &mut tauri::App, args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let settings = Settings::load(&args.data_dir);
    frames::init();
    let dark = win32::system_dark() || settings.str("theme") == "dark";
    let bg = if settings.str("theme") == "light" || !dark { Color(255, 255, 255, 255) } else { Color(24, 25, 29, 255) };
    let panel = window(app, "panel", "panel.html", &args, false, true, Some(bg))?;
    let tab = window(app, "tab", "tab.html", &args, true, true, None)?;
    let glow = window(app, "glow", "glow.html", &args, true, false, None)?;
    let edgewin = window(app, "edge", "edge.html", &args, true, false, None)?;
    let toastwin = window(app, "toasts", "toast.html", &args, true, true, None)?;
    glow.w.set_ignore_cursor_events(true)?;
    edgewin.w.set_ignore_cursor_events(true)?;
    for w in [&tab, &glow, &edgewin, &toastwin] {
        win32::set_tool_window(w.hwnd);
    }
    subclass(panel.hwnd, 1);
    subclass(tab.hwnd, 2);
    subclass(toastwin.hwnd, 3);
    let own_hwnds = vec![panel.hwnd, tab.hwnd, glow.hwnd, toastwin.hwnd, edgewin.hwnd];
    let lang = String::new();
    let mut core = Core {
        args,
        settings,
        lang,
        panel,
        tab,
        glow,
        edgewin,
        toastwin,
        update_win: None,
        whatsnew: Default::default(),
        identify: Default::default(),
        ready: HashSet::new(),
        own_hwnds,
        panel_page_hwnds: Vec::new(),
        panel_alpha: 255,
        dock_display: String::new(),
        applied_scale: (0.0, 0.0),
        ui_reload_pending: false,
        ui_crashes: Vec::new(),
        reload_pending: HashSet::new(),
        recovering: false,
        resize_grab: None,
        chats: Default::default(),
        counts: HashMap::new(),
        site_counts: HashMap::new(),
        counted_pages: HashSet::new(),
        load_state: HashMap::new(),
        first_shown: HashMap::new(),
        asleep: HashMap::new(),
        last_used: HashMap::new(),
        zero_timers: HashMap::new(),
        retry_timers: HashMap::new(),
        fallback_timers: HashMap::new(),
        last_flash: HashMap::new(),
        last_content_at: HashMap::new(),
        load_started_at: HashMap::new(),
        panel_state: PanelState::Hidden,
        help_mode: false,
        settings_mode: false,
        banner_shown: false,
        banner_dismissed: false,
        prev_foreground: 0,
        last_auto_hide_at: 0,
        blurred_while_opening: false,
        focus_watch: false,
        test_foreground: None,
        call_windows: Vec::new(),
        volume_states: HashMap::new(),
        hotkey_warned: String::new(),
        call_window_search: None,
        bar_shares: Default::default(),
        test_browser_pid: None,
        last_esc_at: 0,
        anim_gen: 0,
        anim_frames: 0,
        settings_timer: 0,
        toast_foreground: 0,
        edge: Default::default(),
        toasts: Default::default(),
        upd: Default::default(),
        update_announced: String::new(),
        pending_update_to: String::new(),
        hotkey_ok: false,
        hotkeys: Vec::new(),
        autostart_cache: false,
        dnd_timer: 0,
        save_timer: 0,
        quitting: false,
    };
    core.lang = core.ui_lang();
    core::install(core);
    core::with(|c| c.start());
    Ok(())
}

impl Core {
    fn start(&mut self) {
        let now = rt::epoch_ms();
        for id in apps::ids() {
            self.last_used.insert(id.to_string(), now);
        }
        if self.enabled_apps().is_empty() {
            self.settings.set_in("apps", "instagram", json!(true));
        }
        let active = self.active();
        if !self.is_enabled(&active) {
            let first = self.enabled_apps()[0];
            self.set_setting("active", json!(first));
        }
        self.resolve_legacy_display();
        let d = self.target_display();
        self.applied_scale = (d.scale, d.ui);
        frames::set_display(&d.id, d.hz);
        let g = self.panel_geometry(&d);
        win32::set_bounds(self.panel.hwnd, Rect { x: self.hidden_x(&g, &d), ..g });
        self.apply_theme();
        self.apply_capture_protection();
        self.autostart_cache = crate::autostart::get();
        if self.can_autostart() {
            crate::autostart::migrate();
        }
        crate::tray::create(self);
        self.register_hotkey();
        self.init_chats();
        self.schedule_dnd_end();
        rt::after(60_000, sleep_tick);
        crate::wheel::start();
        self.edge_tick_soon(500);
        self.init_updater();
        // after an update: What's new, once ChatDock is up and the chats have started loading (the
        // self-test shows it itself, where it can't get in the way)
        if self.detect_update().is_some() && !self.args.selftest {
            timer(2000, |c| c.announce_updated());
        }
        if !self.settings.bool("onboarded") {
            self.help_mode = true;
        }
        if self.args.selftest {
            crate::selftest::start();
        }
        log!(
            "ready {{\"version\":\"{}\",\"packaged\":{},\"hidden\":{},\"profile\":{:?},\"exe\":{:?}}}",
            rt::version(),
            !cfg!(debug_assertions),
            self.args.hidden,
            self.args.data_dir.display().to_string(),
            std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()
        );
    }

    /// The panel's page finished loading: the welcome screen opens by itself on the first start.
    pub fn panel_page_ready(&mut self) {
        if !self.settings.bool("onboarded") && self.panel_state == PanelState::Hidden && !self.args.hidden {
            self.help_mode = true;
            self.open_panel(None, "startup");
        }
    }

    // -----------------------------------------------------------------------------------------
    // Geometry (physical pixels; widths are stored in DIP)
    // -----------------------------------------------------------------------------------------
    /// The monitor picked in Settings, while it is connected. Settings keeps its device path, which
    /// stays the same when Windows renumbers its monitors (older versions kept "\\.\DISPLAY1").
    pub fn chosen_display(&self) -> Option<Display> {
        let want = self.settings.get("displayId").as_str().filter(|s| !s.is_empty())?;
        win32::displays().into_iter().find(|d| d.key == want || d.id == want)
    }

    /// Monitor setting "Automatic" (the default): none picked, or the picked one isn't connected
    /// right now (it is used again as soon as it is back).
    pub fn auto_display(&self) -> bool {
        self.chosen_display().is_none()
    }

    /// The monitor the dock is on: the one chosen in Settings; with "Automatic", the one it was used
    /// on last, and at first the outermost monitor on the dock side (its edge is where the mouse
    /// stops when you push it all the way right, or left).
    pub fn target_display(&self) -> Display {
        let list = win32::displays();
        let left = self.on_left();
        let outermost = || {
            list.iter().max_by_key(|d| {
                let edge = if left { -d.bounds.x } else { d.bounds.right() };
                (edge, d.primary)
            })
        };
        self.chosen_display()
            .or_else(|| list.iter().find(|d| !self.dock_display.is_empty() && d.id == self.dock_display).cloned())
            .or_else(|| outermost().cloned())
            .or_else(|| list.first().cloned())
            .unwrap_or(Display {
                id: String::new(),
                key: String::new(),
                name: String::new(),
                internal: false,
                bounds: Rect { x: 0, y: 0, w: 1920, h: 1080 },
                work: Rect { x: 0, y: 0, w: 1920, h: 1040 },
                scale: 1.0,
                ui: 1.0,
                hz: 60,
                primary: true,
            })
    }

    /// Is the dock-side edge of this monitor a real screen edge at height y (the mouse stops there),
    /// or the seam to another monitor (the mouse just passes into it)?
    pub fn outer_edge_at(&self, d: &Display, y: i32) -> bool {
        self.outer_edge_on(d, y, self.on_left())
    }

    /// The same for either edge.
    pub fn outer_edge_on(&self, d: &Display, y: i32, left: bool) -> bool {
        let beyond = if left { d.bounds.x - 1 } else { d.bounds.right() };
        !win32::displays().iter().any(|o| o.id != d.id && o.bounds.contains(beyond, y))
    }

    /// Where along a monitor's left or right edge the tab can come out (outer edge, corners left
    /// alone like edge_step does), as parts of its height from the top: what the monitor map in
    /// Settings lights up.
    pub fn outer_ranges(&self, d: &Display, left: bool) -> Vec<(f64, f64)> {
        let b = d.bounds;
        let margin = (90.0 * d.scale).max(b.h as f64 * 0.1).round() as i32;
        let (top, bottom) = (b.y + margin, d.work.bottom() - margin);
        let mut out: Vec<(f64, f64)> = Vec::new();
        if bottom <= top || b.h <= 0 {
            return out;
        }
        const STEPS: i32 = 60;
        let part = |y: i32| (y - b.y) as f64 / b.h as f64;
        let mut start: Option<i32> = None;
        let mut last = top;
        for i in 0..=STEPS {
            let y = top + (bottom - top) * i / STEPS;
            if self.outer_edge_on(d, y, left) {
                start.get_or_insert(y);
                last = y;
            } else if let Some(y0) = start.take() {
                out.push((part(y0), part(last)));
            }
        }
        if let Some(y0) = start {
            out.push((part(y0), part(last)));
        }
        out
    }

    /// How much of that edge (a part of the monitor's height) the tab can come out on.
    pub fn outer_share(&self, d: &Display, left: bool) -> f64 {
        self.outer_ranges(d, left).iter().map(|(a, b)| b - a).sum()
    }

    /// Monitors numbered the way they sit, left to right (then top to bottom): 1, 2, 3 ... The map,
    /// the list and "Show numbers on the screens" all use these numbers.
    pub fn numbered_displays(&self) -> Vec<(u32, Display)> {
        let mut list = win32::displays();
        list.sort_by_key(|d| (d.bounds.x, d.bounds.y));
        list.into_iter().enumerate().map(|(i, d)| (i as u32 + 1, d)).collect()
    }

    /// "DELL U2720Q", "Built-in screen", or "External monitor" when Windows doesn't know its name.
    pub fn monitor_label(&self, d: &Display) -> String {
        if d.internal {
            self.t("mon.builtin")
        } else if !d.name.is_empty() {
            d.name.clone()
        } else {
            self.t("mon.generic")
        }
    }

    pub fn display_at(&self, x: i32, y: i32) -> Option<Display> {
        win32::displays().into_iter().find(|d| d.bounds.contains(x, y))
    }

    /// "Automatic": the dock moves to this monitor (the tab, line, glow, pop-ups and the panel).
    pub fn use_display(&mut self, d: &Display) {
        if !self.auto_display() || self.dock_display == d.id {
            return;
        }
        log!("dock moves to {}", d.id);
        self.dock_display = d.id.clone();
        self.edge.scrolling = false; // that was on another monitor's edge
        frames::set_display(&d.id, d.hz);
        if self.panel_state == PanelState::Hidden {
            let g = self.panel_geometry(d);
            win32::set_bounds(self.panel.hwnd, Rect { x: self.hidden_x(&g, d), ..g });
        }
        self.hide_glow_now();
        self.update_glow(false);
        self.toasts_reposition();
    }

    pub fn on_left(&self) -> bool {
        self.settings.str("side") == "left"
    }

    fn app_width_dip(&self, id: &str) -> f64 {
        self.settings
            .get("widths")
            .get(id)
            .and_then(Value::as_f64)
            .or_else(|| apps::get(id).and_then(|a| a.width).map(f64::from))
            .unwrap_or_else(|| self.settings.get("width").as_f64().unwrap_or(460.0))
    }

    fn max_width(&self, d: &Display) -> i32 {
        ((MIN_W * d.ui).round() as i32).max((d.work.w as f64 * 0.7).round() as i32)
    }

    pub fn panel_geometry(&self, d: &Display) -> Rect {
        let min = (MIN_W * d.ui).round() as i32;
        let width = ((self.app_width_dip(&self.active()) * d.ui).round() as i32).clamp(min, self.max_width(d));
        let x = if self.on_left() { d.work.x } else { d.work.right() - width };
        Rect { x, y: d.work.y, w: width, h: d.work.h }
    }

    /// x of a panel of this size tucked away just past the dock edge
    pub fn hidden_x(&self, g: &Rect, d: &Display) -> i32 {
        if self.on_left() {
            d.work.x - g.w
        } else {
            d.work.right()
        }
    }

    /// Another monitor right against the dock edge? Then the panel appears in place instead of
    /// sliding across into that monitor.
    fn has_neighbour_on_dock_side(&self, d: &Display) -> bool {
        let left = self.on_left();
        let edge = if left { d.bounds.x } else { d.bounds.right() };
        win32::displays().iter().any(|o| {
            o.id != d.id
                && ((if left { o.bounds.right() } else { o.bounds.x }) - edge).abs() <= 1
                && o.bounds.y < d.bounds.bottom()
                && o.bounds.bottom() > d.bounds.y
        })
    }

    /// Switching to an app that wants a different width while the panel is open.
    pub fn fit_panel_to_app(&mut self) {
        if self.panel_state != PanelState::Open {
            return;
        }
        let g = self.panel_geometry(&self.target_display());
        if win32::window_rect(self.panel.hwnd) == g {
            return;
        }
        win32::set_bounds(self.panel.hwnd, g);
        self.layout_views();
        self.toasts_reposition();
    }

    pub fn view_showable(&self, id: &str) -> bool {
        !self.help_mode
            && !self.settings_mode
            && self.settings.bool("onboarded")
            && *self.first_shown.get(id).unwrap_or(&false)
            && self.load_state.get(id).copied() != Some("error")
    }

    /// Where the chat goes inside the panel: under the header (and banner), beside the grip.
    pub fn layout_views(&mut self) {
        let r = win32::window_rect(self.panel.hwnd);
        let scale = self.target_display().ui; // the panel page's header / grip are CSS px
        let top = ((HEADER_H + if self.banner_shown { BANNER_H } else { 0.0 }) * scale).round() as i32;
        let grip = (GRIP_W * scale).round() as i32;
        let bounds = Rect { x: if self.on_left() { 0 } else { grip }, y: top, w: (r.w - grip).max(1), h: (r.h - top).max(1) };
        let active = self.active();
        let panel_up = self.panel_state != PanelState::Hidden;
        let was_shown = self.chats.is_visible(&active);
        for id in self.chats.ids() {
            let show = panel_up && id == active && self.view_showable(&id);
            self.chats.set_bounds(&id, bounds);
            self.chats.set_visible(&id, show, id == active);
        }
        // The keyboard follows while the user is in the panel: to the chat when it appears (an app
        // that was loading or asleep), to the panel page when it goes (the error screen).
        if self.panel_state == PanelState::Open && win32::foreground_window() == self.panel.hwnd {
            let shown = self.chats.is_visible(&active);
            if shown && !was_shown {
                self.chats.focus(&active);
            } else if was_shown && !shown {
                crate::chats::focus_panel_page();
            }
        }
        self.chats.notify_moved();
        // The panel page covers the whole window; the chat goes on top of it (like Electron's views).
        // (Development builds can leave it as 1.5.0 had it, to check that the self-test notices.)
        if !(cfg!(debug_assertions) && std::env::var_os("CHATDOCK_TEST_OLD_STACKING").is_some()) {
            for &w in &self.panel_page_hwnds {
                win32::keep_at_bottom(w);
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Focus
    // -----------------------------------------------------------------------------------------
    pub fn snap(&self) -> String {
        let fg = win32::foreground_window();
        json!({
            "st": self.panel_state.as_str(),
            "vis": win32::is_visible(self.panel.hwnd),
            "foc": fg == self.panel.hwnd,
            "x": win32::window_rect(self.panel.hwnd).x,
            "fg": if fg == self.panel.hwnd {
                "PANEL".to_string()
            } else if self.args.selftest {
                format!("{}@{}", win32::class_name(fg), win32::process_name(fg))
            } else {
                win32::class_name(fg)
            },
            "tab": self.edge.tab_shown,
        })
        .to_string()
    }

    pub fn remember_foreground(&mut self, source: &str) {
        let mut fg = win32::foreground_window();
        // Clicking the tab / a pop-up activates it, so use whatever was in front before that.
        if source == "tab" && self.is_ours(fg) {
            fg = self.edge.tab_foreground;
        }
        if source == "toast" && self.is_ours(fg) {
            fg = self.toast_foreground;
        }
        self.prev_foreground =
            if source == "tray" || source == "menu" || fg == 0 || self.is_ours(fg) || !win32::is_visible(fg) { 0 } else { fg };
        log!("foreground before open: {fg} {} -> {}", win32::class_name(fg), self.prev_foreground);
    }

    /// Make the panel the foreground window (Windows may refuse; then we force it).
    fn activate_panel(&mut self) {
        if !win32::is_visible(self.panel.hwnd) {
            return;
        }
        if win32::foreground_window() != self.panel.hwnd {
            let ok = win32::force_foreground(self.panel.hwnd);
            log!("focus: forced {} | fg now {}", if ok { "ok" } else { "FAILED" }, win32::class_name(win32::foreground_window()));
        }
    }

    /// Put the keyboard into the chat page (no change to which window is in front).
    fn focus_content(&mut self) {
        let active = self.active();
        if !(self.view_showable(&active) && self.chats.focus(&active)) {
            crate::chats::focus_panel_page();
        }
    }

    /// The panel became the active window (see subclass_proc).
    pub fn on_panel_activated(&mut self) {
        if self.panel_state.showing() && win32::foreground_window() == self.panel.hwnd {
            self.focus_content();
            // once more a moment later: wry's own focus call for the panel page can arrive after ours
            timer(150, |c| {
                if c.panel_state.showing() && win32::foreground_window() == c.panel.hwnd {
                    c.focus_content();
                }
            });
        }
    }

    pub fn focus_panel(&mut self) {
        if !win32::is_visible(self.panel.hwnd) {
            return;
        }
        self.activate_panel();
        self.focus_content();
    }

    // -----------------------------------------------------------------------------------------
    // Open / close
    // -----------------------------------------------------------------------------------------
    pub fn open_panel(&mut self, app_id: Option<&str>, source: &str) {
        log!("open request {source} {} {}", app_id.unwrap_or(""), self.snap());
        if let Some(id) = app_id {
            if self.is_enabled(id) {
                self.set_active(id, false);
            }
        }
        let active = self.active();
        self.wake_app(&active); // a sleeping app loads again when it is opened
        if self.panel_state.showing() {
            self.focus_panel();
            self.broadcast_state();
            return;
        }
        // hidden, or re-opened while still sliding out (focus may already be back in the game)
        if matches!(source, "hotkey" | "tray" | "menu" | "launch") && self.panel_state == PanelState::Hidden {
            let (x, y) = win32::cursor_pos();
            if let Some(m) = self.display_at(x, y) {
                self.use_display(&m); // "Automatic": where the user is
            }
        }
        self.remember_foreground(source);
        self.blurred_while_opening = false;
        self.banner_shown = !self.banner_dismissed && win32::is_exclusive_fullscreen();
        self.hide_tab(true);
        self.stop_hold(false);
        self.hide_glow_now();
        let d = self.target_display();
        let g = self.panel_geometry(&d);
        let slide = !self.has_neighbour_on_dock_side(&d);
        let visible = win32::is_visible(self.panel.hwnd);
        let from_x = if visible { win32::window_rect(self.panel.hwnd).x } else { self.hidden_x(&g, &d) };
        win32::set_bounds(self.panel.hwnd, Rect { x: if slide { from_x } else { g.x }, ..g });
        let opacity = self.panel_opacity();
        let from_alpha = if visible { self.panel_alpha } else { 0 };
        if !slide {
            self.set_panel_alpha(from_alpha); // before it shows: it fades in from there
        } else if opacity < 255 || self.panel_alpha < 255 {
            self.set_panel_alpha(opacity);
        }
        self.panel_state = PanelState::Opening;
        self.counts_seen_in_view(); // the chat is on screen: its number is read
        self.layout_views();
        if !visible {
            win32::show(self.panel.hwnd);
        }
        win32::raise(self.panel.hwnd);
        self.focus_panel();
        self.broadcast_state();
        self.toasts_dismiss_app(&active); // reading it now
        self.toasts_reposition(); // other pop-ups move beside the chat
        log!("open {{\"source\":\"{source}\",\"slide\":{slide},\"prevForeground\":{}}}", self.prev_foreground);
        let kind = if slide { AnimKind::SlideX { from: from_x, to: g.x } } else { AnimKind::Fade { from: from_alpha, to: opacity } };
        self.animate(OPEN_MS, ease_out_quint, kind, AnimDone::Opened);
    }

    fn opened(&mut self) {
        self.panel_state = PanelState::Open;
        self.fit_panel_to_app(); // the app or the dock side changed while it was sliding in
        if win32::foreground_window() == self.panel.hwnd {
            // Already in front: only move the keyboard into the page. Re-activating here could undo
            // a click the user made on the game in this very moment.
            self.focus_content();
        } else if self.blurred_while_opening || win32::mouse_button_down() {
            // The user clicked back into the game while it was still sliding in: treat it like
            // any click elsewhere instead of grabbing focus back.
            log!("clicked elsewhere during the slide-in");
            self.check_auto_hide(false);
        } else {
            self.focus_panel(); // Windows didn't let us take focus when it started; try once more
        }
        self.chats.notify_moved();
        self.toasts_reposition();
        self.broadcast_state();
        log!("opened {}", self.snap());
    }

    pub fn close_panel(&mut self, restore_focus: bool, reason: &str) {
        log!("close request {reason} {}", self.snap());
        if matches!(self.panel_state, PanelState::Hidden | PanelState::Closing) {
            return;
        }
        self.panel_state = PanelState::Closing;
        self.resize_grab = None;
        let target = if restore_focus { self.prev_foreground } else { 0 };
        self.prev_foreground = 0;
        if target != 0 {
            // straight back into the game, no extra click needed
            let ok = win32::restore_foreground(target);
            log!(
                "focus back to {target} {} {} | now {}",
                win32::class_name(target),
                if ok { "ok" } else { "FAILED" },
                win32::class_name(win32::foreground_window())
            );
        }
        let d = self.target_display();
        let b = win32::window_rect(self.panel.hwnd);
        let slide = !self.has_neighbour_on_dock_side(&d);
        let to_x = self.hidden_x(&b, &d);
        self.broadcast_state();
        log!("close {{\"restoreFocus\":{restore_focus},\"target\":{target}}}");
        let kind = if slide { AnimKind::SlideX { from: b.x, to: to_x } } else { AnimKind::Fade { from: self.panel_alpha, to: 0 } };
        self.animate(CLOSE_MS, ease_in_cubic, kind, AnimDone::Closed);
    }

    fn closed(&mut self) {
        win32::hide(self.panel.hwnd);
        self.panel_state = PanelState::Hidden;
        let active = self.active();
        self.last_used.insert(active, rt::epoch_ms());
        self.stop_settings_timer();
        let onboarded = self.settings.bool("onboarded");
        if (self.help_mode && onboarded) || self.settings_mode {
            // next time it opens on the chats
            self.help_mode = self.help_mode && !onboarded;
            self.settings_mode = false;
            self.identify_close();
        }
        self.layout_views(); // the chat counts as hidden for the site now
        self.broadcast_state();
        self.update_glow(false);
        self.toasts_reposition();
        self.schedule_back_to_list();
        log!("closed {}", self.snap());
    }

    pub fn toggle_panel(&mut self, source: &str) {
        if self.panel_state.showing() {
            self.close_panel(true, source);
        } else {
            self.open_panel(None, source);
        }
    }

    /// One step per screen refresh (see frames.rs); positions come from the time, so a late frame
    /// never slows the slide down. The clock starts at the first frame, not at the call: getting
    /// the window ready (focus, layout) must not eat the start of the slide.
    fn animate(&mut self, ms: f64, ease: fn(f64) -> f64, kind: AnimKind, done: AnimDone) {
        self.anim_gen += 1;
        let gen = self.anim_gen;
        let last_x = match kind {
            AnimKind::SlideX { from, .. } => from,
            AnimKind::Fade { .. } => 0,
        };
        ANIM.with(|a| *a.borrow_mut() = Some(Anim { gen, t0: None, ms, ease, kind, done, last_x }));
        frames::request(move |now| anim_frame(gen, now));
    }

    fn anim_step(&mut self, gen: u64, now: f64) -> bool {
        self.anim_frames += 1;
        let Some(mut a) = ANIM.with(|x| x.borrow_mut().take()) else { return false };
        if a.gen != gen {
            ANIM.with(|x| *x.borrow_mut() = Some(a));
            return false;
        }
        let t0 = *a.t0.get_or_insert(now);
        let t = ((now - t0) / a.ms).clamp(0.0, 1.0);
        let p = (a.ease)(t);
        match a.kind {
            AnimKind::SlideX { from, to } => {
                let x = (from as f64 + (to - from) as f64 * p).round() as i32;
                if x != a.last_x {
                    a.last_x = x;
                    let r = win32::window_rect(self.panel.hwnd);
                    win32::move_to(self.panel.hwnd, x, r.y);
                }
            }
            AnimKind::Fade { from, to } => {
                let alpha = (from as f64 + (to as f64 - from as f64) * p).round().clamp(0.0, 255.0) as u8;
                if alpha != self.panel_alpha {
                    self.set_panel_alpha(alpha);
                }
            }
        }
        if t < 1.0 {
            ANIM.with(|x| *x.borrow_mut() = Some(a));
            return true;
        }
        match a.done {
            AnimDone::Opened => self.opened(),
            AnimDone::Closed => self.closed(),
            AnimDone::Moved => {
                self.chats.notify_moved();
                self.toasts_reposition();
            }
        }
        false
    }

    // -----------------------------------------------------------------------------------------
    // Hiding when you click elsewhere
    // -----------------------------------------------------------------------------------------
    pub fn on_panel_blur(&mut self) {
        self.resize_grab = None; // a drag can't go on in a window that isn't active
        log!("blur {}", self.snap());
        if self.panel_state == PanelState::Opening {
            // Only a real mouse click counts (some games grab focus back on their own); decided
            // when the slide-in finishes. The second press of a double click on ChatDock's own tray
            // icon (which opened it) doesn't.
            let tray_again = crate::tray::clicked_just_now() && win32::is_taskbar(win32::foreground_window());
            self.blurred_while_opening = win32::mouse_button_down() && !tray_again;
            return;
        }
        if self.panel_state != PanelState::Open {
            return;
        }
        if self.settings.bool("pinned") {
            timer(80, |c| {
                if c.panel_state == PanelState::Open {
                    win32::raise(c.panel.hwnd); // stay above a topmost game
                }
            });
            return;
        }
        timer(120, |c| c.check_auto_hide(false));
    }

    fn cursor_over_panel(&self) -> bool {
        let (x, y) = win32::cursor_pos();
        win32::window_rect(self.panel.hwnd).contains(x, y)
    }

    /// The foreground window (the self-test can pretend one).
    fn foreground(&self) -> isize {
        self.test_foreground.unwrap_or_else(win32::foreground_window)
    }

    pub fn check_auto_hide(&mut self, dragging: bool) {
        let fg = self.foreground();
        if self.panel_state != PanelState::Open || self.settings.bool("pinned") || fg == self.panel.hwnd {
            log!(
                "autohide: skip {{\"st\":\"{}\",\"pinned\":{},\"foc\":{}}}",
                self.panel_state.as_str(),
                self.settings.bool("pinned"),
                fg == self.panel.hwnd
            );
            return;
        }
        if self.is_ours(fg) {
            log!("autohide: skip, our window has focus"); // e.g. our own dialog, menu or a file picker
            if !self.is_own_window(fg) && !self.focus_watch {
                // A window of the chats' browser that isn't a dialog of ours: the "… is sharing your
                // screen" bar takes the focus when a share starts. The panel isn't the active window
                // any more, so a click elsewhere from there never reaches it: watch where it goes.
                self.focus_watch = true;
                timer(FOCUS_WATCH_MS, |c| c.watch_focus());
            }
            return;
        }
        if win32::mouse_button_down() {
            // Still pressing: maybe dragging a file from Explorer into the chat. Decide on release.
            timer(100, |c| c.check_auto_hide(true));
            return;
        }
        if dragging && self.cursor_over_panel() {
            log!("autohide: skip, released over the panel (drop)");
            self.focus_panel(); // something was dropped onto the chat - keep it open
            return;
        }
        self.last_auto_hide_at = rt::epoch_ms();
        self.close_panel(false, "clicked elsewhere"); // the user already clicked where they wanted to go
    }

    /// The focus is on a window of the chats' browser (the screen-share bar). When it moves on to
    /// another program, that was a click elsewhere.
    fn watch_focus(&mut self) {
        let fg = self.foreground();
        if self.panel_state != PanelState::Open || self.settings.bool("pinned") || fg == self.panel.hwnd || self.is_own_window(fg) {
            self.focus_watch = false; // hidden, pinned, or back in ChatDock (whose own signals take over)
            return;
        }
        if fg == 0 || self.is_chat_browser_window(fg) {
            timer(FOCUS_WATCH_MS, |c| c.watch_focus()); // still there (none: the focus is changing hands)
            return;
        }
        self.focus_watch = false;
        log!("focus moved from the chats' window to {}", win32::class_name(fg));
        timer(120, |c| c.check_auto_hide(false)); // like a blur: it may come straight back
    }

    // -----------------------------------------------------------------------------------------
    // Settings / welcome screens
    // -----------------------------------------------------------------------------------------
    pub fn open_settings(&mut self, section: &str, source: &str) {
        let was_open = self.settings_mode;
        self.read_discord_servers(); // (the Discord page lists them)
        self.settings_mode = true;
        self.help_mode = false;
        self.start_settings_timer();
        if !was_open {
            self.refresh_memory(); // per-app RAM figures, then every few seconds while it stays open
        }
        self.layout_views();
        if self.panel_state.showing() {
            self.activate_panel();
            crate::chats::focus_panel_page();
            self.broadcast_state();
        } else {
            self.open_panel(None, source);
        }
        if !section.is_empty() {
            self.emit("panel", "settings:goto", json!([section]));
        }
    }

    pub fn close_settings(&mut self) {
        if !self.settings_mode {
            return;
        }
        self.settings_mode = false;
        self.identify_close();
        self.counts_seen_in_view(); // back on the chat
        self.stop_settings_timer();
        self.fit_panel_to_app(); // (the app on screen may have changed while Settings was open)
        self.layout_views();
        self.focus_panel();
        self.broadcast_state();
    }

    /// While the settings screen is open, its RAM figures refresh every few seconds.
    fn start_settings_timer(&mut self) {
        rt::cancel(self.settings_timer);
        self.settings_timer = rt::after(4000, settings_tick);
    }

    fn stop_settings_timer(&mut self) {
        rt::cancel(self.settings_timer);
        self.settings_timer = 0;
    }

    pub fn show_help(&mut self) {
        self.help_mode = true;
        self.settings_mode = false;
        self.identify_close();
        self.autostart_cache = crate::autostart::get();
        self.layout_views();
        self.open_panel(None, "menu");
        self.broadcast_state();
    }

    pub fn finish_onboarding(&mut self, autostart: Option<bool>) {
        let first_time = !self.settings.bool("onboarded");
        self.set_setting("onboarded", json!(true));
        if first_time {
            if let Some(on) = autostart {
                self.set_open_at_login(on);
            }
        }
        self.help_mode = false;
        self.counts_seen_in_view(); // the chat is on screen now
        self.layout_views();
        self.focus_panel();
        self.broadcast_state();
        if first_time {
            let title = self.t("balloon.hereTitle");
            let body = self.t(if self.on_left() { "balloon.here.left" } else { "balloon.here.right" });
            self.notice(&title, &body);
        }
    }

    pub fn set_active(&mut self, id: &str, focus: bool) {
        if !self.is_enabled(id) {
            return;
        }
        let leaving = self.active();
        self.last_used.insert(leaving, rt::epoch_ms()); // the app we leave was in use until now
        self.set_setting("active", json!(id));
        if self.panel_state != PanelState::Hidden {
            self.wake_app(id);
        }
        self.fit_panel_to_app();
        self.layout_views();
        if self.panel_state.showing() {
            self.toasts_dismiss_app(id);
        }
        self.counts_seen_in_view();
        if focus && self.panel_state != PanelState::Hidden {
            self.focus_panel();
        }
        self.broadcast_state();
    }

    fn other_app(&self) -> String {
        let list = self.enabled_apps();
        let i = list.iter().position(|a| *a == self.active()).unwrap_or(0);
        list[(i + 1) % list.len()].to_string()
    }

    /// The chat window's see-through setting (60-100 %) as a window alpha.
    pub fn panel_opacity(&self) -> u8 {
        (self.settings.f64("opacity").clamp(0.6, 1.0) * 255.0).round() as u8
    }

    pub fn set_panel_alpha(&mut self, alpha: u8) {
        win32::set_alpha(self.panel.hwnd, alpha);
        self.panel_alpha = alpha;
    }

    /// The resize grip was grabbed: remember where on it (physical px, so any monitor layout and
    /// scale works; the page's own screen coordinates don't match Windows' on mixed-DPI setups).
    pub fn resize_start(&mut self) {
        if self.panel_state != PanelState::Open {
            return;
        }
        let (cx, _) = win32::cursor_pos();
        let r = win32::window_rect(self.panel.hwnd);
        let grip = ((GRIP_W * self.target_display().ui).round() as i32).max(1);
        let off = if self.on_left() { r.right() - cx } else { cx - r.x };
        self.resize_grab = Some(off.clamp(0, grip));
    }

    /// The grip is being dragged (once per frame): the inner edge follows the cursor.
    pub fn resize_to(&mut self) {
        let Some(grab) = self.resize_grab.filter(|_| self.panel_state == PanelState::Open) else {
            self.resize_grab = None;
            return;
        };
        let (cx, _) = win32::cursor_pos();
        let d = self.target_display();
        let edge_px = if self.on_left() { cx + grab } else { cx - grab };
        let min = (MIN_W * d.ui).round() as i32;
        let width = (if self.on_left() { edge_px - d.work.x } else { d.work.right() - edge_px }).clamp(min, self.max_width(&d));
        if width == win32::window_rect(self.panel.hwnd).w {
            return;
        }
        let x = if self.on_left() { d.work.x } else { d.work.right() - width };
        win32::set_bounds(self.panel.hwnd, Rect { x, y: d.work.y, w: width, h: d.work.h });
        self.layout_views();
        let active = self.active();
        self.settings.set_in("widths", &active, json!((width as f64 / d.ui).round())); // remembered per app
        self.save_soon();
        self.toasts_reposition();
    }

    /// The dock moved to the other screen edge. An open panel slides in again from its new side.
    pub fn set_side(&mut self, side: &str) {
        if self.settings.str("side") == side {
            return;
        }
        self.hide_tab(true);
        self.stop_hold(false);
        self.resize_grab = None;
        self.set_setting("side", json!(side));
        log!("dock side {side}");
        if self.auto_display() {
            // "Automatic": the outermost monitor on the new side (its edge is where the mouse stops)
            self.dock_display.clear();
            let d = self.target_display();
            frames::set_display(&d.id, d.hz);
        }
        self.broadcast_state(); // panel, tab and glow mirror themselves
        if self.panel_state == PanelState::Open {
            let d = self.target_display();
            let g = self.panel_geometry(&d);
            let slide = !self.has_neighbour_on_dock_side(&d);
            let from_x = self.hidden_x(&g, &d);
            win32::set_bounds(self.panel.hwnd, Rect { x: if slide { from_x } else { g.x }, ..g });
            self.layout_views();
            if slide {
                self.animate(OPEN_MS, ease_out_quint, AnimKind::SlideX { from: from_x, to: g.x }, AnimDone::Moved);
            }
        }
        self.update_glow(false);
        self.toasts_reposition();
    }

    /// The Electron builds named a non-main monitor by a number that means nothing now. With
    /// exactly one other monitor it can only have been that one; with none plugged in, wait.
    pub fn resolve_legacy_display(&mut self) {
        // 1.5.x kept Windows' name for the monitor ("\\.\DISPLAY1"), which can change: keep its
        // device path instead, as soon as that monitor is connected.
        let saved = self.settings.get("displayId").as_str().filter(|s| s.starts_with(r"\\.\")).map(str::to_string);
        if let Some(gdi) = saved {
            if let Some(d) = win32::displays().into_iter().find(|d| d.id.eq_ignore_ascii_case(&gdi) && d.key != d.id) {
                log!("monitor setting: {} is now kept as {}", d.id, d.key);
                self.set_setting("displayId", json!(d.key));
                self.set_setting("displayLabel", json!(self.monitor_label(&d)));
            }
            return;
        }
        if !self.settings.get("displayId").is_number() {
            return;
        }
        let others: Vec<Display> = win32::displays().into_iter().filter(|d| !d.primary).collect();
        match others.len() {
            0 => {}
            1 => {
                log!("dock monitor from the Electron build: {}", others[0].id);
                self.set_setting("displayId", json!(others[0].key));
                self.set_setting("displayLabel", json!(self.monitor_label(&others[0])));
            }
            _ => {
                log!("dock monitor from the Electron build: several to choose from, main one used");
                self.set_setting("displayId", Value::Null);
            }
        }
    }

    pub fn on_displays_changed(&mut self) {
        self.resolve_legacy_display();
        if !self.dock_display.is_empty() && !win32::displays().iter().any(|d| d.id == self.dock_display) {
            self.dock_display.clear(); // that monitor is gone
        }
        self.hide_tab(true);
        self.stop_hold(false);
        let d = self.target_display();
        self.applied_scale = (d.scale, d.ui);
        frames::forget_display(); // (its adapter is opened anew)
        frames::set_display(&d.id, d.hz);
        if self.panel_state == PanelState::Open {
            win32::set_bounds(self.panel.hwnd, self.panel_geometry(&d));
            self.layout_views();
        } // (still sliding in: opened() fits it)
        self.update_glow(false);
        self.toasts_reposition();
        self.identify_refresh();
        if self.settings_mode {
            self.broadcast_state(); // the monitor map and list in Settings
        }
    }

    pub fn apply_theme(&mut self) {
        let theme = self.settings.str("theme").to_string();
        let t = match theme.as_str() {
            "dark" => Some(tauri::Theme::Dark),
            "light" => Some(tauri::Theme::Light),
            _ => None,
        };
        for w in [&self.panel, &self.tab, &self.toastwin] {
            let _ = w.w.set_theme(t);
        }
        let dark = match theme.as_str() {
            "dark" => true,
            "light" => false,
            _ => win32::system_dark(),
        };
        let bg = if dark { Color(24, 25, 29, 255) } else { Color(255, 255, 255, 255) };
        let _ = self.panel.w.set_background_color(Some(bg));
        self.chats.set_theme(&theme, dark);
    }

    /// "Hide from screenshots & streams": every window of ChatDock's own (the tab shows the apps,
    /// their unread numbers and calls). The call and sign-in windows a site opens belong to WebView2,
    /// and Windows lets a program hide only its own windows.
    pub fn apply_capture_protection(&mut self) {
        let on = self.settings.bool("hideFromCapture");
        let mut hwnds = vec![self.panel.hwnd, self.toastwin.hwnd, self.tab.hwnd, self.glow.hwnd, self.edgewin.hwnd];
        hwnds.extend(self.update_win.as_ref().map(|w| w.hwnd));
        hwnds.extend(self.whatsnew.win.as_ref().map(|w| w.hwnd));
        hwnds.extend(self.identify_hwnds());
        hwnds.extend(crate::popups::hwnds()); // the windows the chats open (a call, a sign-in)
        for hwnd in hwnds {
            win32::set_capture_excluded(hwnd, on);
        }
    }

    // -----------------------------------------------------------------------------------------
    // Keyboard (the panel's own page and the chat pages)
    // -----------------------------------------------------------------------------------------
    /// A key the panel handles: vk = Windows virtual-key code. `host` = the panel's own page.
    pub fn on_key(&mut self, vk: u32, ctrl: bool, alt: bool, shift: bool, host: bool) {
        let active = self.active();
        if vk == 0x1B && !ctrl && !alt && !shift {
            // one Esc still reaches the page (closes its pop-ups); two quick ones hide the panel
            let now = rt::epoch_ms();
            if now - self.last_esc_at < 450 {
                self.last_esc_at = 0;
                self.close_panel(true, "esc esc");
            } else {
                self.last_esc_at = now;
                if self.settings_mode && host {
                    // one Esc goes back: clears the search, then back to Settings' home page, then out
                    // of Settings (the page knows which it is)
                    self.emit("panel", "settings:esc", json!([]));
                }
            }
            return;
        }
        self.last_esc_at = 0;
        match (vk, ctrl, alt) {
            (0x31..=0x39 | 0x61..=0x69, true, false) if !shift => {
                let n = if vk >= 0x61 { vk - 0x61 } else { vk - 0x31 } as usize;
                if let Some(target) = self.enabled_apps().get(n) {
                    self.set_active(target, true);
                }
            }
            (0x09, true, _) => {
                let next = self.other_app();
                self.set_active(&next, true);
            }
            (0x52, true, false) | (0x74, _, false) => self.reload_app(&active, shift), // Ctrl+R, F5
            (0x57, true, false) => self.close_panel(true, "ctrl+w"),
            (0xBB | 0x6B, true, false) => self.zoom_step(&active, 1),
            (0xBD | 0x6D, true, false) => self.zoom_step(&active, -1),
            (0x30 | 0x60, true, false) => self.zoom_step(&active, 0),
            (0x25, false, true) => self.chats.go_back(&active),
            (0x27, false, true) => self.chats.go_forward(&active),
            _ => {}
        }
    }

    pub fn zoom_step(&mut self, id: &str, dir: i32) {
        let current = self.zoom_of(id);
        let next = if dir == 0 {
            1.0
        } else {
            // the next step up or down from where it is (Ctrl+wheel can leave it between steps)
            let steps = core::ZOOM_STEPS;
            let up = steps.iter().copied().find(|z| *z > current + 0.001);
            let down = steps.iter().copied().rev().find(|z| *z < current - 0.001);
            if dir > 0 {
                up.unwrap_or(steps[steps.len() - 1])
            } else {
                down.unwrap_or(steps[0])
            }
        };
        self.settings.set_in("zoom", id, json!(next));
        self.save_soon();
        self.chats.set_zoom(id, next);
        self.broadcast_state();
    }

    /// The page zoomed itself (Ctrl + mouse wheel): remember it for that app.
    pub fn zoom_changed(&mut self, id: &str, factor: f64) {
        if (self.zoom_of(id) - factor).abs() < 0.001 {
            return;
        }
        self.settings.set_in("zoom", id, json!((factor * 100.0).round() / 100.0));
        self.save_soon();
        self.broadcast_state();
    }
}

fn anim_frame(gen: u64, now: f64) {
    // the app state busy (a window message in the middle of other work): try on the next frame
    let again = core::with(|c| c.anim_step(gen, now)).unwrap_or(true);
    if again {
        frames::request(move |now| anim_frame(gen, now));
    }
}

fn settings_tick() {
    let again = core::with(|c| {
        if c.settings_mode && c.panel_state == PanelState::Open {
            c.broadcast_state();
        }
        c.settings_mode
    });
    match again {
        Some(true) => {
            core::with(|c| c.settings_timer = rt::after(4000, settings_tick));
        }
        Some(false) => {}
        None => {
            rt::after(50, settings_tick);
        }
    }
}

fn sleep_tick() {
    core::with(|c| c.sleep_check());
    rt::after(60_000, sleep_tick);
}
