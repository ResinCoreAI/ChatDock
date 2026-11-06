//! Who wrote, for the apps whose pages never say it in a notification (Instagram and Facebook send
//! theirs through Web Push, which WebView2 doesn't have, so ChatDock only ever saw the unread
//! number). When that number goes up, the newest unread conversation is read from the chat list the
//! page shows: its name, its last message and its picture go in the pop-up. A click on the pop-up,
//! or on the app in the edge tab while something is unread, opens that conversation. A page left on
//! a conversation goes back to its list after the chat has been hidden a while, so the next message
//! can be read there (not while something is being typed).
//! The sites' markup has no stable names, so the list is found by its shape: rows that link to a
//! conversation (".../t/…"), or rows with a picture and two lines of text; an unread row's last
//! line is bolder than a read one's. What was read is only put in the pop-up, never in the log (the
//! log says how the page looked: how many rows, how many unread, the lengths).

use std::{
    cell::RefCell,
    collections::HashSet,
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::Value;

use crate::{
    core::{later, timer, Core, PanelState},
    log, rt,
};

/// The wait before pages go back to their lists (schedule_back_to_list).
static BACK_TIMER: AtomicU64 = AtomicU64::new(0);

/// Self-test: pages are going back to their lists in a while.
pub fn back_pending() -> bool {
    BACK_TIMER.load(Ordering::SeqCst) != 0
}

thread_local! {
    /// apps with a reply on its way (send_reply)
    static SENDING: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// Apps whose chat list is read for the pop-ups.
pub fn reads_list(app: &str) -> bool {
    matches!(app, "instagram" | "facebook")
}

/// Apps whose unread number also counts things that aren't chats (Facebook's: likes, comments,
/// friend requests), so each rise is checked against the chat list.
pub fn counts_notifications(app: &str) -> bool {
    app == "facebook"
}

/// The newest unread conversation, as the chat list shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Latest {
    pub name: String,
    pub text: String,
    pub avatar: String,
    /// its link ("/messages/t/…"), when the row is one; else its name finds it again
    pub href: String,
}

/// What the chat list said.
#[derive(Clone, Debug, PartialEq)]
pub enum ListRead {
    /// the newest unread conversation, and how many look unread (0: the list isn't sure enough to
    /// count by, see list_read)
    Unread(Latest, u32),
    /// the list is there and nothing in it looks unread
    NothingUnread,
    /// no list to read: another page, one still loading, or one not understood
    Unknown,
}

impl ListRead {
    pub fn latest(self) -> Option<Latest> {
        match self {
            ListRead::Unread(l, _) => Some(l),
            _ => None,
        }
    }
}

/// How long the chat stays hidden before a page left on a conversation goes back to its list.
const BACK_AFTER_MS: u64 = 20_000;

/// Reads the list (and, with OPEN = true, clicks the newest unread row). The same helpers are used
/// by OPEN_JS and BACK_JS.
const HELPERS: &str = r#"
  const shown = (e) => { const r = e.getBoundingClientRect(); return r.width > 4 && r.height > 4 && r.bottom > 0 && r.top < innerHeight && r.right > 0 && r.left < innerWidth; };
  const texts = (row) => [...row.querySelectorAll('span, div, abbr, h3, h4')]
    .filter((e) => !e.children.length && e.textContent.trim() && shown(e))
    .map((e) => ({ e, t: e.textContent.trim().replace(/\s+/g, ' ') }));
  const picture = (row) => { const i = row.querySelector('img[src^="https:"], image'); return i ? (i.getAttribute('src') || i.getAttribute('href') || i.getAttribute('xlink:href') || '') : ''; };
  const innermost = (list) => list.filter((r) => !list.some((o) => o !== r && r.contains(o)));
  const cut = (s, n) => Array.from(s).slice(0, n).join(''); // whole characters: half an emoji isn't JSON Rust reads
  const isTime = (t) => /^(\d+\s*[a-z฀-๿぀-ヿ一-鿿.]{1,10}|\d{1,2}[:.]\d{2}(\s*[ap]\.?m\.?)?|now|just now|yesterday|ตอนนี้|เมื่อสักครู่|เมื่อวาน)$/i.test(t);
  const rowsOf = () => {
    let rows = [...document.querySelectorAll('a[href*="/t/"]')].filter((r) => shown(r) && texts(r).length >= 2);
    if (rows.length >= 2) return { rows, links: rows.length };
    const all = [...document.querySelectorAll('[role="button"], [role="listitem"], [role="row"], [role="gridcell"], [role="link"]')]
      .filter((r) => shown(r) && r.getBoundingClientRect().height < 140 && picture(r) && texts(r).length >= 2);
    return { rows: innermost(all), links: rows.length };
  };
"#;

const LATEST_JS: &str = r#"((open) => {
%HELPERS%
  const out = { how: 'none', page: 'other', links: 0, rows: 0, weights: 0, unread: 0 };
  const path = location.pathname;
  out.page = /\/t\//.test(path) ? 'conversation' : /^\/(direct\/inbox|messages)\/?$/.test(path) ? 'list' : 'other';
  const { rows, links } = rowsOf();
  out.links = links;
  out.rows = rows.length;
  if (rows.length < 2) return JSON.stringify(out);
  const weight = (e) => parseInt(getComputedStyle(e).fontWeight, 10) || 400;
  const mine = /^(you|คุณ)\s*:/i;
  const info = rows.map((r) => {
    const ts = texts(r);
    let line = ts.slice(1).find((x) => x.t.length > 2 && !/^[·•|]/.test(x.t)) || ts[1];
    // a short message ("ok", "55", one emoji) is passed over above, and what comes next is its time
    if (line !== ts[1] && isTime(line.t) && !/^[·•|]/.test(ts[1].t)) line = ts[1];
    return { r, name: ts[0].t, text: line.t, w: weight(line.e) };
  });
  const base = Math.min(...info.map((i) => i.w));
  out.base = base;
  out.weights = new Set(info.map((i) => i.w)).size;
  const unread = info.filter((i) => i.w > base && i.w >= 600 && !mine.test(i.text));
  out.unread = unread.length;
  if (!unread.length) return JSON.stringify(out);
  const top = unread[0];
  const link = top.r.closest('a[href]') || top.r.querySelector('a[href*="/t/"]');
  Object.assign(out, { how: 'list', name: cut(top.name, 80), text: cut(top.text, 300), avatar: picture(top.r).slice(0, 2000), href: link ? link.getAttribute('href') : '' });
  // (not away from a conversation with a half-written message in it: that stays where it is)
  const typing = [...document.querySelectorAll('[contenteditable="true"], textarea')].some((b) => (b.value || b.textContent || '').trim());
  if (open && !(out.page === 'conversation' && typing)) {
    top.r.click();
    out.opened = true;
  }
  return JSON.stringify(out);
})(%OPEN%)"#;

