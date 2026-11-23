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
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
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

/// How a message is told in the conversation: the same words, with emoji and spaces left out
/// (Facebook draws emoji as pictures, and a line break in a bubble is no character at all); a reply
/// of emoji only is compared with the pictures' own text.
const COUNT_HELPER: &str = r#"
  const flat = (s) => (s || '').replace(/[\p{Extended_Pictographic}\u{FE0F}\u{200D}\u{1F3FB}-\u{1F3FF}]/gu, '').replace(/\s+/g, '');
  const alts = (e) => [...e.querySelectorAll('img[alt]')].map((i) => i.getAttribute('alt')).join('');
  const sameText = (t, e, want) => { const fw = flat(want); return fw ? flat(t) === fw : (t + (e ? alts(e) : '')).replace(/\s+/g, '') === want.replace(/\s+/g, ''); };
  const countIn = (box, want) => [...(document.querySelector('[role="main"]') || document).querySelectorAll('[role="row"], [role="listitem"], div[dir="auto"], span[dir="auto"]')]
    .slice(-60).filter((e) => !box.contains(e) && sameText(e.textContent || '', e, want)).length;
  // the buttons around the box (a send button is among them only while something is in it)
  const buttonsNear = (box) => {
    const out = new Set();
    let scope = box;
    for (let i = 0; i < 8 && scope.parentElement; i++) {
      scope = scope.parentElement;
      scope.querySelectorAll('[role="button"][aria-label], button[aria-label], button[type="submit"]').forEach((b) => { if (!box.contains(b)) out.add(b); });
    }
    return [...out];
  };
  const labelOf = (b) => ((b.getAttribute('aria-label') || '') + ' ' + (b.textContent || '')).trim();
"#;

