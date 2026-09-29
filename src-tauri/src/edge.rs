//! The dock edge: the white tab (comes out when the cursor is held on the edge, follows the cursor
//! along it), the line that grows along the edge while the cursor is held there, and the unread
//! glow. Positions are physical pixels; sizes are given in DIP and scaled with the display.

use serde_json::json;

use crate::{
    apps,
    core::{self, timer, Core, PanelState},
    frames, log, rt,
    win32::{self, Display, Rect},
};

const EDGE_PX: f64 = 2.0; // cursor within this many DIP of the dock edge counts as "on the edge"
const DWELL_MS: i64 = 150; // with no hold time set: how long it rests there before the tab appears
const HOLD_LINE_MS: i64 = 110; // with a hold time: the line along the edge starts after this long
const HOLD_SLACK_PX: f64 = 10.0; // once holding, a shaky hand may drift this far off the edge
const EDGE_W: f64 = 32.0; // edge line window (Windows won't make it narrower); the line is drawn at the edge
const TAB_W: f64 = 60.0; // tab window: the white pill plus room for its shadow
const GLOW_W: f64 = 32.0;
const GLOW_H: f64 = 128.0;
const TAB_LINGER_MS: i64 = 900; // tab stays this long after the cursor wanders off
const TAB_FOLLOW_MARGIN: f64 = 26.0; // sliding along the edge, the cursor stays this far inside the pill
const TAB_GLIDE_MS: f64 = 30.0; // the tab closes about 63% of its distance to the cursor in this time
const WHEEL_ZONE_PX: f64 = 24.0; // the wheel turning this close to the edge is scrolling what's there (a scrollbar)
const WHEEL_LEAVE_PX: f64 = 60.0; // after that, the edge works again once the pointer is this far away

#[derive(Default)]
pub struct EdgeState {
    pub tab_shown: bool,
    pub tab_shown_at: i64,
    pub tab_foreground: isize,
    pub tab_last_foreground: isize,
    pub tab_center_y: Option<i32>,
    tab_y: f64,
    tab_target_y: f64,
    tab_drawn_y: Option<i32>,
    last_inside_at: i64,
    pub dwell_start: i64,
    pub hold_line: bool,
    hold_y: Option<i32>,
    last_edge_frame: f64,
    tab_hide_timer: u64,
    edge_hide_timer: u64,
    glow_hide_timer: u64,
    /// the last wheel turn looked at (wheel::last)
    wheel_seen: i64,
    /// the wheel turned on the edge: the user is scrolling, not asking for the chat. No line or tab
    /// until the pointer has left the edge.
    pub scrolling: bool,
    /// self-test: a pretend cursor (x, y, a game holds the mouse)
    pub test_cursor: Option<(i32, i32, bool)>,
    /// how many times the edge has been looked at (the self-test sees the watch running)
    pub looks: u64,
}

fn edge_tick() {
    match core::with(|c| if c.quitting { None } else { Some(c.edge_step()) }) {
        Some(Some(0)) => frames::request(|_| edge_tick()), // every screen refresh while the tab or the line moves
        Some(Some(ms)) => {
            rt::after(ms, edge_tick);
        }
        Some(None) => {
            rt::after(200, edge_tick); // quitting, or handing over to the installer: if that fails, go on
        }
        None => {
            rt::after(16, edge_tick); // the app state was busy: try again
        }
    }
}

impl Core {
    pub fn edge_tick_soon(&mut self, ms: u64) {
        rt::after(ms, edge_tick);
    }

    /// How long the cursor has to be held against the edge before the tab comes out (0 = right away).
    fn hold_time(&self) -> i64 {
        (self.settings.f64("edgeHold").clamp(0.0, 10.0) * 1000.0).round() as i64
    }

    pub(crate) fn tab_size(&self, d: &Display) -> (i32, i32) {
        let n = self.enabled_apps().len() as f64;
        // the phone / screen icons of calls go under the apps, after a line (tab.css)
        let chips = self.call_chips().len() as f64;
        let calls = if chips > 0.0 { 10.0 + 40.0 * chips } else { 0.0 };
        ((TAB_W * d.ui).round() as i32, ((74.0 + 40.0 * n + calls) * d.ui).round() as i32)
    }