/// Opens one conversation of the list: by its link, else by its name.
const OPEN_JS: &str = r#"((href, name) => {
%HELPERS%
  let row = href ? [...document.querySelectorAll('a[href]')].find((a) => a.getAttribute('href') === href && shown(a)) : null;
  if (!row && name) {
    const all = [...document.querySelectorAll('a[href*="/t/"], [role="button"], [role="listitem"], [role="row"], [role="gridcell"], [role="link"]')]
      .filter((r) => shown(r) && r.getBoundingClientRect().height < 140 && (texts(r)[0] || {}).t === name);
    row = innermost(all)[0];
  }
  if (row) {
    row.click();
    return 'opened';
  }
  if (href && href.startsWith('/') && href.includes('/t/')) {
    location.assign(href);
    return 'went';
  }
  return 'not found';
})(%HREF%, %NAME%)"#;

/// Back to the chat list (the site's own link to it), unless something is being typed.
const BACK_JS: &str = r#"(() => {
  if (!/\/t\//.test(location.pathname)) return 'on the list';
  if ([...document.querySelectorAll('[contenteditable="true"], textarea')].some((b) => (b.value || b.textContent || '').trim())) return 'typing';
  const home = [...document.querySelectorAll('a[href="/direct/inbox/"], a[href="/direct/inbox"], a[href="/messages/"], a[href="/messages"]')]
    .find((a) => { const r = a.getBoundingClientRect(); return r.width > 0 && r.height > 0; });
  if (!home) return 'no link back';
  home.click();
  return 'back';
})()"#;

/// How a reply went (send_reply).
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Sent {
    /// it shows as the last message of the conversation
    Confirmed,
    /// something is being typed there already: nothing was touched
    Typing,
    /// that conversation didn't open (or another one is on screen): nothing was typed
    NotOpened,
    NoComposer,
    /// typed and sent, but it never showed up: maybe not sent
    NotConfirmed,
    /// another reply to this app is still on its way, or the app can't take replies
    Busy,
}

