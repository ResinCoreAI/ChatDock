//! ChatDock's own message pop-ups: a small always-on-top window in a screen corner that also shows
//! over borderless games (Windows' own toasts are muted while you play). One window holds a stack of
//! cards; it is shown without taking focus, so a game keeps its keyboard and mouse.
//! Game mode (Settings: popupQuietFullscreen): while a game or a video fills the screen the pop-ups
//! wait, and when it's gone one card says who wrote (one waiting pop-up just shows as itself).

use serde_json::{json, Value};

use crate::{
    apps,
    core::{timer, Core},
    log, rt,
    win32::{self, Display, Rect},
};

const CARD_W: f64 = 360.0;
const PAD: f64 = 16.0; // transparent room around the cards for their shadow (keep in sync with toast.css)
const EDGE: f64 = 12.0; // gap between the cards and the screen edge / the open chat panel
const MAX_QUEUE: usize = 30;
/// Game mode: conversations kept for the card after the game, and how often it looks whether the
/// game is still in front (it has to be gone this long: a quick Alt+Tab back doesn't count).
const MAX_HELD: usize = 20;
const GAME_CHECK_MS: u64 = 1000;
const GAME_GONE_MS: i64 = 1500;
/// rows the card after the game shows (the rest: "and N more")
const SUMMARY_ROWS: usize = 4;

/// What a site notification (or the unread count) turns into.
pub struct Fields {
    pub title: String,
    pub body: String,
    pub icon: String,
    pub tag: String,
    pub source: u64, // WebView2 notification to click through to (0 = none)
    /// the small line after the app's name ("Server · #channel"); "" = "just messaged"
    pub meta: String,
    /// the last line; "" = "Click to open this chat"
    pub hint: String,
    /// the conversation a click opens (read from the app's chat list)
    pub target: Option<crate::inbox::Latest>,
}

/// A pop-up game mode kept for later: one per conversation (the newest of it, and how many came).
#[derive(Clone, Debug)]
pub struct Held {
    pub app: String,
    tag: String,
    who: String,
    text: String,
    icon: String,
    meta: String,
    source: u64,
    target: Option<crate::inbox::Latest>,
    chime: bool,
    pub n: u32,
}

#[derive(Clone)]
pub struct Item {
    key: String,
    pub app_id: String,
    app_name: String,
    icon_name: String,
    accent: String,
    title: String,
    body: String,
    icon: String,
    meta: String,
    hint: String,
    tag: String,
    pub source: u64,
    pub chime: bool,
    pub action: String,
    pub target: Option<crate::inbox::Latest>,
    /// the card after a game: who wrote meanwhile, one row each
    pub rows: Vec<Held>,
    remaining: i64,
    timer: u64,
    started_at: i64,
}

impl Item {
    #[allow(clippy::too_many_arguments)]
    pub fn new_own(
        app_id: &str,
        app_name: &str,
        icon_name: &str,
        accent: &str,
        meta: String,
        title: String,
        body: String,
        hint: String,
        tag: &str,
        action: &str,
    ) -> Self {
        Item {
            key: String::new(),
            app_id: app_id.into(),
            app_name: app_name.into(),
            icon_name: icon_name.into(),
            accent: accent.into(),
            title,
            body,
            icon: String::new(),
            meta,
            hint,
            tag: tag.into(),
            source: 0,
            chime: false,
            action: action.into(),
            target: None,
            rows: Vec::new(),
            remaining: 0,
            timer: 0,
            started_at: 0,
        }
    }
}

#[derive(Default)]
pub struct Toasts {
    items: Vec<Item>, // newest first
    content_height: f64,
    hovered: bool,
    fg_before: isize, // window that was in front of the pop-ups (usually the game)
    last_fg: isize,
    watch_timer: u64,
    seq: u64,
    pending_show: bool,
    show_fallback: u64,
    hide_fallback: u64,
    /// "Where the mouse is" (Settings: pop-ups show on): the monitor the mouse was on when the first
    /// of the pop-ups on screen came, so the stack doesn't jump around after it
    pub display: String,
    /// game mode: what waits for the end of the game (newest first), the look at whether it's still
    /// in front, and since when it hasn't been
    pub held: Vec<Held>,
    game_timer: u64,
    game_gone_at: i64,
    /// self-test: a pretend game in front (or not), and the pop-ups shown off screen
    pub test_game: Option<bool>,
    pub offscreen: bool,
}

pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

impl Core {
    fn lifetime_ms(&self) -> i64 {
        self.settings.i64("popupDuration").max(0) * 1000 // 0 = stays until dismissed
    }

    fn max_visible(&self) -> usize {
        self.settings.i64("popupMax").clamp(1, 5) as usize
    }

    /// A message pop-up for a chat app (in game mode, while a game fills the screen: kept for later).
    pub fn popup(&mut self, id: &str, f: Fields) {
        if apps::get(id).is_none() {
            return;
        }
        if self.game_holds() {
            self.hold_popup(id, f);
            return;
        }
        self.show_popup(id, f);
    }

    fn show_popup(&mut self, id: &str, f: Fields) {
        let Some(a) = apps::get(id) else { return };
        let mut item = Item::new_own(
            id,
            a.name,
            a.icon,
            a.colors[a.colors.len() - 1],
            if f.meta.is_empty() { self.t("toast.justMessaged") } else { f.meta },
            f.title,
            f.body,
            if f.hint.is_empty() { self.t("toast.clickToOpen") } else { f.hint },
            &f.tag,
            "",
        );
        item.icon = f.icon;
        item.source = f.source;
        item.target = f.target;
        item.chime = self.settings.bool("popupSound") && self.settings.app_pref(id, "chime");
        self.toasts_push(item);
    }

    // -----------------------------------------------------------------------------------------
    // Game mode
    // -----------------------------------------------------------------------------------------
    /// Game mode is on and a game (or a video) fills the screen: pop-ups wait.
    pub fn game_holds(&self) -> bool {
        self.settings.bool("popupQuietFullscreen") && self.toasts.test_game.unwrap_or_else(win32::game_in_front)
    }

    fn hold_popup(&mut self, id: &str, f: Fields) {
        let tag = if f.tag.is_empty() { format!("{id}:{}", f.title) } else { f.tag };
        let n = match self.toasts.held.iter().position(|h| h.tag == tag) {
            Some(i) => self.toasts.held.remove(i).n + 1,
            None => 1,
        };
        let chime = self.settings.bool("popupSound") && self.settings.app_pref(id, "chime");
        let held = Held {
            app: id.into(),
            tag,
            who: f.title,
            text: f.body,
            icon: f.icon,
            meta: f.meta,
            source: f.source,
            target: f.target,
            chime,
            n,
        };
        self.toasts.held.insert(0, held);
        self.toasts.held.truncate(MAX_HELD);
        log!("game mode: a pop-up from {id} waits ({} waiting)", self.toasts.held.len());
        if self.toasts.game_timer == 0 {
            self.toasts.game_gone_at = 0;
            self.toasts.game_timer = timer(GAME_CHECK_MS, |c| c.game_tick());
        }
    }

    /// While pop-ups wait: is the game still in front? Gone a moment (not a quick look elsewhere and
    /// back), or game mode switched off: they come.
    fn game_tick(&mut self) {
        self.toasts.game_timer = 0;
        if self.toasts.held.is_empty() {
            return;
        }
        if self.game_holds() {
            self.toasts.game_gone_at = 0;
        } else {
            let now = rt::epoch_ms();
            if self.toasts.game_gone_at == 0 {
                self.toasts.game_gone_at = now;
            } else if now - self.toasts.game_gone_at >= GAME_GONE_MS {
                self.release_held();
                return;
            }
        }
        self.toasts.game_timer = timer(GAME_CHECK_MS, |c| c.game_tick());
    }