/// Step 1 of a reply (send_on_page): waits until the conversation the pop-up is about is the one
/// open (its link, or its name at the top of the page: by name when there is no link that names a
/// conversation, and only while no other chat in the list has that name), finds its message box,
/// leaves it alone if something is being typed there, else marks it (with this reply's own token)
/// and puts the caret in it. What the buttons beside the empty box say is noted: none of those is
/// the send button. The text then goes in as typed keys (Input.insertText, see send_on_page).
const PREP_JS: &str = r#"(async (href, name, deadline, token, text) => {
%COUNT%
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const thread = (h) => (h.match(/\/t\/[^/?#]+/) || [''])[0];
  const want = thread(href || '');
  // the conversation's own header: never a list row or a link (those name other conversations)
  const named = () => [...document.querySelectorAll('h1, h2, header span, [role="main"] span')]
    .filter((e) => !e.closest('a, [role="link"], [role="row"], [role="listitem"], [role="gridcell"]'))
    .some((e) => { const r = e.getBoundingClientRect(); return r.top >= 0 && r.top < 160 && r.width > 0 && (e.textContent || '').trim() === name; });
  const isOpen = () => /\/t\//.test(location.pathname) && (want ? thread(location.pathname) === want : !!name && named());
  // no link (Instagram's list has none): by name only while one chat in the list has that name
  const rowSel = 'a[href*="/t/"], [role="link"], [role="row"], [role="listitem"], [role="button"]';
  const sameName = () => [...document.querySelectorAll(rowSel)]
    .filter((r) => !(r.parentElement && r.parentElement.closest(rowSel)))
    .filter((r) => [...r.querySelectorAll('span, div')].some((x) => !x.childElementCount && (x.textContent || '').trim() === name)).length;
  if (!want && sameName() > 1) return 'not opened';
  const boxes = () => [...document.querySelectorAll('[contenteditable="true"][role="textbox"], div[contenteditable="true"], textarea')]
    .filter((b) => { const r = b.getBoundingClientRect(); return r.width > 40 && r.height > 10; });
  const content = (b) => (b.value || b.textContent || '').trim();
  let box = null;
  // by the clock, not by turns: a hidden page gets its timers once a second at most
  const until = Date.now() + 4000;
  while (Date.now() < until && !(isOpen() && (box = boxes().pop()))) await sleep(100);
  if (!isOpen()) return 'not opened';
  // a single-page site changes its address before the conversation on screen: let it settle
  await sleep(600);
  box = boxes().pop() || null;
  if (!isOpen()) return 'not opened';
  if (!box) return 'no composer';
  if (content(box) || box.querySelector('img')) return 'typing';
  // ChatDock stopped waiting meanwhile (a page that slept long): nothing is typed
  if (Date.now() > deadline) return 'late';
  document.querySelectorAll('[data-chatdock-reply]').forEach((b) => b.removeAttribute('data-chatdock-reply'));
  box.setAttribute('data-chatdock-reply', token);
  // what the buttons beside the empty box say: none of those is the send button (a like button
  // that turns into the send button once something is typed says something else then)
  const old = buttonsNear(box).map(labelOf);
  // how often it is in the conversation already ("OK" may well be): only one more counts as sent
  // counted in the conversation only (the chat list shows the last message too)
  const before = countIn(box, text);
  window.__chatdockReply = { token, before, want: text, path: location.pathname, old };
  box.focus();
  const sel = getSelection();
  if (sel && !box.matches('textarea')) { sel.selectAllChildren(box); sel.collapseToEnd(); }
  return 'ready' + (box.closest('[data-lexical-editor]') ? ' lexical' : '');
})(%HREF%, %NAME%, %DEADLINE%, %TOKEN%, %TEXT%)"#;

/// What the later steps share: this reply's marked box (none once the page shows another
/// conversation), the text it should hold, how often that text is in the conversation, taking our
/// own text out of the box again, and the end of the reply (its marks go).
const STEP_HELPERS: &str = r#"
%COUNT%
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const mine = window.__chatdockReply;
  const st = mine && mine.token === token ? mine : { before: 0, want: '', path: '' };
  const found = document.querySelector('[data-chatdock-reply="' + CSS.escape(token) + '"]');
  const box = found && found.isConnected && st.path === location.pathname ? found : null;
  const content = (b) => (b.value || b.textContent || '').trim();
  const holds = () => !!box && sameText(content(box), box, st.want);
  const count = () => countIn(box, st.want);
  // the box was empty when this reply began (step 1): what is in it now is ours
  const erase = () => {
    if (!box || !(content(box) || box.querySelector('img'))) return;
    box.focus();
    if (box.matches('textarea')) { box.value = ''; box.dispatchEvent(new Event('input', { bubbles: true })); }
    else { document.execCommand('selectAll', false); document.execCommand('delete', false); }
  };
  const done = () => {
    if (found) found.removeAttribute('data-chatdock-reply');
    if (window.__chatdockReply && window.__chatdockReply.token === token) delete window.__chatdockReply;
  };
"#;

/// Step 2: the typed keys are in the box ("keys"; the box keeps the caret for Enter); else the text
/// goes in the way a paste does ("script"). Anything else: what we typed is taken out ("lost").
const TYPED_JS: &str = r#"(async (token, text) => {
%STEP%
  if (!box) { done(); return 'gone'; }
  for (let i = 0; i < 10 && !holds(); i++) await sleep(100);
  if (holds()) {
    if (!box.contains(document.activeElement)) box.focus();
    return 'keys';
  }
  if (content(box) || box.querySelector('img')) { erase(); done(); return 'lost'; }
  box.focus();
  document.execCommand('insertText', false, text);
  await sleep(150);
  if (holds()) return 'script';
  erase();
  done();
  return 'lost';
})(%TOKEN%, %TEXT%)"#;

/// The reply was given up on after its text went in: the text comes out again.
const ERASE_JS: &str = r#"((token) => {
%STEP%
  erase();
  done();
  return 'erased';
})(%TOKEN%)"#;

