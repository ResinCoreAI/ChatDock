//! The Browser's own tabs (under ChatDock's app tabs, never mixed with them): which pages are
//! open, which one is on screen, and which have gone unused long enough to give their memory back.
//! Pure: browser.rs keeps the WebView2 views that go with them.

use serde_json::{json, Value};

use crate::search::{web_url, URL_MAX};

pub const MAX_TABS: usize = 10;
/// A tab out of sight this long drops its page (kept: address and title; a click reloads it).
pub const IDLE_DROP_MS: i64 = 30 * 60 * 1000;
const TITLE_MAX: usize = 300;

#[derive(Clone, Debug, PartialEq)]
pub struct Tab {
    pub id: u32,
    /// "" = our start page
    pub url: String,
    pub title: String,
    pub last_used: i64,
    /// the tab whose page opened this one as a window (a sign-in pop-up goes back to it on closing)
    pub opener: Option<u32>,
}

impl Tab {
    pub fn is_start(&self) -> bool {
        self.url.is_empty()
    }
}

#[derive(Debug)]
pub struct Tabs {
    list: Vec<Tab>,
    open: usize,
    next: u32,
}

impl Default for Tabs {
    fn default() -> Self {
        Tabs::restore(&Value::Null, 0, 0)
    }
}

fn clip(s: &str) -> String {
    s.chars().take(TITLE_MAX).collect()
}

fn url_ok(url: &str) -> bool {
    url.is_empty() || (web_url(url) && url.len() <= URL_MAX)
}

impl Tabs {
    /// From settings.json's `browser.tabs` (each `{url, title}`; a broken one becomes a start page).
    /// Never empty.
    pub fn restore(saved: &Value, open: usize, now: i64) -> Tabs {
        let mut t = Tabs { list: Vec::new(), open: 0, next: 1 };
        for s in saved.as_array().into_iter().flatten().take(MAX_TABS) {
            let url = s.get("url").and_then(Value::as_str).filter(|u| url_ok(u)).unwrap_or("");
            let title = s.get("title").and_then(Value::as_str).unwrap_or("");
            t.push(url, title, now);
        }
        if t.list.is_empty() {
            t.push("", "", now);
        }
        t.open = open.min(t.list.len() - 1);
        t
    }

    fn push(&mut self, url: &str, title: &str, now: i64) -> u32 {
        let id = self.next;
        self.next += 1;
        self.list.push(Tab { id, url: url.to_string(), title: clip(title), last_used: now, opener: None });
        id
    }
    pub fn list(&self) -> &[Tab] {
        &self.list
    }
    pub fn open_index(&self) -> usize {
        self.open
    }
    pub fn current(&self) -> &Tab {
        &self.list[self.open]
    }
    pub fn get(&self, id: u32) -> Option<&Tab> {
        self.list.iter().find(|t| t.id == id)
    }
    pub fn full(&self) -> bool {
        self.list.len() >= MAX_TABS
    }
    /// A new tab right after the open one, and it becomes the open one. None at MAX_TABS.
    pub fn add(&mut self, url: &str, now: i64) -> Option<u32> {
        if self.full() {
            return None;
        }
        let id = self.next;
        self.next += 1;
        self.list[self.open].last_used = now; // the one left was in use until now
        let at = self.open + 1;
        let url = if url_ok(url) { url } else { "" };
        self.list.insert(at, Tab { id, url: url.to_string(), title: String::new(), last_used: now, opener: None });
        self.open = at;
        Some(id)
    }

