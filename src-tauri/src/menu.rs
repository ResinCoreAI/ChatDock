//! ChatDock's own small menu window, one for all of them: the "+" list after the app tabs, a
//! right-click on an app tab, and a right-click inside a chat page (WebView2's own menu is replaced:
//! no "Save image as", "Print" or "Inspect" in a chat). It is a separate window because anything the
//! panel page draws below its header is covered by the chat itself.
//!
//! Flow: show_menu sends the items to ui/menu.html, the page answers with its size, then the window
//! goes beside the anchor (kept on the monitor) and takes the focus. A pick, Esc or a click
//! anywhere else closes it.

use serde_json::{json, Value};

use crate::{
    apps,
    core::{timer, Core, PanelState, Win},
    log, rt,
    win32::{self, Rect},
};

/// What the open menu is about.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Ctx {
    #[default]
    None,
    /// "+": the apps that are off
    Add,
    /// a right-click on this app's tab
    Tab(String),
    /// a right-click in this app's page
    Page(PageCtx),
}

/// Where in the page the right-click was (WebView2's ContextMenuTarget).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageCtx {
    pub app: String,
    pub link: String,
    pub selection: String,
    pub editable: bool,
    pub image: String,
}

#[derive(Default)]
pub struct Menu {
    pub win: Option<Win>,
    ctx: Ctx,
    /// where it goes (physical px): the point, and the top of the thing it hangs from, for showing
    /// above it when there's no room below
    anchor: (i32, i32, i32),
    shown: bool,
    shown_at: i64,
    /// the rows on show: (id, greyed out)
    offered: Vec<(String, bool)>,
}

/// A pick counts only for a row that is on show and not greyed out (the page is ours, but a late
/// click must not act on the next menu's context).
pub fn pick_ok(offered: &[(String, bool)], shown: bool, id: &str) -> bool {
    shown && offered.iter().any(|(o, off)| o == id && !off)
}

/// Paste brings at most this many characters (a huge clipboard would hang the page).
pub const PASTE_MAX: usize = 200_000;

pub fn paste_cap(text: &str) -> String {
    text.chars().take(PASTE_MAX).collect()
}

impl Menu {
    pub fn new(win: Win) -> Self {
        Menu { win: Some(win), ..Default::default() }
    }
}

/// One row: an id the pick comes back with, then what the page shows.
fn item(id: &str, label: String, icon: &str) -> Value {
    json!({ "id": id, "label": label, "icon": icon })
}

/// The rows of a right-click in a page, by id, in order ("-" = a line between groups).
pub fn page_ids(p: &PageCtx, can_back: bool, can_forward: bool) -> Vec<&'static str> {
    let mut ids = Vec::new();
    if !p.selection.is_empty() {
        ids.push("copy");
    }
    if p.editable {
        ids.extend(["paste", "selectall"]);
    }
    if !p.link.is_empty() {
        if !ids.is_empty() {
            ids.push("-");
        }
        ids.extend(["openlink", "copylink"]);
    }
    if !p.image.is_empty() && p.link.is_empty() {
        if !ids.is_empty() {
            ids.push("-");
        }
        ids.push("copyimage");
    }
    if !ids.is_empty() {
        ids.push("-");
    }
    if can_back {
        ids.push("back");
    }
    if can_forward {
        ids.push("forward");
    }
    ids.push("reload");
    ids
}

/// Where the menu goes: below and right of the point, moved in to stay on the monitor, or above the
/// thing it hangs from when it doesn't fit below. (all physical px)
pub fn place(at: (i32, i32), top: i32, size: (i32, i32), work: Rect) -> (i32, i32) {
    let (w, h) = size;
    let x = at.0.min(work.right() - w).max(work.x);
    let y = if at.1 + h > work.bottom() { (top - h).max(work.y) } else { at.1 };
    (x, y)
}

/// "Open link in browser": not to this PC however the link names it (127.0.0.1.nip.io): a plain
/// http name is looked up first, off the UI thread, as the chats' own requests are.
fn open_checked(app: String, link: String) {
    match apps::http_check(&apps::unshim(&link)) {
        apps::HttpCheck::Block => log!("{app}: a link to this PC not opened"),
        apps::HttpCheck::Allow => crate::core::later(move |c| c.open_external(&app, &link)),
        apps::HttpCheck::Resolve(host) => {
            let _ = std::thread::Builder::new().name("chatdock-lookup".into()).spawn(move || {
                if apps::resolves_local(&host) {
                    log!("{app}: a link to a name of this PC not opened");
                } else {
                    crate::core::later(move |c| c.open_external(&app, &link));
                }
            });
        }
    }
}

impl Core {
    /// "+" after the tabs: the apps that are off. (x, y: the button's bottom-left in the panel
    /// page's CSS px; top: its top)
    pub fn open_add_menu(&mut self, x: f64, y: f64, top: f64) {
        let items: Vec<Value> =
            apps::ids().filter(|id| !self.is_enabled(id)).filter_map(apps::get).map(|a| item(a.id, a.name.to_string(), a.icon)).collect();
        if items.is_empty() {
            return;
        }
        let (px, py, ptop) = self.panel_point(x, y, top);
        self.show_menu(Ctx::Add, items, (px, py, ptop));
    }