/// Step 3, after Enter was pressed as a key: sent when the text shows once more in the conversation
/// and the box is empty. If the text is still in the box (and ChatDock is still waiting, `stop`):
/// Enter as a page event, then the conversation's send button (one that didn't say the same while
/// the box was empty, and never a like, sticker, voice clip, attachment or forward). Not sent: what
/// ChatDock typed doesn't stay in the box.
const CONFIRM_JS: &str = r#"(async (token, stop) => {
%STEP%
  if (!box) { done(); return JSON.stringify({ r: 'not confirmed', how: 'box gone', left: '' }); }
  const sent = () => count() > st.before && !content(box);
  const wait = async (ms) => { const until = Date.now() + ms; while (Date.now() < until) { if (sent()) return true; await sleep(100); } return sent(); };
  const shown = (e) => { const r = e.getBoundingClientRect(); return r.width > 4 && r.height > 4; };
  const sendButton = () => {
    const yes = /send|ส่ง|enviar|senden|envoyer|invia|送信|发送|發送|전송|gửi|kirim|отправ/i;
    const no = /like|ถูกใจ|me gusta|gefällt|j'aime|mi piace|いいね|赞|讚|좋아요|thích|suka|нрав|sticker|สติกเกอร์|gif|emoji|อีโมจิ|voice|เสียง|audio|voz|sprach|vocal|clip|attach|แนบ|file|ไฟล์|datei|archivo|photo|รูป|image|bild|foto|camera|กล้อง|forward|ส่งต่อ|weiterleiten|reenviar|transférer|inoltra|転送|转发/i;
    return buttonsNear(box).find((b) => {
      const label = labelOf(b);
      return !(st.old || []).includes(label) && shown(b) && yes.test(label) && !no.test(label);
    }) || null;
  };
  let how = 'key';
  if (!(await wait(2500)) && holds() && Date.now() < stop) {
    how = 'event';
    box.focus();
    const enter = (type) => box.dispatchEvent(new KeyboardEvent(type, { key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true, cancelable: true }));
    enter('keydown'); enter('keypress'); enter('keyup');
    if (!(await wait(1500)) && holds() && Date.now() < stop) {
      const b = sendButton();
      how = b ? 'button' : 'no button';
      if (b) { b.click(); await wait(3000); }
    }
  }
  const ok = sent();
  if (!ok) erase();
  const left = content(box) ? 'text left' : 'box empty';
  done();
  return JSON.stringify({ r: ok ? 'confirmed' : 'not confirmed', how, left });
})(%TOKEN%, %STOP%)"#;

fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Fills a script's %PLACEHOLDERS% in one pass: what goes in is never searched again (a contact's
/// name that says "%TEXT%" stays a name).
fn fill(script: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(script.len());
    let mut rest = script;
    'scan: while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        for (key, value) in values {
            if let Some(after) = rest.strip_prefix(key) {
                out.push_str(value);
                rest = after;
                continue 'scan;
            }
        }
        out.push('%');
        rest = &rest[1..];
    }
    out.push_str(rest);
    out
}

/// `deadline`: the page's clock time (ms) after which nothing is typed any more.
fn prep_js(text: &str, href: &str, name: &str, deadline: u64, token: &str) -> String {
    let script = PREP_JS.replace("%COUNT%", COUNT_HELPER);
    let deadline = deadline.to_string();
    fill(
        &script,
        &[
            ("%HREF%", &js_str(href)),
            ("%NAME%", &js_str(name)),
            ("%DEADLINE%", &deadline),
            ("%TOKEN%", &js_str(token)),
            ("%TEXT%", &js_str(text)),
        ],
    )
}

fn step_script(script: &str) -> String {
    script.replace("%STEP%", STEP_HELPERS).replace("%COUNT%", COUNT_HELPER)
}

fn typed_js(text: &str, token: &str) -> String {
    fill(&step_script(TYPED_JS), &[("%TOKEN%", &js_str(token)), ("%TEXT%", &js_str(text))])
}

fn erase_js(token: &str) -> String {
    fill(&step_script(ERASE_JS), &[("%TOKEN%", &js_str(token))])
}