    /// A page's new window as a tab next to the open one; closing it goes back to `opener`.
    pub fn add_window(&mut self, url: &str, opener: u32, now: i64) -> Option<u32> {
        let id = self.add(url, now)?;
        self.list[self.open].opener = Some(opener);
        Some(id)
    }
    pub fn select(&mut self, id: u32, now: i64) -> bool {
        let Some(i) = self.list.iter().position(|t| t.id == id) else { return false };
        self.list[self.open].last_used = now; // the one left was in use until now
        self.open = i;
        self.list[i].last_used = now;
        true
    }
    /// The open tab closed: its right neighbour opens (the left one at the end). The last tab
    /// closed: a fresh start page takes its place.
    pub fn close(&mut self, id: u32, now: i64) -> bool {
        let Some(i) = self.list.iter().position(|t| t.id == id) else { return false };
        let was_open = i == self.open;
        let gone = self.list.remove(i);
        if self.list.is_empty() {
            self.push("", "", now);
            self.open = 0;
            return true;
        }
        if i < self.open {
            self.open -= 1;
        } else if self.open >= self.list.len() {
            self.open = self.list.len() - 1;
        }
        // a window that closes goes back to the page that opened it
        if let Some(at) = gone.opener.filter(|_| was_open).and_then(|o| self.list.iter().position(|t| t.id == o)) {
            self.open = at;
        }
        self.list[self.open].last_used = now;
        true
    }
    /// The page went somewhere (a bad address is ignored; "" = back to the start page).
    pub fn set_page(&mut self, id: u32, url: &str, title: &str) {
        if let Some(t) = self.list.iter_mut().find(|t| t.id == id) {
            if url_ok(url) {
                t.url = url.to_string();
            }
            t.title = clip(title);
        }
    }
    pub fn set_title(&mut self, id: u32, title: &str) {
        if let Some(t) = self.list.iter_mut().find(|t| t.id == id) {
            t.title = clip(title);
        }
    }
    /// Ctrl+PgDn (1) / Ctrl+PgUp (-1), round the ends.
    pub fn step(&mut self, delta: i32, now: i64) {
        let n = self.list.len() as i32;
        self.list[self.open].last_used = now; // the one left was in use until now
        self.open = (self.open as i32 + delta).rem_euclid(n) as usize;
        self.list[self.open].last_used = now;
    }
    /// Tabs unused for IDLE_DROP_MS, never the open one or one `keep` holds on to (playing sound).
    pub fn idle(&self, now: i64, keep: impl Fn(u32) -> bool) -> Vec<u32> {
        self.list
            .iter()
            .enumerate()
            .filter(|(i, t)| *i != self.open && now - t.last_used >= IDLE_DROP_MS && !keep(t.id))
            .map(|(_, t)| t.id)
            .collect()
    }
    pub fn saved(&self) -> Value {
        Value::Array(self.list.iter().map(|t| json!({ "url": t.url, "title": t.title })).collect())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn urls(t: &Tabs) -> Vec<&str> {
        t.list().iter().map(|t| t.url.as_str()).collect()
    }

    #[test]
    fn a_new_install_has_one_start_page() {
        let t = Tabs::default();
        assert_eq!(t.list().len(), 1);
        assert!(t.current().is_start());
    }

    #[test]
    fn restored_tabs_keep_their_order_and_drop_bad_addresses() {
        let saved = json!([
            { "url": "https://a.com/", "title": "A" },
            { "url": "file:///C:/secret", "title": "x" },
            { "url": "https://c.com/", "title": "C".repeat(500) },
            "junk"
        ]);
        let t = Tabs::restore(&saved, 2, 5);
        assert_eq!(urls(&t), ["https://a.com/", "", "https://c.com/", ""]);
        assert_eq!(t.open_index(), 2);
        assert_eq!(t.current().title.chars().count(), 300);
        assert_eq!(Tabs::restore(&saved, 99, 0).open_index(), 3);
        let many: Vec<_> = (0..15).map(|i| json!({ "url": format!("https://s{i}.com/"), "title": "" })).collect();
        assert_eq!(Tabs::restore(&json!(many), 0, 0).list().len(), MAX_TABS);
    }

    #[test]
    fn a_new_tab_goes_right_after_the_open_one() {
        let mut t = Tabs::restore(&json!([{ "url": "https://a.com/" }, { "url": "https://b.com/" }]), 0, 0);
        let id = t.add("https://new.com/", 1).unwrap();
        assert_eq!(urls(&t), ["https://a.com/", "https://new.com/", "https://b.com/"]);
        assert_eq!(t.current().id, id);
    }

    #[test]
    fn no_eleventh_tab() {
        let mut t = Tabs::default();
        for _ in 1..MAX_TABS {
            assert!(t.add("", 0).is_some());
        }
        assert!(t.full());
        assert_eq!(t.add("https://x.com/", 0), None);
        assert_eq!(t.list().len(), MAX_TABS);
    }

    #[test]
    fn closing_the_open_tab_opens_its_right_neighbour() {
        let saved = json!([{ "url": "https://a.com/" }, { "url": "https://b.com/" }, { "url": "https://c.com/" }]);
        let mut t = Tabs::restore(&saved, 1, 0);
        let b = t.current().id;
        assert!(t.close(b, 1));
        assert_eq!(t.current().url, "https://c.com/");
        let c = t.current().id;
        t.close(c, 2); // the last one in the strip: the left one opens
        assert_eq!(t.current().url, "https://a.com/");
        let a = t.current().id;
        t.close(a, 3); // the very last tab: a fresh start page
        assert_eq!(t.list().len(), 1);
        assert!(t.current().is_start());
        assert!(!t.close(9999, 4));
    }

    #[test]
    fn closing_a_tab_left_of_the_open_one_keeps_the_open_one() {
        let saved = json!([{ "url": "https://a.com/" }, { "url": "https://b.com/" }, { "url": "https://c.com/" }]);
        let mut t = Tabs::restore(&saved, 2, 0);
        let a = t.list()[0].id;
        t.close(a, 1);
        assert_eq!(t.current().url, "https://c.com/");
    }

    #[test]
    fn pages_titles_and_stepping() {
        let mut t = Tabs::restore(&json!([{ "url": "" }, { "url": "" }, { "url": "" }]), 0, 0);
        let id = t.current().id;
        t.set_page(id, "https://a.com/", "A");
        t.set_page(id, "javascript:alert(1)", "B"); // ignored address, title kept from the page
        assert_eq!((t.current().url.as_str(), t.current().title.as_str()), ("https://a.com/", "B"));
        t.set_page(id, "", "");
        assert!(t.current().is_start());
        t.step(-1, 0);
        assert_eq!(t.open_index(), 2);
        t.step(1, 0);
        assert_eq!(t.open_index(), 0);
        let second = t.list()[1].id;
        assert!(t.select(second, 0));
        assert_eq!(t.open_index(), 1);
        assert!(!t.select(4242, 0));
    }

    #[test]
    fn idle_tabs_but_never_the_open_or_a_kept_one() {
        let saved = json!([{ "url": "https://a.com/" }, { "url": "https://b.com/" }, { "url": "https://c.com/" }]);
        let mut t = Tabs::restore(&saved, 0, 0);
        let (a, b, c) = (t.list()[0].id, t.list()[1].id, t.list()[2].id);
        t.select(a, IDLE_DROP_MS); // a: used just now
        assert_eq!(t.idle(IDLE_DROP_MS, |_| false), [b, c]);
        assert_eq!(t.idle(IDLE_DROP_MS, |id| id == c), [b]); // c plays sound
        assert!(t.idle(IDLE_DROP_MS - 1, |_| false).iter().all(|id| *id != a));
    }

    #[test]
    fn saved_is_what_restore_reads() {
        let saved = json!([{ "url": "https://a.com/", "title": "A" }, { "url": "", "title": "" }]);
        let t = Tabs::restore(&saved, 1, 0);
        assert_eq!(t.saved(), saved);
    }

    #[test]
    fn a_tab_left_counts_as_used_until_it_was_left() {
        let saved = json!([{ "url": "https://a.com/" }, { "url": "https://b.com/" }]);
        let mut t = Tabs::restore(&saved, 0, 0);
        let (a, b) = (t.list()[0].id, t.list()[1].id);
        // read a for 35 minutes, then a quick look at b: a was in use until just now
        t.select(b, 35 * 60 * 1000);
        assert!(t.idle(35 * 60 * 1000 + 60_000, |_| false).is_empty(), "a was left a minute ago");
        t.add("", 36 * 60 * 1000);
        assert!(!t.idle(36 * 60 * 1000 + 1, |_| false).contains(&b));
        let _ = a;
    }

    #[test]
    fn a_window_closing_goes_back_to_the_page_that_opened_it() {
        let saved = json!([{ "url": "https://a.com/" }, { "url": "https://b.com/" }, { "url": "https://c.com/" }]);
        let mut t = Tabs::restore(&saved, 0, 0);
        let a = t.list()[0].id;
        let popup = t.add_window("", a, 1).unwrap();
        assert_eq!(t.list()[1].id, popup);
        t.close(popup, 2);
        assert_eq!(t.current().id, a, "back to its opener, not the tab on its right");
        // the opener already gone: the usual rule
        let p2 = t.add_window("", a, 3).unwrap();
        t.close(a, 4);
        t.select(p2, 5);
        t.close(p2, 6);
        assert_eq!(t.list().len(), 2);
    }
}