    /// One look at the cursor. Returns when to look again (0 = at the next screen refresh).
    pub fn edge_step(&mut self) -> u64 {
        self.edge.looks += 1;
        let next = self.edge_look();
        // the wheel only matters while the edge is being held or the tab is out
        let guard = self.settings.bool("edgeWheel");
        crate::wheel::listen(guard && (self.edge.dwell_start != 0 || self.edge.hold_line || self.edge.tab_shown));
        next
    }

    fn edge_look(&mut self) -> u64 {
        let mode = self.settings.str("edgeMode").to_string();
        if self.panel_state != PanelState::Hidden || mode == "off" {
            self.stop_hold(false);
            return 200;
        }
        let (px, py, test_captured) = match self.edge.test_cursor {
            Some((x, y, cap)) => (x, y, Some(cap)),
            None => {
                let (x, y) = win32::cursor_pos();
                (x, y, None)
            }
        };
        let auto = self.auto_display();
        // "Automatic": the edge counts on whichever monitor the mouse is on (while nothing is going
        // on at the edge yet). The dock moves there when the mouse arrives at its outer edge.
        if auto && !self.edge.tab_shown && !self.edge.hold_line && self.edge.dwell_start == 0 {
            if let Some(m) = self.display_at(px, py) {
                let reach = (EDGE_PX * m.scale).round().max(1.0) as i32;
                let at = if self.on_left() { px < m.bounds.x + reach } else { px >= m.bounds.right() - reach };
                if at && m.id != self.target_display().id && self.outer_edge_at(&m, py) {
                    self.use_display(&m);
                }
            }
        }
        let d = self.target_display();
        let b = d.bounds;
        let s = d.scale;
        let left = self.on_left();
        let right = b.right();
        let now = rt::epoch_ms();
        // Leave the corners alone: they hold window close buttons, menus, the Start button and the clock.
        let margin = (90.0 * s).max(b.h as f64 * 0.1).round() as i32;
        let zone_top = b.y + margin;
        let zone_bottom = d.work.bottom() - margin;
        let reach = ((if self.edge.dwell_start != 0 { HOLD_SLACK_PX } else { EDGE_PX }) * s).round().max(1.0) as i32;
        let at_edge = if left { px >= b.x && px < b.x + reach } else { px >= right - reach && px <= right };
        // The wheel turning with the pointer on the edge (or on the tab) is scrolling the page there,
        // not asking for the chat: the line and the tab go, and stay away until the pointer has left
        // the edge.
        let dist = if left { px - b.x } else { right - px };
        let on_screen_y = py >= b.y && py <= b.bottom();
        let wheel = crate::wheel::last();
        if wheel > self.edge.wheel_seen {
            self.edge.wheel_seen = wheel;
            let zone = if self.edge.tab_shown { TAB_W + WHEEL_ZONE_PX } else { WHEEL_ZONE_PX };
            if self.settings.bool("edgeWheel") && on_screen_y && dist >= 0 && dist < (zone * s).round() as i32 && !self.edge.scrolling {
                self.edge.scrolling = true;
                self.stop_hold(false);
                self.hide_tab(true);
                log!("edge: the wheel turned on the edge (scrolling): no tab until the pointer leaves it");
            }
        }
        if self.edge.scrolling && !(on_screen_y && dist >= 0 && dist < (WHEEL_LEAVE_PX * s).round() as i32) {
            self.edge.scrolling = false;
        }
        // With "Automatic", a seam between two monitors never counts: the mouse only passes it.
        let on_edge = at_edge && py >= zone_top && py <= zone_bottom && (!auto || self.outer_edge_at(&d, py)) && !self.edge.scrolling;
        // A game that has taken the mouse (pointer hidden, or held inside the game) pushes the pointer
        // against the screen edge whenever you aim or turn. That must never bring the tab out.
        let captured = (on_edge || self.edge.tab_shown) && test_captured.unwrap_or_else(win32::mouse_captured);

        if self.edge.tab_shown {
            if captured {
                self.hide_tab(true);
                return 60;
            }
            let fg = win32::foreground_window();
            if fg != self.edge.tab_last_foreground {
                // e.g. a topmost game was clicked and rose above the tab
                self.edge.tab_last_foreground = fg;
                win32::raise(self.tab.hwnd);
            }
            let r = win32::window_rect(self.tab.hwnd);
            let pad = (30.0 * s) as i32;
            let inside_x = if left { px >= b.x && px <= r.right() + pad } else { px >= r.x - pad && px <= right };
            let inside = inside_x && py >= r.y - pad && py <= r.bottom() + pad;
            if on_edge {
                self.follow_cursor(py); // slide along the edge with the cursor
                self.edge.last_inside_at = now;
            } else if inside {
                self.edge.last_inside_at = now;
            } else if now - self.edge.last_inside_at > TAB_LINGER_MS {
                self.hide_tab(false);
            }
            self.glide_tab();
            return if self.edge.tab_shown { 0 } else { 16 };
        }

        let busy_mouse = test_captured.is_none() && win32::mouse_button_down(); // dragging a window or a scrollbar
        if on_edge && !captured && !busy_mouse && !(mode == "no-fullscreen" && win32::is_fullscreen_app_active()) {
            if self.edge.dwell_start == 0 {
                self.edge.dwell_start = now;
            }
            let held = now - self.edge.dwell_start;
            let hold_ms = self.hold_time();
            if held >= if hold_ms > 0 { hold_ms } else { DWELL_MS } {
                self.stop_hold(true); // the line lights up and fades as the tab slides out
                self.show_tab(Some(py));
                return 0;
            }
            if hold_ms > 0 && held >= HOLD_LINE_MS {
                self.show_hold_line(py, held, hold_ms, &d);
            }
            return if self.edge.hold_line { 0 } else { 16 };
        }
        self.stop_hold(false);
        let near = dist >= 0 && dist < (250.0 * s) as i32 && on_screen_y;
        if near {
            40
        } else {
            110
        } // look more often only while the cursor is near the edge
    }