    /// After the game: what waited and is still news, in one card (a single one shows as itself).
    fn release_held(&mut self) {
        let held = std::mem::take(&mut self.toasts.held);
        let mut rows: Vec<Held> = held.into_iter().filter(|h| self.still_news(h)).collect();
        log!("game mode: the game is gone, {} conversations waited", rows.len());
        for h in rows.iter_mut().filter(|h| h.tag.ends_with(":count")) {
            h.n = self.counts.get(&h.app).copied().unwrap_or(h.n).max(1); // an unread number: as it is now
        }
        if rows.len() == 1 {
            let h = rows.remove(0);
            let meta = if h.meta.is_empty() && h.n > 1 { self.tv("toast.unread", &[("n", h.n.to_string())]) } else { h.meta };
            let fields = Fields {
                title: h.who,
                body: h.text,
                icon: h.icon,
                tag: h.tag,
                source: h.source,
                meta,
                hint: String::new(),
                target: h.target,
            };
            self.show_popup(&h.app, fields);
            return;
        }
        if rows.is_empty() {
            return;
        }
        let total: u32 = rows.iter().map(|h| h.n).sum();
        let mut item = Item::new_own(
            "chatdock",
            "ChatDock",
            "logo",
            "#8b5cf6",
            self.tv("game.count", &[("n", total.to_string())]),
            self.t("game.title"),
            String::new(),
            String::new(),
            "chatdock:game",
            "game",
        );
        item.chime = rows.iter().any(|h| h.chime);
        item.rows = rows;
        self.toasts_push(item);
        // more to read than one message: it stays longer
        if let Some(it) = self.toasts.items.first_mut().filter(|it| it.action == "game" && it.remaining > 0) {
            it.remaining = (it.remaining * 2).max(15_000);
            if !self.toasts.hovered && self.ready.contains("toasts") {
                self.toast_start(0);
            }
        }
    }

    /// A pop-up that waited is still news: its app may still pop up, its chat isn't on screen, and
    /// (an unread number's) something is still unread.
    fn still_news(&self, h: &Held) -> bool {
        self.popup_allowed(Some(&h.app))
            && !self.app_on_screen(&h.app)
            && (!h.tag.ends_with(":count") || self.counts.get(&h.app).copied().unwrap_or(0) > 0)
    }

    /// Pop-ups waiting for the end of the game (the self-test).
    pub fn held_count(&self) -> usize {
        self.toasts.held.len()
    }

    /// ChatDock telling the user something (instead of a tray balloon).
    pub fn notice(&mut self, title: &str, body: &str) {
        let item = Item::new_own(
            "chatdock",
            "ChatDock",
            "logo",
            "#8b5cf6",
            String::new(),
            title.into(),
            body.into(),
            String::new(),
            "chatdock:notice",
            "",
        );
        self.toasts_push(item);
    }

    /// A short word from ChatDock that goes by itself (e.g. "Discord: mic off" after a voice key),
    /// shown even when message pop-ups are off: the user just asked for it.
    pub fn notice_brief(&mut self, title: &str, body: &str, ms: i64) {
        let item = Item::new_own(
            "chatdock",
            "ChatDock",
            "logo",
            "#8b5cf6",
            String::new(),
            title.into(),
            body.into(),
            String::new(),
            "chatdock:brief",
            "",
        );
        self.toasts_push(item);
        if let Some(it) = self.toasts.items.first_mut().filter(|it| it.tag == "chatdock:brief") {
            it.remaining = ms;
        }
        if !self.toasts.hovered {
            self.toast_start(0); // with its own, shorter time
        }
    }

    pub fn toasts_push(&mut self, mut item: Item) {
        if !self.toasts_visible() {
            let (x, y) = win32::cursor_pos();
            self.toasts.display = self.display_at(x, y).map(|d| d.id).unwrap_or_default();
        }
        if !item.tag.is_empty() {
            if let Some(same) = self.toasts.items.iter().find(|x| x.tag == item.tag).map(|x| x.key.clone()) {
                self.toast_remove(&same, false); // an update of the same conversation replaces the old card
            }
        }
        self.toasts.seq += 1;
        item.key = format!("t{}", self.toasts.seq);
        item.remaining = self.lifetime_ms();
        self.toasts.items.insert(0, item);
        while self.toasts.items.len() > MAX_QUEUE {
            if let Some(old) = self.toasts.items.pop() {
                rt::cancel(old.timer);
            }
        }
        // its time runs once the pop-up page can show it (at start it may still be loading)
        if !self.toasts.hovered && self.ready.contains("toasts") {
            self.toast_start(0);
        }
        self.toasts_render(true);
    }