/// Apps whose pop-ups can be replied to (their conversations are opened from the chat list).
pub fn can_reply(app: &str) -> bool {
    matches!(app, "instagram" | "facebook")
}

/// Waits until the conversation the pop-up is about is the one open (its link, or its name at the
/// top of the page), types the reply into its box the way a paste does (the site's own input
/// handlers run), presses Enter, and waits up to 4 s for it to show as a message.
const SEND_JS: &str = r#"(async (text, href, name) => {
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const thread = (h) => (h.match(/\/t\/[^/?#]+/) || [''])[0];
  // the conversation's own header: never a list row or a link (those name other conversations)
  const named = () => [...document.querySelectorAll('h1, h2, header span, [role="main"] span')]
    .filter((e) => !e.closest('a, [role="link"], [role="row"], [role="listitem"], [role="gridcell"]'))
    .some((e) => { const r = e.getBoundingClientRect(); return r.top >= 0 && r.top < 160 && r.width > 0 && (e.textContent || '').trim() === name; });
  const isOpen = () => /\/t\//.test(location.pathname) && (href ? thread(location.pathname) === thread(href) : !!name && named());
  const boxes = () => [...document.querySelectorAll('[contenteditable="true"][role="textbox"], div[contenteditable="true"], textarea')]
    .filter((b) => { const r = b.getBoundingClientRect(); return r.width > 40 && r.height > 10; });
  const content = (b) => (b.value || b.textContent || '').trim();
  let box = null;
  // by the clock, not by turns: a hidden page gets its timers once a second at most
  let until = Date.now() + 4000;
  while (Date.now() < until && !(isOpen() && (box = boxes().pop()))) await sleep(100);
  if (!isOpen()) return 'not opened';
  // a single-page site changes its address before the conversation on screen: let it settle
  await sleep(600);
  box = boxes().pop() || null;
  if (!isOpen()) return 'not opened';
  if (!box) return 'no composer';
  if (content(box)) return 'typing';
  const want = text.trim().replace(/\s+/g, ' ');
  // how often it is in the conversation already ("OK" may well be): only one more counts as sent
  const count = () => [...document.querySelectorAll('[role="row"], [role="listitem"], div[dir="auto"], span[dir="auto"]')]
    .slice(-60).filter((e) => !box.contains(e) && (e.textContent || '').trim().replace(/\s+/g, ' ') === want).length;
  const before = count();
  box.focus();
  document.execCommand('insertText', false, text);
  const enter = (type) => box.dispatchEvent(new KeyboardEvent(type, { key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true, cancelable: true }));
  enter('keydown'); enter('keypress'); enter('keyup');
  until = Date.now() + 4000;
  while (Date.now() < until) {
    if (count() > before && !content(box)) return 'confirmed';
    await sleep(100);
  }
  // not sent: what ChatDock typed doesn't stay in the box (the user's own text never gets here)
  if (content(box).replace(/\s+/g, ' ') === want) {
    box.focus();
    document.execCommand('selectAll', false);
    document.execCommand('delete', false);
  }
  return 'not confirmed';
})(%TEXT%, %HREF%, %NAME%)"#;

fn send_js_to(text: &str, href: &str, name: &str) -> String {
    let j = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into());
    SEND_JS.replace("%TEXT%", &j(text)).replace("%HREF%", &j(href)).replace("%NAME%", &j(name))
}

#[cfg(test)]
fn send_js(text: &str) -> String {
    send_js_to(text, "/t/1/", "Nok")
}

fn sent_of(result: &str) -> Sent {
    match serde_json::from_str::<String>(result).as_deref() {
        Ok("confirmed") => Sent::Confirmed,
        Ok("typing") => Sent::Typing,
        Ok("not opened") => Sent::NotOpened,
        Ok("no composer") => Sent::NoComposer,
        _ => Sent::NotConfirmed,
    }
}

/// The log line for a reply: how long it was and how it went, never what it said.
fn sent_log(s: &Sent, text: &str) -> String {
    let how = match s {
        Sent::Confirmed => "confirmed",
        Sent::Typing => "refused: something is being typed there",
        Sent::NotOpened => "not sent: the conversation didn't open",
        Sent::NoComposer => "not sent: no message box",
        Sent::NotConfirmed => "not confirmed",
        Sent::Busy => "busy",
    };
    format!("sent: {} chars, {how}", text.chars().count())
}