    fn clamp_tab_y(&self, y: f64, h: i32, d: &Display) -> f64 {
        let lo = d.bounds.y as f64 + 4.0 * d.scale;
        let hi = (d.work.bottom() - h) as f64 - 4.0 * d.scale;
        y.clamp(lo, hi.max(lo))
    }

    /// Put the tab centred on the cursor right away (when it appears, or when it grows / shrinks).
    pub fn place_tab(&mut self, cursor_y: Option<i32>) {
        let d = self.target_display();
        let (_, h) = self.tab_size(&d);
        let cy = cursor_y.unwrap_or(d.bounds.y + d.bounds.h / 2) as f64;
        self.edge.tab_y = self.clamp_tab_y(cy - h as f64 / 2.0, h, &d);
        self.edge.tab_target_y = self.edge.tab_y;
        self.edge.tab_drawn_y = None;
        self.move_tab(&d);
    }

    /// What's on the tab changed size (a call started or ended): fit it again while it's out.
    pub fn refit_tab(&mut self) {
        if self.edge.tab_shown {
            let cy = self.edge.tab_center_y;
            self.place_tab(cy);
        }
    }

    /// The cursor slides along the edge: the pill is pushed along so the cursor stays inside it
    /// (with a margin), instead of jumping to re-centre itself on the cursor.
    fn follow_cursor(&mut self, cursor_y: i32) {
        let d = self.target_display();
        let (_, h) = self.tab_size(&d);
        let m = (16.0 + TAB_FOLLOW_MARGIN) * d.ui;
        let top = self.edge.tab_target_y + m;
        let bottom = self.edge.tab_target_y + h as f64 - m;
        let cy = cursor_y as f64;
        if cy < top {
            self.edge.tab_target_y -= top - cy;
        } else if cy > bottom {
            self.edge.tab_target_y += cy - bottom;
        }
        self.edge.tab_target_y = self.clamp_tab_y(self.edge.tab_target_y, h, &d);
    }

    /// One frame: cover part of the way to the target. The share depends on the time since the last
    /// frame, so it glides the same at 60 Hz and at 300 Hz.
    fn glide_tab(&mut self) {
        let now = rt::now_ms();
        let dt = if self.edge.last_edge_frame > 0.0 { (now - self.edge.last_edge_frame).min(100.0) } else { 1000.0 / 60.0 };
        self.edge.last_edge_frame = now;
        let diff = self.edge.tab_target_y - self.edge.tab_y;
        if diff.abs() < 0.5 {
            self.edge.tab_y = self.edge.tab_target_y;
        } else {
            self.edge.tab_y += diff * (1.0 - (-dt / TAB_GLIDE_MS).exp());
        }
        let d = self.target_display();
        self.move_tab(&d);
    }