    /// A right-click on an app's tab: reload, sounds, close.
    pub fn open_tab_menu(&mut self, id: &str) {
        let Some(a) = apps::get(id) else { return };
        let sound = self.settings.app_prefs(id).get("sound").and_then(Value::as_bool).unwrap_or(true);
        let last = self.enabled_apps().len() <= 1;
        let mut close = item("close", self.tv("menu.closeTab", &[("name", a.name.to_string())]), "close");
        close["danger"] = json!(true);
        if last {
            close["disabled"] = json!(true);
            close["hint"] = json!(self.t("menu.lastTab"));
        }
        let items = vec![
            item("reload", self.t("menu.reload"), "reload"),
            item("sound", self.t(if sound { "menu.mute" } else { "menu.unmute" }), if sound { "volumeOff" } else { "volume" }),
            json!({ "sep": true }),
            close,
        ];
        let (x, y) = win32::cursor_pos();
        self.show_menu(Ctx::Tab(id.to_string()), items, (x, y, y));
    }

    /// A right-click in a chat page (from chats.rs, with WebView2's own menu turned away).
    pub fn open_page_menu(&mut self, p: PageCtx, can_back: bool, can_forward: bool) {
        let mut items = Vec::new();
        for id in page_ids(&p, can_back, can_forward) {
            items.push(match id {
                "-" => json!({ "sep": true }),
                "copy" => item(id, self.t("menu.copy"), "copy"),
                "paste" => item(id, self.t("menu.paste"), "paste"),
                "selectall" => item(id, self.t("menu.selectAll"), "selectAll"),
                "openlink" => item(id, self.t("menu.openLink"), "external"),
                "copylink" => item(id, self.t("menu.copyLink"), "link"),
                "copyimage" => item(id, self.t("menu.copyImage"), "link"),
                "back" => item(id, self.t("menu.back"), "chevronLeft"),
                "forward" => item(id, self.t("menu.forward"), "chevronRight"),
                _ => item(id, self.t("menu.reload"), "reload"),
            });
        }
        let (x, y) = win32::cursor_pos();
        self.show_menu(Ctx::Page(p), items, (x, y, y));
    }

    /// A point of the panel page (CSS px) on the screen.
    fn panel_point(&self, x: f64, y: f64, top: f64) -> (i32, i32, i32) {
        let r = win32::window_rect(self.panel.hwnd);
        let ui = self.display_at(r.x + r.w / 2, r.y + r.h / 2).unwrap_or_else(|| self.target_display()).ui;
        let px = |v: f64| (v * ui).round() as i32;
        (r.x + px(x), r.y + px(y), r.y + px(top))
    }

    fn show_menu(&mut self, ctx: Ctx, items: Vec<Value>, anchor: (i32, i32, i32)) {
        if self.menu.win.is_none() {
            return;
        }
        self.menu.ctx = ctx;
        self.menu.anchor = anchor;
        self.menu.offered = items
            .iter()
            .filter_map(|i| Some((i.get("id")?.as_str()?.to_string(), i.get("disabled").and_then(Value::as_bool).unwrap_or(false))))
            .collect();
        self.emit("menu", "menu:show", json!([{ "items": items }]));
    }

    /// The page drew the rows: their size (CSS px). Place the window and give it the focus.
    pub fn menu_size(&mut self, w: f64, h: f64) {
        let Some(win) = self.menu.win.as_ref() else { return };
        let hwnd = win.hwnd;
        let (x, y, top) = self.menu.anchor;
        let d = self.display_at(x, y).unwrap_or_else(|| self.target_display());
        let size = ((w * d.ui).round() as i32, (h * d.ui).round() as i32);
        let (mx, my) = place((x, y), top, size, d.work);
        win32::set_bounds(hwnd, Rect { x: mx, y: my, w: size.0, h: size.1 });
        win32::show_inactive(hwnd);
        win32::raise(hwnd);
        win32::force_foreground(hwnd);
        self.menu.shown = true;
        self.menu.shown_at = rt::epoch_ms();
    }

    /// A row was picked.
    pub fn menu_pick(&mut self, id: &str) {
        if !pick_ok(&self.menu.offered, self.menu.shown, id) {
            return;
        }
        let ctx = std::mem::take(&mut self.menu.ctx);
        self.close_menu(true);
        log!("menu: {id}");
        match ctx {
            Ctx::Add => {
                if apps::get(id).is_some() {
                    self.set_app_enabled(id, true);
                    self.set_active(id, true);
                }
            }
            Ctx::Tab(app) => match id {
                "reload" => self.reload_app(&app, false),
                "sound" => {
                    let on = self.settings.app_prefs(&app).get("sound").and_then(Value::as_bool).unwrap_or(true);
                    self.set_app_pref(&app, "sound", !on);
                }
                "close" => self.set_app_enabled(&app, false),
                _ => {}
            },
            Ctx::Page(p) => self.page_action(&p, id),
            Ctx::None => {}
        }
    }