fn latest_js(open: bool) -> String {
    LATEST_JS.replace("%HELPERS%", HELPERS).replace("%OPEN%", if open { "true" } else { "false" })
}

fn open_js(href: &str, name: &str) -> String {
    let js = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into());
    OPEN_JS.replace("%HELPERS%", HELPERS).replace("%HREF%", &js(href)).replace("%NAME%", &js(name))
}

/// What the page script returned (a JSON string inside the script's JSON result).
fn parse(result: &str) -> Value {
    serde_json::from_str::<String>(result).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null)
}

/// The conversation read, if one was found (never empty text or name).
fn latest_of(v: &Value) -> Option<Latest> {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let (name, text) = (s("name"), s("text"));
    (v.get("how").and_then(Value::as_str) == Some("list") && !name.is_empty() && !text.is_empty()).then(|| Latest {
        name,
        text,
        avatar: s("avatar"),
        href: s("href"),
    })
}

/// The conversation read, or that the list has nothing unread, or that there was no list. Counted
/// ("nothing unread", or how many are) only when the page is sure to be the chat list: on a chats
/// page, three rows or more that link to their conversations, and a normal (not bold) weight to
/// tell unread rows from (with every row bold, they could all be unread).
fn list_read(v: &Value) -> ListRead {
    let n = |k: &str| v.get(k).and_then(Value::as_i64).unwrap_or(0);
    let page = v.get("page").and_then(Value::as_str).unwrap_or("");
    let sure = n("rows") >= 3 && n("links") >= 3 && matches!(page, "list" | "conversation") && (1..600).contains(&n("base"));
    if let Some(l) = latest_of(v) {
        return ListRead::Unread(l, if sure { n("unread").max(0) as u32 } else { 0 });
    }
    if sure && n("unread") == 0 {
        ListRead::NothingUnread
    } else {
        ListRead::Unknown
    }
}

/// How the page looked, for the log: never its text.
fn shape(v: &Value) -> String {
    let n = |k: &str| v.get(k).and_then(Value::as_i64).unwrap_or(0);
    let len = |k: &str| v.get(k).and_then(Value::as_str).map(|s| s.chars().count()).unwrap_or(0);
    format!(
        "page {}, {} rows ({} links), {} boldness levels, {} unread: {} (name {} chars, text {} chars, picture {}, link {})",
        v.get("page").and_then(Value::as_str).unwrap_or("?"),
        n("rows"),
        n("links"),
        n("weights"),
        n("unread"),
        v.get("how").and_then(Value::as_str).unwrap_or("no answer"),
        len("name"),
        len("text"),
        if len("avatar") > 0 { "yes" } else { "no" },
        if len("href") > 0 { "yes" } else { "no" },
    )
}

type Then = Box<dyn FnOnce(&mut Core, Option<Latest>) + Send>;
type ThenRead = Box<dyn FnOnce(&mut Core, ListRead) + Send>;
type ThenSent = Box<dyn FnOnce(&mut Core, Sent) + Send>;

impl Core {
    /// Reads `app`'s chat list, then `then` with what it said.
    pub fn read_list(&mut self, app: &str, then: ThenRead) {
        if !reads_list(app) || !self.chats.has(app) {
            then(self, ListRead::Unknown);
            return;
        }
        let a = app.to_string();
        self.chats.execute(app, &latest_js(false), move |result| {
            let v = parse(&result);
            later(move |c| {
                log!("{a}: who wrote: {}", shape(&v));
                then(c, list_read(&v));
            });
        });
    }

    /// Reads the newest unread conversation from `app`'s chat list, then `then` with it (None: the
    /// page shows no list, or nothing there looks unread).
    pub fn read_latest(&mut self, app: &str, then: Then) {
        self.read_list(app, Box::new(move |c, read| then(c, read.latest())));
    }

    /// The edge tab or a pop-up opened `app` on something new: straight into that conversation (the
    /// newest unread one when there's no target).
    /// The user clicked a quick reply or pressed Enter in the reply box: into the conversation the
    /// pop-up is about, and only that one. One reply per app at a time.
    pub fn send_reply(&mut self, app: &str, target: Latest, text: String, then: ThenSent) {
        if !can_reply(app) || !self.chats.has(app) || SENDING.with(|s| s.borrow().contains(app)) {
            log!("{app}: {}", sent_log(&Sent::Busy, &text));
            then(self, Sent::Busy);
            return;
        }
        self.open_conversation(app, Some(target.clone()));
        self.send_on_page(app, target, text, then);
    }