    fn toasts_send(&self, visible: &[Item], more: usize, is_new: bool) {
        let items: Vec<Value> = visible
            .iter()
            .map(|it| {
                // the card after a game: a row per conversation
                let rows: Vec<Value> = it
                    .rows
                    .iter()
                    .take(SUMMARY_ROWS)
                    .map(|h| {
                        let a = apps::get(&h.app);
                        json!({
                            "appId": h.app, "appName": a.map(|a| a.name).unwrap_or(""), "iconName": a.map(|a| a.icon).unwrap_or("logo"),
                            "who": h.who, "text": h.text, "icon": h.icon, "meta": h.meta, "n": h.n,
                        })
                    })
                    .collect();
                let more = it.rows.len().saturating_sub(SUMMARY_ROWS);
                json!({
                    "key": it.key, "appId": it.app_id, "appName": it.app_name, "iconName": it.icon_name, "accent": it.accent,
                    "title": it.title, "body": it.body, "icon": it.icon, "meta": it.meta, "hint": it.hint,
                    "rows": rows, "moreRows": more, "moreLabel": self.tv("game.more", &[("n", more.to_string())]),
                })
            })
            .collect();
        let chime = is_new && self.toasts.items.first().is_some_and(|it| it.chime);
        self.emit(
            "toasts",
            "toasts",
            json!([{
                "items": items,
                "more": more,
                "labels": { "more": self.tv("toast.more", &[("n", more.to_string())]), "close": self.t("toast.close") },
                "lang": crate::i18n::locale(&self.lang),
                "position": self.settings.str("popupPosition"),
                "shown": win32::is_visible(self.toastwin.hwnd),
                "chime": chime,
            }]),
        );
    }

    fn toasts_render(&mut self, is_new: bool) {
        if !self.ready.contains("toasts") {
            return; // the page sends 'ui:ready' and gets everything then
        }
        if self.toasts.items.is_empty() {
            if win32::is_visible(self.toastwin.hwnd) && !self.toasts.pending_show {
                // The last cards slide away first; the page says 'toast:empty' when they are gone.
                self.toasts_send(&[], 0, false);
                rt::cancel(self.toasts.hide_fallback);
                self.toasts.hide_fallback = timer(900, |c| c.toasts_hide());
            } else {
                self.toasts_hide();
            }
            return;
        }
        rt::cancel(self.toasts.hide_fallback);
        let n = self.max_visible().min(self.toasts.items.len());
        let visible: Vec<Item> = self.toasts.items[..n].to_vec();
        let more = self.toasts.items.len() - n;
        self.toasts_send(&visible, more, is_new);
        if !win32::is_visible(self.toastwin.hwnd) && !self.toasts.pending_show {
            // Wait for the page to report the stack's height, so the window never shows at a wrong size.
            let fg = win32::foreground_window();
            if fg != 0 && fg != self.toastwin.hwnd {
                self.toasts.fg_before = fg;
            }
            self.toasts.pending_show = true;
            rt::cancel(self.toasts.show_fallback);
            self.toasts.show_fallback = timer(150, |c| c.toasts_reveal()); // in case the size report is slow
        } else if win32::is_visible(self.toastwin.hwnd) {
            win32::raise(self.toastwin.hwnd);
        }
    }

    /// The conversation the newest pop-up of `app` opens (the self-test).
    pub(crate) fn toasts_target(&self, app: &str) -> Option<crate::inbox::Latest> {
        self.toasts.items.iter().find(|it| it.app_id == app).and_then(|it| it.target.clone())
    }

    /// Pop-ups whose time hasn't started yet (the self-test).
    pub(crate) fn toasts_unstarted(&self) -> usize {
        self.toasts.items.iter().filter(|it| it.timer == 0).count()
    }

    pub fn toasts_ready(&mut self) {
        if !self.toasts.items.is_empty() {
            // pop-ups made before the page was ready: their time starts now
            for i in 0..self.toasts.items.len() {
                if self.toasts.items[i].timer == 0 && !self.toasts.hovered {
                    self.toast_start(i);
                }
            }
            self.toasts_render(false);
        }
    }

    fn toasts_reveal(&mut self) {
        rt::cancel(self.toasts.show_fallback);
        if !self.toasts.pending_show {
            return;
        }
        self.toasts.pending_show = false;
        if self.toasts.items.is_empty() {
            return;
        }
        self.toasts_place();
        win32::show_inactive(self.toastwin.hwnd); // never takes focus from the game
        win32::raise(self.toastwin.hwnd);
        self.toasts_watch();
    }