    fn move_tab(&mut self, d: &Display) {
        let (w, h) = self.tab_size(d);
        let y = self.edge.tab_y.round() as i32;
        if Some(y) == self.edge.tab_drawn_y {
            return;
        }
        self.edge.tab_drawn_y = Some(y);
        let x = if self.on_left() { d.bounds.x } else { d.bounds.right() - w };
        win32::set_bounds(self.tab.hwnd, Rect { x, y, w, h });
        self.edge.tab_center_y = Some(y + h / 2);
    }

    pub fn tab_bounds(&self) -> Rect {
        win32::window_rect(self.tab.hwnd)
    }

    pub fn show_tab(&mut self, cursor_y: Option<i32>) {
        rt::cancel(self.edge.tab_hide_timer);
        self.edge.tab_foreground = win32::foreground_window(); // the game / app the user is in right now
        self.edge.tab_last_foreground = self.edge.tab_foreground;
        self.place_tab(cursor_y);
        self.emit("tab", "tab:show", json!([self.ui_state()]));
        win32::show_inactive(self.tab.hwnd);
        win32::raise(self.tab.hwnd);
        self.edge.tab_shown = true;
        self.edge.tab_shown_at = rt::epoch_ms();
        self.edge.last_inside_at = self.edge.tab_shown_at;
        self.edge.last_edge_frame = 0.0;
        self.hide_glow_now();
        log!("tab show {:?}", win32::window_rect(self.tab.hwnd));
    }

    pub fn hide_tab(&mut self, instant: bool) {
        if !self.edge.tab_shown {
            return;
        }
        self.edge.tab_shown = false;
        log!("tab hide {}", if instant { "instant" } else { "linger" });
        self.emit("tab", "tab:hide", json!([instant]));
        rt::cancel(self.edge.tab_hide_timer);
        // Let the page draw its "tucked away" frame before the window disappears, so the next show
        // never flashes a stale frame.
        self.edge.tab_hide_timer = timer(if instant { 50 } else { 170 }, |c| {
            if !c.edge.tab_shown {
                win32::hide(c.tab.hwnd);
            }
            c.update_glow(false);
        });
    }

    /// The cursor is being held against the edge: the line grows from it towards the top and bottom
    /// of the screen and reaches both when the hold time is up (edge.js draws it at the screen's
    /// refresh rate; here it only learns where the cursor is).
    fn show_hold_line(&mut self, cursor_y: i32, held: i64, total: i64, d: &Display) {
        let s = d.ui; // edge.js works in its own CSS px
        let b = d.bounds;
        let y_dip = ((cursor_y - b.y) as f64 / s).round();
        if !self.edge.hold_line {
            rt::cancel(self.edge.edge_hide_timer);
            self.edge.hold_line = true;
            self.edge.hold_y = Some(cursor_y);
            let w = (EDGE_W * s).round() as i32;
            let x = if self.on_left() { b.x } else { b.right() - w };
            win32::set_bounds(self.edgewin.hwnd, Rect { x, y: b.y, w, h: b.h });
            self.emit("edge", "edge:start", json!([{ "y": y_dip, "held": held, "total": total, "side": self.settings.str("side") }]));
            win32::show_inactive(self.edgewin.hwnd);
            win32::raise(self.edgewin.hwnd);
            return;
        }
        if self.edge.hold_y != Some(cursor_y) {
            self.edge.hold_y = Some(cursor_y);
            self.emit("edge", "edge:move", json!([y_dip]));
        }
    }

    /// The hold ended: done (the tab comes out; the line lights up and fades) or given up (it
    /// shrinks back into the cursor).
    pub fn stop_hold(&mut self, done: bool) {
        self.edge.dwell_start = 0;
        if !self.edge.hold_line {
            return;
        }
        self.edge.hold_line = false;
        self.edge.hold_y = None;
        self.emit("edge", if done { "edge:done" } else { "edge:cancel" }, json!([]));
        rt::cancel(self.edge.edge_hide_timer);
        self.edge.edge_hide_timer = timer(if done { 360 } else { 260 }, |c| {
            if !c.edge.hold_line {
                win32::hide(c.edgewin.hwnd);
            }
        });
    }