    /// The part of send_reply that runs in the page (the self-test calls it on a stand-in page).
    pub fn send_on_page(&mut self, app: &str, target: Latest, text: String, then: ThenSent) {
        SENDING.with(|s| s.borrow_mut().insert(app.to_string()));
        let a = app.to_string();
        let js = send_js_to(&text, &target.href, &target.name);
        self.chats.execute_async(app, &js, move |r| {
            let sent = sent_of(&r);
            later(move |c| {
                SENDING.with(|s| s.borrow_mut().remove(&a));
                log!("{a}: {}", sent_log(&sent, &text));
                if c.panel_state == PanelState::Hidden {
                    // the page is on that conversation now: back to its list in a while, as after
                    // the chat was hidden (the next message is read there, and isn't marked seen)
                    c.schedule_back_to_list();
                }
                then(c, sent);
            });
        });
    }

    pub fn open_conversation(&mut self, app: &str, target: Option<Latest>) {
        if !reads_list(app) || !self.chats.has(app) {
            return;
        }
        let a = app.to_string();
        let js = match &target {
            Some(t) => open_js(&t.href, &t.name),
            None => latest_js(true),
        };
        self.chats.execute(app, &js, move |result| {
            let said = match serde_json::from_str::<String>(&result) {
                Ok(s) if s.starts_with('{') => {
                    let v = parse(&result);
                    if v.get("opened").and_then(Value::as_bool) == Some(true) {
                        "opened the newest unread".to_string()
                    } else {
                        format!("nothing to open ({})", shape(&v))
                    }
                }
                Ok(s) => s,
                Err(_) => "no answer".into(),
            };
            later(move |_| log!("{a}: open the conversation: {said}"));
        });
    }

    /// The chat was hidden: a while later, pages left on a conversation go back to their list (so
    /// the next message can be read there).
    pub fn schedule_back_to_list(&mut self) {
        // the chat hidden again: 20 s from now, not from an earlier time it was hidden
        rt::cancel(BACK_TIMER.swap(0, Ordering::SeqCst));
        let t = timer(BACK_AFTER_MS, |c| {
            BACK_TIMER.store(0, Ordering::SeqCst);
            if c.panel_state != PanelState::Hidden {
                return; // out again meanwhile
            }
            let mut playing = false;
            for app in c.enabled_apps() {
                if !reads_list(app) || !c.chats.has(app) || c.chats.page_call_live(app) || c.chats.in_call(app) {
                    continue;
                }
                if c.chats.playing_audio(app) {
                    playing = true; // a voice message or a video still playing: it would stop
                    continue;
                }
                c.back_to_list_now(app);
            }
            if playing {
                c.schedule_back_to_list();
            }
        });
        BACK_TIMER.store(t, Ordering::SeqCst);
    }

    pub fn back_to_list_now(&mut self, app: &str) {
        let a = app.to_string();
        self.chats.execute(app, BACK_JS, move |result| {
            let said = serde_json::from_str::<String>(&result).unwrap_or_else(|_| "no answer".into());
            if said != "on the list" {
                later(move |_| log!("{a}: back to its chat list: {said}"));
            }
        });
    }

    /// The self-test: the pop-up for an unread number, as if its list had said `latest`.
    pub fn count_popup_for_test(&mut self, app: &str, latest: Option<Latest>) {
        self.count_popup(app, latest);
    }