/// `stop`: the page's clock time (ms) after which no other way of sending is tried.
fn confirm_js(token: &str, stop: u64) -> String {
    fill(&step_script(CONFIRM_JS), &[("%TOKEN%", &js_str(token)), ("%STOP%", &stop.to_string())])
}

/// Enter pressed as a key (DevTools Input.dispatchKeyEvent): a real key event, which sites that
/// ignore page-made events (Facebook's message box) take.
const ENTER_DOWN: &str = r#"{"type":"keyDown","key":"Enter","code":"Enter","windowsVirtualKeyCode":13,"nativeVirtualKeyCode":13,"text":"\r","unmodifiedText":"\r"}"#;
const ENTER_UP: &str = r#"{"type":"keyUp","key":"Enter","code":"Enter","windowsVirtualKeyCode":13,"nativeVirtualKeyCode":13}"#;

/// Step 1's answer: ready (and whether the box is a Lexical editor), or why not.
fn prep_of(result: &str) -> Result<bool, Sent> {
    match serde_json::from_str::<String>(result).as_deref() {
        Ok("ready") => Ok(false),
        Ok("ready lexical") => Ok(true),
        Ok("typing") => Err(Sent::Typing),
        Ok("not opened") => Err(Sent::NotOpened),
        Ok("no composer") => Err(Sent::NoComposer),
        _ => Err(Sent::NotConfirmed),
    }
}

/// Step 3's answer: sent or not, and how (for the log: never the text).
fn confirm_of(result: &str) -> (Sent, String) {
    let v = serde_json::from_str::<String>(result).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).unwrap_or(Value::Null);
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let sent = if s("r") == "confirmed" { Sent::Confirmed } else { Sent::NotConfirmed };
    let how = if v.is_null() { "no answer".to_string() } else { format!("{}, {}", s("how"), s("left")) };
    (sent, how)
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

/// A reply on its way (send_on_page): reported once, by its last step or by the time running out.
#[derive(Clone)]
struct Reply {
    app: String,
    text: String,
    /// marks this reply's box in the page (a late step of an earlier reply never touches it)
    token: String,
    /// when it began (the PC's clock, ms)
    start: u64,
    once: Arc<Mutex<Option<ThenSent>>>,
}

/// A reply's times, from its start: nothing typed after, nothing more tried after, given up at.
const REPLY_TYPE_BY_MS: u64 = 10_000;
const REPLY_TRY_BY_MS: u64 = 23_000;
const REPLY_GIVE_UP_MS: u64 = 30_000;
static REPLY_SEQ: AtomicU64 = AtomicU64::new(0);

impl Reply {
    fn finished(&self) -> bool {
        self.once.lock().map(|t| t.is_none()).unwrap_or(true)
    }