    /// The monitor pop-ups show on (Settings): the chat's (the default), the main one, or the one the
    /// mouse was on when they started.
    pub fn toast_display(&self) -> Display {
        let list = win32::displays();
        match self.settings.str("popupDisplay") {
            "main" => list.iter().find(|d| d.primary).cloned(),
            "mouse" => list.iter().find(|d| d.id == self.toasts.display).cloned(),
            _ => None,
        }
        .unwrap_or_else(|| self.target_display())
    }

    fn toasts_place(&mut self) {
        if self.toasts.items.is_empty() {
            return;
        }
        let d = self.toast_display();
        let s = d.ui;
        let wa = d.work;
        let w = ((CARD_W + PAD * 2.0) * s).round() as i32;
        let content = if self.toasts.content_height > 0.0 { self.toasts.content_height } else { 120.0 };
        let h = ((content.max(60.0)) * s).round().min(wa.h as f64) as i32;
        let pos = self.settings.str("popupPosition").to_string();
        let on_left = pos.ends_with("left");
        let (edge, pad) = ((EDGE * s).round() as i32, (PAD * s).round() as i32);
        // the open chat: pop-ups go beside it, never on top of it
        let panel = if self.panel_state.showing() && d.id == self.target_display().id { Some(self.panel_geometry(&d)) } else { None };
        let x = if on_left {
            let mut x = wa.x + edge - pad;
            if let Some(p) = panel.filter(|p| p.x <= wa.x + 1) {
                x = x.max(p.right() + edge - pad);
            }
            x
        } else {
            let mut right = wa.right() - edge + pad;
            if let Some(p) = panel.filter(|p| p.right() >= wa.right() - 1) {
                right = right.min(p.x - edge + pad);
            }
            right - w
        };
        let y = if pos.starts_with("bottom") { wa.bottom() - edge + pad - h } else { wa.y + edge - pad };
        let x = if self.toasts.offscreen { -30_000 } else { x }; // (the self-test: shown, but not on the screen)
        win32::set_bounds(self.toastwin.hwnd, Rect { x, y, w, h });
    }

    fn toast_start(&mut self, index: usize) {
        let lifetime_zero;
        {
            let Some(it) = self.toasts.items.get_mut(index) else { return };
            rt::cancel(it.timer);
            it.timer = 0;
            it.started_at = rt::epoch_ms();
            lifetime_zero = it.remaining <= 0;
        }
        if lifetime_zero {
            return; // stays until clicked or closed
        }
        let it = &mut self.toasts.items[index];
        let key = it.key.clone();
        it.timer = timer(it.remaining.max(1500) as u64, move |c| {
            c.toast_remove(&key, true);
        });
    }

    fn set_hovered(&mut self, on: bool) {
        if on == self.toasts.hovered {
            return;
        }
        self.toasts.hovered = on;
        let now = rt::epoch_ms();
        for i in 0..self.toasts.items.len() {
            if on {
                let it = &mut self.toasts.items[i];
                if it.timer != 0 {
                    rt::cancel(it.timer);
                    it.timer = 0;
                    it.remaining -= now - it.started_at;
                }
            } else {
                let it = &mut self.toasts.items[i];
                if it.remaining > 0 {
                    it.remaining = it.remaining.max(2500);
                }
                self.toast_start(i);
            }
        }
    }

    fn toast_remove(&mut self, key: &str, rerender: bool) -> Option<Item> {
        let i = self.toasts.items.iter().position(|x| x.key == key)?;
        let it = self.toasts.items.remove(i);
        rt::cancel(it.timer);
        if rerender {
            self.toasts_render(false);
        }
        Some(it)
    }

    /// The app's chats were read (opened, or read elsewhere): its pop-ups go, the ones waiting for
    /// the end of a game too, and its rows in the card after one.
    pub fn toasts_dismiss_app(&mut self, app: &str) {
        self.toasts.held.retain(|h| h.app != app);
        self.toasts_drop(|it| it.app_id == app, |h| h.app == app);
    }

