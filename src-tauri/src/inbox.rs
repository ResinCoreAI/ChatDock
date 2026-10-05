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

use serde_json::Value;

use crate::{
    core::{later, timer, Core, PanelState},
    log,
};

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
    /// the newest unread conversation
    Unread(Latest),
    /// the list is there (three rows or more) and nothing in it looks unread
    NothingUnread,
    /// no list to read: another page, one still loading, or one not understood
    Unknown,
}

impl ListRead {
    pub fn latest(self) -> Option<Latest> {
        match self {
            ListRead::Unread(l) => Some(l),
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
    const line = ts.slice(1).find((x) => x.t.length > 2 && !/^[·•|]/.test(x.t)) || ts[1];
    return { r, name: ts[0].t, text: line.t, w: weight(line.e) };
  });
  const base = Math.min(...info.map((i) => i.w));
  out.weights = new Set(info.map((i) => i.w)).size;
  const unread = info.filter((i) => i.w > base && i.w >= 600 && !mine.test(i.text));
  out.unread = unread.length;
  if (!unread.length) return JSON.stringify(out);
  const top = unread[0];
  const link = top.r.closest('a[href]') || top.r.querySelector('a[href*="/t/"]');
  Object.assign(out, { how: 'list', name: top.name.slice(0, 80), text: top.text.slice(0, 300), avatar: picture(top.r).slice(0, 2000), href: link ? link.getAttribute('href') : '' });
  if (open) {
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

/// The conversation read, or that the list has nothing unread, or that there was no list.
fn list_read(v: &Value) -> ListRead {
    if let Some(l) = latest_of(v) {
        return ListRead::Unread(l);
    }
    let n = |k: &str| v.get(k).and_then(Value::as_i64).unwrap_or(0);
    if n("rows") >= 3 && n("unread") == 0 {
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
        timer(BACK_AFTER_MS, |c| {
            if c.panel_state != PanelState::Hidden {
                return; // out again meanwhile
            }
            for app in c.enabled_apps() {
                if reads_list(app) && c.chats.has(app) && !c.chats.page_call_live(app) {
                    c.back_to_list_now(app);
                }
            }
        });
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

    use super::{latest_js, latest_of, list_read, open_js, shape, Latest, ListRead};

    #[test]
    fn a_list_with_nothing_unread_is_told_from_no_list() {
        assert_eq!(
            list_read(&json!({ "how": "none", "page": "conversation", "rows": 11, "links": 11, "weights": 1, "unread": 0 })),
            ListRead::NothingUnread
        );
        // too few rows to be sure it's the list, or nothing there at all
        assert_eq!(list_read(&json!({ "how": "none", "rows": 2, "unread": 0 })), ListRead::Unknown);
        assert_eq!(list_read(&json!({ "how": "none", "rows": 0 })), ListRead::Unknown);
        assert_eq!(list_read(&serde_json::Value::Null), ListRead::Unknown);
        // something unread that couldn't be read whole isn't "nothing unread"
        assert_eq!(list_read(&json!({ "how": "none", "rows": 9, "unread": 1 })), ListRead::Unknown);
        assert!(matches!(list_read(&json!({ "how": "list", "rows": 9, "unread": 1, "name": "Nok", "text": "hi" })), ListRead::Unread(_)));
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