    fn done(&self, c: &mut Core, sent: Sent, how: &str) {
        let Some(then) = self.once.lock().ok().and_then(|mut t| t.take()) else { return };
        SENDING.with(|s| s.borrow_mut().remove(&self.app));
        c.chats.cdp(&self.app, "Emulation.setFocusEmulationEnabled", r#"{"enabled":false}"#, |_| {});
        let how = if how.is_empty() { String::new() } else { format!(" ({how})") };
        log!("{}: {}{how}", self.app, sent_log(&sent, &self.text));
        if c.panel_state == PanelState::Hidden {
            // the page is on that conversation now: back to its list in a while, as after the chat
            // was hidden (the next message is read there, and isn't marked seen)
            c.schedule_back_to_list();
        }
        then(c, sent);
    }
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

    /// The part of send_reply that runs in the page (the self-test calls it on a stand-in page), in
    /// steps: the conversation and its box made ready (PREP_JS), the text typed in as keys, checked
    /// (TYPED_JS), Enter pressed as a key, and the check that it went (CONFIRM_JS), which also tries
    /// Enter as a page event and the send button when the key didn't send it.
    pub fn send_on_page(&mut self, app: &str, target: Latest, text: String, then: ThenSent) {
        SENDING.with(|s| s.borrow_mut().insert(app.to_string()));
        let start = rt::epoch_ms().max(0) as u64;
        let token = format!("r{}", REPLY_SEQ.fetch_add(1, Ordering::Relaxed) + 1);
        let reply = Reply { app: app.to_string(), text, token, start, once: Arc::new(Mutex::new(Some(then))) };
        // the page's answer, or nothing within 30 s (the page crashed or went elsewhere meanwhile):
        // whichever comes first reports, once, and replies to this app work again either way. The
        // page itself stops in time: nothing is typed after 10 s, nothing more is tried after 23 s
        // (the steps after typing take 8 s at most), and both clocks are the PC's.
        let late = reply.clone();
        timer(REPLY_GIVE_UP_MS, move |c| {
            if !late.finished() {
                let app = late.app.clone();
                c.chats.execute(&app, &erase_js(&late.token), |_| {});
            }
            late.done(c, Sent::NotConfirmed, "no answer from the page");
        });
        self.reply_prep(reply, target, true);
    }

    /// Step 1 of send_on_page. A page that loaded anew meanwhile (the conversation was opened by
    /// going to its address) ends the script with no answer: tried once more when it has loaded.
    fn reply_prep(&mut self, reply: Reply, target: Latest, first: bool) {
        if reply.finished() {
            return;
        }
        let js = prep_js(&reply.text, &target.href, &target.name, reply.start + REPLY_TYPE_BY_MS, &reply.token);
        let app = reply.app.clone();
        self.chats.execute_async(&app, &js, move |r| {
            later(move |c| match prep_of(&r) {
                Err(_) if first && r == "null" => {
                    timer(2500, move |c| c.reply_prep(reply, target, false));
                }
                Err(sent) => reply.done(c, sent, ""),
                Ok(_) if reply.finished() => {} // (given up on meanwhile: nothing was typed)
                Ok(lexical) => {
                    let keys = serde_json::json!({ "text": reply.text }).to_string();
                    let app = reply.app.clone();
                    // keys reach only a page that has the keyboard: the page is told it has it while
                    // the reply is typed (the game keeps the real keyboard; done() tells it no more)
                    c.chats.cdp(&app, "Emulation.setFocusEmulationEnabled", r#"{"enabled":true}"#, move |_| {
                        later(move |c| {
                            if reply.finished() {
                                c.reply_given_up(&reply);
                                return;
                            }
                            let app = reply.app.clone();
                            c.chats.cdp(&app, "Input.insertText", &keys, move |k| {
                                let keys_ok = !k.starts_with("err:");
                                later(move |c| c.reply_typed(reply, lexical, keys_ok));
                            });
                        });
                    });
                }
            });
        });
    }