    /// The site closed its notification (read elsewhere): its pop-up goes, waiting or shown.
    pub fn toasts_dismiss_source(&mut self, app: &str, source: u64) {
        self.toasts.held.retain(|h| !(h.app == app && h.source == source));
        self.toasts_drop(|it| it.app_id == app && it.source == source, |h| h.app == app && h.source == source);
    }

    /// Takes away the cards `card` picks and the rows `row` picks (a card after a game left with no
    /// rows goes too); redraws when anything went.
    fn toasts_drop(&mut self, card: impl Fn(&Item) -> bool, row: impl Fn(&Held) -> bool) {
        let size = |items: &[Item]| items.len() + items.iter().map(|it| it.rows.len()).sum::<usize>();
        let before = size(&self.toasts.items);
        for it in &mut self.toasts.items {
            it.rows.retain(|h| !row(h));
        }
        self.toasts.items.retain(|it| {
            let goes = card(it) || (it.action == "game" && it.rows.is_empty());
            if goes {
                rt::cancel(it.timer);
            }
            !goes
        });
        if size(&self.toasts.items) != before {
            self.recount_game_cards();
            self.toasts_render(false);
        }
    }

    /// The card after a game says how many came in all: again once a row has gone.
    fn recount_game_cards(&mut self) {
        for i in 0..self.toasts.items.len() {
            if self.toasts.items[i].action == "game" {
                let total: u32 = self.toasts.items[i].rows.iter().map(|h| h.n).sum();
                self.toasts.items[i].meta = self.tv("game.count", &[("n", total.to_string())]);
            }
        }
    }

    pub fn toasts_dismiss_all(&mut self) {
        self.toasts.held.clear();
        for it in &self.toasts.items {
            rt::cancel(it.timer);
        }
        self.toasts.items.clear();
        self.toasts_render(false);
    }

    fn toasts_hide(&mut self) {
        rt::cancel(self.toasts.watch_timer);
        self.toasts.watch_timer = 0;
        rt::cancel(self.toasts.show_fallback);
        rt::cancel(self.toasts.hide_fallback);
        self.toasts.pending_show = false;
        self.toasts.hovered = false;
        self.toasts.content_height = 0.0;
        if win32::is_visible(self.toastwin.hwnd) {
            win32::hide(self.toastwin.hwnd);
            log!("pop-ups hidden");
        }
    }

    fn toasts_return_focus(&mut self) {
        let fg = self.toasts.fg_before;
        if fg != 0 && fg != self.toastwin.hwnd {
            win32::restore_foreground(fg);
            if win32::is_visible(self.toastwin.hwnd) {
                win32::raise(self.toastwin.hwnd);
            }
        }
    }

    /// Safety net: a click on the pop-ups that didn't open the chat (their see-through edge, a
    /// ChatDock notice) must not leave the game unfocused, nor the chat: with the pop-ups in front,
    /// a click elsewhere would never reach the chat, and it would stay over the game.
    pub fn on_toast_focus(&mut self) {
        timer(250, |c| {
            if win32::foreground_window() != c.toastwin.hwnd {
                return;
            }
            if c.panel_state == crate::core::PanelState::Open {
                c.focus_panel();
            } else if !c.panel_state.showing() {
                c.toasts_return_focus();
            }
        });
    }

    /// While pop-ups are up: remember the latest window in front, and stay above topmost games.
    fn toasts_watch(&mut self) {
        rt::cancel(self.toasts.watch_timer);
        self.toasts.last_fg = win32::foreground_window();
        self.toasts.watch_timer = rt::after(400, toast_watch_tick);
    }

    pub fn toasts_reposition(&mut self) {
        if win32::is_visible(self.toastwin.hwnd) {
            self.toasts_place();
        }
    }

    /// Pop-up settings changed (corner, how many at once): redraw what is on screen.
    pub fn toasts_refresh(&mut self) {
        if !self.toasts.items.is_empty() {
            self.toasts_render(false);
        }
    }

    pub fn toasts_visible(&self) -> bool {
        win32::is_visible(self.toastwin.hwnd)
    }

    pub fn toasts_count(&self) -> usize {
        self.toasts.items.len()
    }

    pub fn toasts_bounds(&self) -> Rect {
        win32::window_rect(self.toastwin.hwnd)
    }