    /// The self-test: the conversation the newest pop-up of `app` opens.
    pub fn test_popup_target(&self, app: &str) -> Option<Latest> {
        self.toasts_target(app)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{latest_js, latest_of, list_read, open_js, send_js, sent_log, sent_of, shape, Latest, ListRead, Sent};

    #[test]
    fn a_reply_is_passed_as_json_and_never_logged() {
        let js = send_js("a \"quoted\" ข้อความ\nline 2");
        assert!(js.contains(r#""a \"quoted\" ข้อความ\nline 2""#));
        assert!(!js.contains("%TEXT%"));
        assert_eq!(sent_of("\"confirmed\""), Sent::Confirmed);
        assert_eq!(sent_of("\"typing\""), Sent::Typing);
        assert_eq!(sent_of("\"no composer\""), Sent::NoComposer);
        assert_eq!(sent_of("garbage"), Sent::NotConfirmed);
        let line = sent_log(&Sent::Confirmed, "secret words");
        assert_eq!(line, "sent: 12 chars, confirmed");
        assert!(!line.contains("secret"));
    }

    #[test]
    fn a_list_with_nothing_unread_is_told_from_no_list() {
        // Facebook on 10-05: a conversation open, its list beside it, every row read
        let read = json!({ "how": "none", "page": "conversation", "rows": 11, "links": 11, "weights": 1, "base": 400, "unread": 0 });
        assert_eq!(list_read(&read), ListRead::NothingUnread);
        // every row bold: they could all be unread
        let bold = json!({ "how": "none", "page": "list", "rows": 5, "links": 5, "base": 700, "unread": 0 });
        assert_eq!(list_read(&bold), ListRead::Unknown);
        // rows without links (found by their shape), or another page: not sure it's the chat list
        let shapes = json!({ "how": "none", "page": "list", "rows": 9, "links": 0, "base": 400, "unread": 0 });
        assert_eq!(list_read(&shapes), ListRead::Unknown);
        let other = json!({ "how": "none", "page": "other", "rows": 9, "links": 9, "base": 400, "unread": 0 });
        assert_eq!(list_read(&other), ListRead::Unknown);
        // too few rows, or nothing there at all
        let few = json!({ "how": "none", "page": "list", "rows": 2, "links": 2, "base": 400, "unread": 0 });
        assert_eq!(list_read(&few), ListRead::Unknown);
        assert_eq!(list_read(&json!({ "how": "none", "rows": 0 })), ListRead::Unknown);
        assert_eq!(list_read(&serde_json::Value::Null), ListRead::Unknown);
        // something unread that couldn't be read whole isn't "nothing unread"
        let half = json!({ "how": "none", "page": "list", "rows": 9, "links": 9, "base": 400, "unread": 1 });
        assert_eq!(list_read(&half), ListRead::Unknown);
        // one unread: counted on a list it is sure of (Facebook 10-05 15:39: its number 3, one chat)
        let one = json!({ "how": "list", "page": "conversation", "rows": 11, "links": 11, "base": 400, "unread": 1, "name": "Nok", "text": "hi" });
        assert!(matches!(list_read(&one), ListRead::Unread(_, 1)));
        assert!(matches!(
            list_read(&json!({ "how": "list", "rows": 9, "unread": 1, "name": "Nok", "text": "hi" })),
            ListRead::Unread(_, 0)
        ));
    }

    #[test]
    fn what_the_page_said_becomes_the_pop_up() {
        let v = json!({ "how": "list", "page": "list", "rows": 9, "links": 9, "weights": 2, "unread": 1, "name": " Somchai ", "text": "ไปเล่นกันป่าว", "avatar": "https://x/y.jpg", "href": "/messages/t/1/" });
        assert_eq!(
            latest_of(&v),
            Some(Latest {
                name: "Somchai".into(),
                text: "ไปเล่นกันป่าว".into(),
                avatar: "https://x/y.jpg".into(),
                href: "/messages/t/1/".into()
            })
        );
        assert_eq!(latest_of(&json!({ "how": "none", "rows": 0 })), None);
        assert_eq!(latest_of(&json!({ "how": "list", "name": "", "text": "x" })), None);
    }

    #[test]
    fn the_log_never_holds_the_messages() {
        let v = json!({ "how": "list", "page": "list", "rows": 9, "links": 0, "weights": 2, "unread": 1, "name": "Somchai", "text": "secret words", "avatar": "https://x", "href": "" });
        let line = shape(&v);
        assert!(!line.contains("Somchai") && !line.contains("secret"));
        assert!(line.contains("9 rows") && line.contains("name 7 chars") && line.contains("picture yes") && line.contains("link no"));
    }

    #[test]
    fn the_scripts_are_whole() {
        for js in [latest_js(false), latest_js(true), open_js("/messages/t/1/", "Na\"me")] {
            assert!(!js.contains('%'), "a placeholder left");
        }
        assert!(open_js("/t/1", "a\"b").contains(r#"("/t/1", "a\"b")"#)); // names are passed as JSON strings
    }
}