    pub fn update_glow(&mut self, pulse: bool) {
        let show = self.settings.bool("glow") && self.total_unread() > 0 && self.panel_state == PanelState::Hidden && !self.edge.tab_shown;
        let visible = win32::is_visible(self.glow.hwnd);
        if !show {
            if visible && self.edge.glow_hide_timer == 0 {
                // fade out first (a fade already running is left alone)
                self.emit("glow", "glow:hide", json!([]));
                self.edge.glow_hide_timer = timer(240, |c| {
                    c.edge.glow_hide_timer = 0;
                    win32::hide(c.glow.hwnd);
                });
            }
            return;
        }
        let appear = !visible || self.edge.glow_hide_timer != 0;
        rt::cancel(self.edge.glow_hide_timer);
        self.edge.glow_hide_timer = 0;
        let d = self.target_display();
        let s = d.ui;
        let (gw, gh) = ((GLOW_W * s).round() as i32, (GLOW_H * s).round() as i32);
        let cy = self.edge.tab_center_y.unwrap_or(d.bounds.y + d.bounds.h / 2);
        let y = (cy - gh / 2).clamp(d.bounds.y, (d.work.bottom() - gh).max(d.bounds.y));
        let x = if self.on_left() { d.bounds.x } else { d.bounds.right() - gw };
        win32::set_bounds(self.glow.hwnd, Rect { x, y, w: gw, h: gh });
        let colors: Vec<&str> = self
            .enabled_apps()
            .into_iter()
            .filter(|id| self.shown_count(id) > 0)
            .flat_map(|id| apps::get(id).map(|a| a.colors.to_vec()).unwrap_or_default())
            .collect();
        self.emit("glow", "glow:state", json!([{ "colors": colors, "pulse": pulse, "side": self.settings.str("side"), "appear": appear }]));
        if !visible {
            win32::show_inactive(self.glow.hwnd);
        }
        win32::raise(self.glow.hwnd);
    }

    /// Out of the way at once (the panel or the tab takes its place).
    pub fn hide_glow_now(&mut self) {
        rt::cancel(self.edge.glow_hide_timer);
        self.edge.glow_hide_timer = 0;
        if win32::is_visible(self.glow.hwnd) {
            win32::hide(self.glow.hwnd);
        }
    }

    /// The tab was clicked without opening the panel (e.g. the too-early click guard): the game
    /// mustn't be left without focus.
    pub fn on_tab_focus(&mut self) {
        timer(200, |c| {
            if c.panel_state != PanelState::Hidden || win32::foreground_window() != c.tab.hwnd {
                return;
            }
            let fg = c.edge.tab_foreground;
            if fg != 0 && !c.is_ours(fg) {
                win32::restore_foreground(fg);
                if c.edge.tab_shown {
                    win32::raise(c.tab.hwnd); // a topmost game just came to the front; stay above it
                }
                log!("tab took focus without opening the panel; focus given back");
            }
        });
    }

    pub fn on_tab_open(&mut self, id: &str, call: bool) {
        let since = rt::epoch_ms() - self.edge.tab_shown_at;
        log!("tab click {id}{} {{\"sinceShown\":{since}}} {}", if call { " (call)" } else { "" }, self.snap());
        if !self.edge.tab_shown || since < 150 {
            log!("tab click ignored");
            return;
        }
        if let Some(hwnd) = self.tab_call_window(id, call) {
            self.hide_tab(true);
            let front = win32::restore_foreground(hwnd);
            log!("tab: back to the {id} call window ({})", if front { "in front" } else { "not in front" });
            return;
        }
        let target = if self.is_enabled(id) { id.to_string() } else { self.preferred_app() };
        self.open_panel(Some(&target), "tab");
    }

    /// A call icon of an app whose call has a window of its own (Messenger, Instagram): that window.
    pub fn tab_call_window(&self, id: &str, call: bool) -> Option<isize> {
        if !call {
            return None;
        }
        self.call_windows.iter().find(|(h, a)| a == id && win32::is_window(*h)).map(|(h, _)| *h)
    }
}