    pub fn on_toast_message(&mut self, channel: &str, args: &[Value]) {
        let key = args.first().and_then(Value::as_str).unwrap_or("").to_string();
        match channel {
            "toast:size" => {
                if let Some(h) = args.first().and_then(Value::as_f64).filter(|h| h.is_finite()) {
                    self.toasts.content_height = h.ceil();
                    self.toasts_place();
                    if self.toasts.pending_show {
                        self.toasts_reveal();
                    }
                }
            }
            // The last card has finished sliding away.
            "toast:empty" => {
                log!("pop-ups: the last one has slid away ({} left)", self.toasts.items.len());
                if self.toasts.items.is_empty() {
                    self.toasts_hide();
                }
            }
            "toast:click" => {
                if let Some(it) = self.toast_remove(&key, true) {
                    self.open_from_toast(it);
                }
            }
            // a row of the card after a game: that chat (the other rows stay)
            "toast:row" => {
                let i = args.get(1).and_then(Value::as_u64).unwrap_or(u64::MAX) as usize;
                let Some(pos) = self.toasts.items.iter().position(|x| x.key == key) else { return };
                if i >= self.toasts.items[pos].rows.len() {
                    return;
                }
                let row = self.toasts.items[pos].rows.remove(i);
                if self.toasts.items[pos].rows.is_empty() {
                    self.toast_remove(&key, false);
                }
                self.recount_game_cards();
                self.toasts_render(false);
                self.toast_foreground = self.toasts.fg_before;
                self.open_message(&row.app, row.source, row.target);
            }
            "toast:more" => {
                if let Some(first) = self.toasts.items.first().map(|x| x.key.clone()) {
                    if let Some(it) = self.toast_remove(&first, true) {
                        self.open_from_toast(it);
                    }
                }
            }
            "toast:dismiss" => {
                let was_in_front = win32::foreground_window() == self.toastwin.hwnd;
                self.toast_remove(&key, true);
                if was_in_front {
                    self.toasts_return_focus();
                }
            }
            "toast:hover" => {
                let on = args.first().and_then(Value::as_bool).unwrap_or(false);
                self.set_hovered(on);
            }
            _ => {}
        }
    }

    /// A pop-up was clicked: open the chat on that app and let the site open that conversation (the
    /// card after a game: its newest one).
    fn open_from_toast(&mut self, it: Item) {
        self.toast_foreground = self.toasts.fg_before;
        if it.action == "update" || it.action == "whatsnew" {
            self.open_settings("updates", "toast");
            return;
        }
        if it.action == "game" {
            if let Some(h) = it.rows.into_iter().next() {
                self.open_message(&h.app, h.source, h.target);
            }
            return;
        }
        if it.app_id == "chatdock" {
            return;
        }
        self.open_message(&it.app_id, it.source, it.target);
    }

    /// The chat on `app`, and in it the conversation a pop-up was about: the site's own notification
    /// clicked (it opens the conversation itself), or the one read from its chat list.
    fn open_message(&mut self, app: &str, source: u64, target: Option<crate::inbox::Latest>) {
        if !self.is_enabled(app) {
            return;
        }
        if source != 0 {
            self.click_notification(source);
        }
        if self.settings_mode {
            self.settings_mode = false;
            self.identify_close();
            self.layout_views();
        }
        log!("pop-up clicked {app}");
        self.open_panel(Some(app), "toast");
        if target.is_some() {
            self.open_conversation(app, target); // straight into who wrote
        }
    }
}

fn toast_watch_tick() {
    let ran = crate::core::with(|c| {
        if !win32::is_visible(c.toastwin.hwnd) {
            c.toasts.watch_timer = 0;
            return;
        }
        let fg = win32::foreground_window();
        if fg != 0 && fg != c.toastwin.hwnd {
            c.toasts.fg_before = fg;
        }
        if fg != c.toasts.last_fg {
            c.toasts.last_fg = fg;
            win32::raise(c.toastwin.hwnd);
        }
        c.toasts.watch_timer = rt::after(400, toast_watch_tick);
    });
    if ran.is_none() {
        rt::after(50, toast_watch_tick); // the app state was busy: look again in a moment
    }
}