    fn page_action(&mut self, p: &PageCtx, id: &str) {
        let app = p.app.as_str();
        match id {
            "copy" => {
                win32::set_clipboard_text(&p.selection);
            }
            "copylink" => {
                win32::set_clipboard_text(&apps::unshim(&p.link));
            }
            "copyimage" => {
                win32::set_clipboard_text(&p.image);
            }
            "openlink" => open_checked(app.to_string(), p.link.clone()),
            "paste" => {
                if let Some(text) = win32::clipboard_text().map(|t| paste_cap(&t)) {
                    let js = format!("document.execCommand('insertText', false, {})", serde_json::to_string(&text).unwrap_or_default());
                    self.chats.execute(app, &js, |_| {});
                }
            }
            "selectall" => self.chats.execute(app, "document.execCommand('selectAll')", |_| {}),
            "back" => self.chats.go_back(app),
            "forward" => self.chats.go_forward(app),
            "reload" => self.reload_app(app, false),
            _ => {}
        }
    }

    /// picked: a row was chosen or Esc pressed, the chat gets the keyboard back. Otherwise the
    /// menu lost the focus to a click somewhere else: that click decides about the chat too.
    pub fn close_menu(&mut self, picked: bool) {
        let Some(win) = self.menu.win.as_ref() else { return };
        if !self.menu.shown {
            return;
        }
        self.menu.shown = false;
        self.menu.ctx = Ctx::None;
        self.menu.offered.clear();
        win32::hide(win.hwnd);
        self.emit("menu", "menu:hide", json!([]));
        if self.panel_state != PanelState::Open {
            return;
        }
        if picked {
            self.focus_panel();
        } else {
            timer(60, |c| c.check_auto_hide(false));
        }
    }

    /// The menu window lost the focus.
    pub fn on_menu_blur(&mut self) {
        // (the focus it was just given can bounce once while Windows hands it over)
        if rt::epoch_ms() - self.menu.shown_at < 150 {
            if let Some(w) = self.menu.win.as_ref() {
                let hwnd = w.hwnd;
                timer(30, move |c| {
                    if c.menu.shown && win32::foreground_window() != hwnd {
                        c.close_menu(false);
                    }
                });
            }
            return;
        }
        self.close_menu(false);
    }
}

#[cfg(test)]
mod tests {
    use super::{page_ids, place, PageCtx};
    use crate::win32::Rect;

    #[test]
    fn a_page_menu_offers_what_was_clicked() {
        let plain = PageCtx::default();
        assert_eq!(page_ids(&plain, false, false), vec!["reload"]);
        assert_eq!(page_ids(&plain, true, true), vec!["back", "forward", "reload"]);
        let typing = PageCtx { editable: true, selection: "hi".into(), ..Default::default() };
        assert_eq!(page_ids(&typing, true, false), vec!["copy", "paste", "selectall", "-", "back", "reload"]);
        let link = PageCtx { link: "https://example.com".into(), image: "https://example.com/a.png".into(), ..Default::default() };
        assert_eq!(page_ids(&link, false, false), vec!["openlink", "copylink", "-", "reload"]);
        let image = PageCtx { image: "https://example.com/a.png".into(), ..Default::default() };
        assert_eq!(page_ids(&image, false, false), vec!["copyimage", "-", "reload"]);
    }

    #[test]
    fn only_a_row_that_was_offered_can_be_picked() {
        use super::pick_ok;
        let offered = vec![("reload".to_string(), false), ("close".to_string(), true)];
        assert!(pick_ok(&offered, true, "reload"));
        assert!(!pick_ok(&offered, true, "close"), "a greyed-out row");
        assert!(!pick_ok(&offered, true, "paste"), "not in this menu");
        assert!(!pick_ok(&offered, false, "reload"), "the menu isn't open");
    }

    #[test]
    fn paste_is_capped() {
        assert_eq!(super::paste_cap("abc"), "abc");
        let big = "ก".repeat(super::PASTE_MAX + 10);
        assert_eq!(super::paste_cap(&big).chars().count(), super::PASTE_MAX);
    }

    #[test]
    fn the_menu_stays_on_the_monitor() {
        let work = Rect { x: 0, y: 0, w: 1920, h: 1040 };
        assert_eq!(place((100, 50), 20, (200, 300), work), (100, 50), "below and right of the point");
        assert_eq!(place((1850, 50), 20, (200, 300), work), (1720, 50), "moved in from the right edge");
        assert_eq!(place((100, 900), 880, (200, 300), work), (100, 580), "above it when there's no room below");
        assert_eq!(place((-50, 10), 0, (200, 2000), work), (0, 0), "never off the top or left");
    }
}