    /// The time ran out while the reply was being typed or sent: what was typed comes out again, and
    /// the page isn't told it has the keyboard any more (done() did that unless this came later).
    fn reply_given_up(&mut self, reply: &Reply) {
        self.chats.execute(&reply.app, &erase_js(&reply.token), |_| {});
        self.chats.cdp(&reply.app, "Emulation.setFocusEmulationEnabled", r#"{"enabled":false}"#, |_| {});
    }

    /// Step 2 of send_on_page: the text is in the box: Enter as a key, then the check.
    fn reply_typed(&mut self, reply: Reply, lexical: bool, keys_ok: bool) {
        if reply.finished() {
            self.reply_given_up(&reply);
            return;
        }
        let app = reply.app.clone();
        self.chats.execute_async(&app, &typed_js(&reply.text, &reply.token), move |r| {
            let typed = serde_json::from_str::<String>(&r).unwrap_or_else(|_| "no answer".into());
            later(move |c| {
                let how = format!(
                    "{} box, typed by {}",
                    if lexical { "lexical" } else { "plain" },
                    if keys_ok { &typed } else { "script (no keys)" }
                );
                if typed != "keys" && typed != "script" {
                    reply.done(c, Sent::NotConfirmed, &how);
                    return;
                }
                if reply.finished() {
                    c.reply_given_up(&reply);
                    return;
                }
                let app = reply.app.clone();
                c.chats.cdp(&app, "Input.dispatchKeyEvent", ENTER_DOWN, move |_| {
                    later(move |c| {
                        let app = reply.app.clone();
                        c.chats.cdp(&app, "Input.dispatchKeyEvent", ENTER_UP, move |_| {
                            later(move |c| {
                                if reply.finished() {
                                    c.reply_given_up(&reply);
                                    return;
                                }
                                let app = reply.app.clone();
                                let js = confirm_js(&reply.token, reply.start + REPLY_TRY_BY_MS);
                                c.chats.execute_async(&app, &js, move |r| {
                                    let (sent, sent_how) = confirm_of(&r);
                                    later(move |c| reply.done(c, sent, &format!("{how}, sent by {sent_how}")));
                                });
                            });
                        });
                    });
                });
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
        if SENDING.with(|s| s.borrow().contains(app)) {
            return; // a reply is on its way in that conversation
        }
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

    use super::{
        confirm_js, confirm_of, erase_js, fill, latest_js, latest_of, list_read, open_js, prep_js, prep_of, sent_log, shape, typed_js,
        Latest, ListRead, Sent,
    };

    #[test]
    fn a_reply_is_passed_as_json_and_never_logged() {
        let text = "a \"quoted\" ข้อความ\nline 2";
        for js in [prep_js(text, "/messages/t/1/", "Nok", 1, "r1"), typed_js(text, "r1")] {
            assert!(js.contains(r#""a \"quoted\" ข้อความ\nline 2""#));
            for p in ["%TEXT%", "%STEP%", "%COUNT%", "%DEADLINE%", "%TOKEN%", "%HREF%", "%NAME%"] {
                assert!(!js.contains(p), "{p} left");
            }
        }
        for js in [confirm_js("r1", 5), erase_js("r1")] {
            for p in ["%STEP%", "%COUNT%", "%TOKEN%", "%STOP%"] {
                assert!(!js.contains(p), "{p} left");
            }
        }
        assert_eq!(prep_of("\"ready\""), Ok(false));
        assert_eq!(prep_of("\"ready lexical\""), Ok(true));
        assert_eq!(prep_of("\"typing\""), Err(Sent::Typing));
        assert_eq!(prep_of("\"not opened\""), Err(Sent::NotOpened));
        assert_eq!(prep_of("\"no composer\""), Err(Sent::NoComposer));
        assert_eq!(prep_of("garbage"), Err(Sent::NotConfirmed));
        let line = sent_log(&Sent::Confirmed, "secret words");
        assert_eq!(line, "sent: 12 chars, confirmed");
        assert!(!line.contains("secret"));
    }

    #[test]
    fn what_goes_into_a_script_is_never_filled_in_again() {
        // a contact named "%TEXT%" and a reply that says "%NAME%": each stays what it is
        let js = prep_js("%NAME%", "/messages/t/1/", "%TEXT%", 7, "r2");
        assert!(js.ends_with(r#"("/messages/t/1/", "%TEXT%", 7, "r2", "%NAME%")"#), "{}", &js[js.len() - 80..]);
        assert_eq!(fill("a %X% %Y% %Z", &[("%X%", "%Y%"), ("%Y%", "y")]), "a %Y% y %Z");
    }

    #[test]
    fn how_a_reply_went_is_read_without_its_text() {
        let ok = serde_json::to_string(r#"{"r":"confirmed","how":"button","left":"box empty"}"#).unwrap();
        assert_eq!(confirm_of(&ok), (Sent::Confirmed, "button, box empty".to_string()));
        let no = serde_json::to_string(r#"{"r":"not confirmed","how":"no button","left":"box empty"}"#).unwrap();
        assert_eq!(confirm_of(&no).0, Sent::NotConfirmed);
        assert_eq!(confirm_of("null"), (Sent::NotConfirmed, "no answer".to_string()));
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
