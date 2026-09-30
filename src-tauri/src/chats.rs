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
    log, rt, site_log,
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
/// 4. Says when a call is on: a connected call (a voice channel stays connected while everyone is
///    quiet), or a live mic, camera or screen share; and whether it's a call and whether the screen
///    is being shared. A call keeps its app awake, and the edge tab shows a phone or a screen for
///    it. Only weak references: the page still decides when its call and devices go.
/// 5. This app's volume in ChatDock (0-1, sent by ChatDock) on top of the site's own: media elements
///    play at the site's volume times it (the site still reads back its own value), and Web Audio
///    goes through one gain per context in front of the speakers.
const SITE_SCRIPT: &str = r#"(() => {
  if (window.WebSocket) {
    const Real = window.WebSocket;
    // this PC however it is written: 127.x, localhost, *.localhost, ::1, ::, 0.0.0.0, IPv4 inside IPv6
    const thisPc = (url) => {
      try {
        const h = new URL(String(url), location.href).hostname.toLowerCase().replace(/^\[|\]$/g, '').replace(/\.+$/, '');
        return h === 'localhost' || h.endsWith('.localhost') || h === '::1' || h === '::' || h === '0.0.0.0' || /^127\./.test(h) || /^::ffff:(7f[0-9a-f]{2}:|0:0$)/.test(h);
      } catch (e) { return false; }
    };
    const WebSocket = function WebSocket(url, protocols) {
      if (!new.target) throw new TypeError("Failed to construct 'WebSocket': Please use the 'new' operator.");
      const target = thisPc(url) ? 'wss://local-blocked.invalid/' : url;
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
      const o = Object.assign({}, options || {});
      delete o.actions; // page notifications reject buttons (WhatsApp's call Accept / Decline)
      if (!o.tag) delete o.renotify;
      if (o.silent) delete o.vibrate;
      try { new Notification(title, o); } catch (e) {}
      return Promise.resolve();
    };
    sw.getNotifications = function getNotifications() { return Promise.resolve([]); };
  }
  if (window.WeakRef && window.MediaStreamTrack) {
    let held = [];
    let said = '';
    let timer = 0;
    const shares = new WeakSet(); // screen-share tracks
    const isTrack = (x) => x instanceof MediaStreamTrack;
    const keep = (x) => (isTrack(x) ? x.readyState === 'live' : x.connectionState !== 'closed' && x.connectionState !== 'failed');
    const connected = (x) => {
      try { return /^(connecting|connected|disconnected)$/.test(x.connectionState) && x.getTransceivers().length > 0; } catch (e) { return false; }
    };
    const tell = () => {
      held = held.filter((r) => { const x = r.deref(); return x && keep(x); });
      let live = 0;
      let call = false;
      let share = false;
      for (const r of held) {
        const x = r.deref();
        if (!x) continue;
        if (isTrack(x)) {
          live++;
          if (shares.has(x)) share = true;
        } else if (connected(x)) {
          live++;
          call = true;
        }
      }
      const now = JSON.stringify({ type: 'call', live, call, share });
      if (now !== said) {
        said = now;
        try { window.chrome.webview.postMessage(now); } catch (e) {}
      }
      if (held.length && !timer) timer = setInterval(tell, 15000);
      else if (!held.length && timer) { clearInterval(timer); timer = 0; }
    };
    const has = (x) => held.some((r) => r.deref() === x);
    const hold = (list) => {
      for (const x of list) if (!has(x)) held.push(new WeakRef(x));
      tell();
    };
    // a screen share can also end from the browser's own "Stop sharing" bar: hear it right away
    const shared = (t) => { shares.add(t); t.addEventListener('ended', tell); };
    const track = MediaStreamTrack.prototype;
    const realStop = track.stop;
    track.stop = function stop() { const r = realStop.apply(this, arguments); if (has(this)) tell(); return r; };
    const realClone = track.clone;
    track.clone = function clone() {
      const t = realClone.apply(this, arguments);
      if (has(this)) {
        if (shares.has(this)) shared(t);
        hold([t]);
      }
      return t;
    };
    if (window.MediaStream) {
      const realStreamClone = MediaStream.prototype.clone;
      MediaStream.prototype.clone = function clone() {
        const copy = realStreamClone.apply(this, arguments);
        const from = this.getTracks();
        if (from.some(has)) {
          const to = copy.getTracks();
          to.forEach((t, i) => { if (shares.has(from[i])) shared(t); });
          hold(to);
        }
        return copy;
      };
    }
    const md = navigator.mediaDevices;
    for (const name of md ? ['getUserMedia', 'getDisplayMedia'] : []) {
      const original = md[name];
      if (typeof original !== 'function') continue;
      const display = name === 'getDisplayMedia';
      md[name] = function (...args) {
        return original.apply(this, args).then((stream) => {
          try {
            const tracks = stream.getTracks();
            if (display) tracks.forEach((t) => shared(t));
            hold(tracks);
          } catch (e) {}
          return stream;
        });
      };
    }
    if (window.RTCPeerConnection) {
      const pc = RTCPeerConnection.prototype;
      for (const name of ['setLocalDescription', 'setRemoteDescription']) {
        const real = pc[name];
        pc[name] = function (...args) {
          if (!has(this)) {
            this.addEventListener('connectionstatechange', tell);
            hold([this]);
          }
          return real.apply(this, args);
        };
      }
      const realClose = pc.close;
      pc.close = function close() { const r = realClose.apply(this, arguments); if (has(this)) tell(); return r; };
    }
  }
  (() => {
    const desc = window.HTMLMediaElement && Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, 'volume');
    if (!desc || !desc.get || !desc.set) return;
    let level = 1;
    const asked = new WeakMap(); // element -> the volume the site set
    const seen = new WeakSet();
    let elements = [];
    let gains = [];
    const apply = (el) => {
      if (!(el instanceof HTMLMediaElement)) return;
      if (!seen.has(el)) {
        seen.add(el);
        elements.push(new WeakRef(el));
        if (elements.length % 64 === 0) elements = elements.filter((r) => r.deref()); // (gone ones)
      }
      try { desc.set.call(el, Math.max(0, Math.min(1, (asked.has(el) ? asked.get(el) : 1) * level))); } catch (e) {}
    };
    Object.defineProperty(HTMLMediaElement.prototype, 'volume', {
      configurable: true,
      enumerable: desc.enumerable,
      get() { return asked.has(this) ? asked.get(this) : seen.has(this) ? 1 : desc.get.call(this); },
      set(v) {
        const n = Number(v);
        if (!(n >= 0 && n <= 1)) { desc.set.call(this, v); return; } // throws as usual
        asked.set(this, n);
        apply(this);
      },
    });
    const realPlay = HTMLMediaElement.prototype.play;
    HTMLMediaElement.prototype.play = function play() { apply(this); return realPlay.apply(this, arguments); };
    // media that plays without play() or a volume of its own (autoplay on a stream, not in the page)
    for (const prop of ['src', 'srcObject']) {
      const d = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, prop);
      if (d && d.set) {
        Object.defineProperty(HTMLMediaElement.prototype, prop, { ...d, set(v) { d.set.call(this, v); apply(this); } });
      }
    }
    document.addEventListener('play', (e) => { if (e.target instanceof HTMLMediaElement) apply(e.target); }, true);
    const AN = window.AudioNode;
    const ADN = window.AudioDestinationNode;
    if (AN && ADN) {
      const realConnect = AN.prototype.connect;
      const realDisconnect = AN.prototype.disconnect;
      const front = new WeakMap(); // context -> the gain in front of its speakers
      const speakers = (d) => d instanceof ADN && !(window.OfflineAudioContext && d.context instanceof OfflineAudioContext);
      const gainFor = (ctx) => {
        let g = front.get(ctx);
        if (!g) {
          g = ctx.createGain();
          g.gain.value = level;
          realConnect.call(g, ctx.destination);
          front.set(ctx, g);
          gains.push(new WeakRef(g));
        }
        return g;
      };
      AN.prototype.connect = function connect(dest, ...rest) {
        if (speakers(dest)) {
          realConnect.call(this, gainFor(dest.context), ...rest);
          return dest;
        }
        return realConnect.call(this, dest, ...rest);
      };
      AN.prototype.disconnect = function disconnect(dest, ...rest) {
        if (speakers(dest)) {
          const g = front.get(dest.context);
          if (g) return realDisconnect.call(this, g, ...rest);
        }
        return realDisconnect.apply(this, arguments);
      };
    }
    // Frames (a YouTube player in a chat, a Discord activity): the page passes its level down to
    // each frame, and a new frame asks its parent. Handled before the site's own listeners, under
    // this run's own random key, and only once the level has been other than full, so neither the
    // site nor a frame in it can tell who is listening.
    const KEY = '%KEY%';
    let told = false;
    const passDown = () => {
      if (level !== 1) told = true;
      if (!told) return;
      for (let i = 0; i < window.frames.length; i++) {
        try { window.frames[i].postMessage({ [KEY]: level }, '*'); } catch (e) {}
      }
    };
    const setLevel = (v) => {
      level = Math.max(0, Math.min(1, Number(v) || 0));
      elements = elements.filter((r) => { const el = r.deref(); if (el) apply(el); return !!el; });
      gains = gains.filter((r) => { const g = r.deref(); if (g) { try { g.gain.value = level; } catch (e) {} } return !!g; });
      passDown();
    };
    const isChild = (w) => { for (let i = 0; i < window.frames.length; i++) if (window.frames[i] === w) return true; return false; };
    window.addEventListener('message', (e) => {
      const d = e.data;
      if (!d || typeof d !== 'object') return;
      if (KEY in d && e.source === window.parent && window.parent !== window) {
        e.stopImmediatePropagation();
        setLevel(d[KEY]);
      } else if ((KEY + 'Ask') in d && isChild(e.source)) {
        e.stopImmediatePropagation();
        if (told) try { e.source.postMessage({ [KEY]: level }, '*'); } catch (err) {}
      }
    }, true);
    if (window.parent !== window) {
      try { window.parent.postMessage({ [KEY + 'Ask']: 1 }, '*'); } catch (e) {}
    }
    /*SELFTEST*/
    try {
      window.chrome.webview.addEventListener('message', (e) => {
        const d = e.data;
        if (d && d.type === KEY) setLevel(d.level);
        else if (d && d.type === KEY + 'Check') {
          const last = elements.length ? elements[elements.length - 1].deref() : null;
          const g = gains.length ? gains[gains.length - 1].deref() : null;
          window.chrome.webview.postMessage(JSON.stringify({ type: 'volume-state', level, elements: elements.length,
            real: last ? desc.get.call(last) : null, seenBySite: last ? last.volume : null, gain: g ? g.gain.value : null }));
        }
      });
    } catch (e) {}
  })();
})();"#;

/// SITE_SCRIPT as this run injects it: the volume messages under this run's key, and only in a
/// self-test run the mark its checks read (Symbol.for('chatdock.volume')).
fn site_script(selftest: bool) -> String {
    let mark = "try { Object.defineProperty(window, Symbol.for('chatdock.volume'), { get: () => level }); } catch (e) {}";
    SITE_SCRIPT.replace("%KEY%", volume_key()).replace("/*SELFTEST*/", if selftest { mark } else { "" })
}

/// This run's name for the volume messages between ChatDock and the pages (and between the frames
/// in them): random, so no page can know it or tell what it is.
pub fn volume_key() -> &'static str {
    static KEY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    KEY.get_or_init(|| {
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher(); // (seeded by Windows)
        h.write_u64(crate::rt::epoch_ms() as u64);
        format!("_{:016x}", h.finish())
    })
}

pub struct View {
    controller: ICoreWebView2Controller,
    webview: ICoreWebView2,
    playing: bool,
    visible: bool,
    in_use: bool,
    in_call: bool,
    /// the page is in a call (connected), and sharing the screen: the edge tab's icons
    call: bool,
    share: bool,
    /// a call window it opened is open (Messenger, Instagram)
    popup_call: bool,
    low_memory: bool,
}

#[derive(Default)]
pub struct Chats {
    clear_when_made: HashSet<String>,
    clear_after_blank: HashSet<String>,
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
    /// The panel page's own WebView2 (Tauri's): gets the keyboard when no chat is on screen.
    static PANEL_CTRL: RefCell<Option<ICoreWebView2Controller>> = const { RefCell::new(None) };
    /// Apps with a navigation ChatDock started itself (their home page, a retry): allowed wherever it goes.
    static OWN_NAV: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    /// When each app last opened links outside (rate_ok)
    static OUTSIDE: RefCell<HashMap<String, Vec<i64>>> = RefCell::new(HashMap::new());
}

/// Keyboard into the panel page itself (header, settings, welcome and error screens). Tauri's own
/// window focus call does nothing for windows ChatDock shows itself, and it could send the game a
/// synthetic Alt key.
pub fn focus_panel_page() {
    if let Some(c) = PANEL_CTRL.with(|p| p.borrow().clone()) {
        unsafe {
            let _ = c.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
        }
    }
}

fn env() -> Option<ICoreWebView2Environment> {
    ENV.with(|e| e.borrow().clone())
}

static BROWSER_PID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// The longest message the page script sends (its volume state is the longest, ~200 bytes).
const MESSAGE_MAX: usize = 1024;
/// Requests the chat pages made to this PC and ChatDock refused (the self-test counts them).
pub static LOCAL_BLOCKED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The WebView2 browser process the chats run in (0 until the first one is up). Its own windows,
/// like the "… is sharing your screen" bar, count as ChatDock's.
pub fn browser_pid() -> u32 {
    BROWSER_PID.load(std::sync::atomic::Ordering::Relaxed)
}

/// The WebView2 runtime's version ("141.0.3537.71"), for the log.
pub fn browser_version() -> String {
    env()
        .and_then(|e| unsafe {
            let mut p = PWSTR::null();
            e.BrowserVersionString(&mut p).ok().map(|_| take_pwstr(p))
        })
        .unwrap_or_default()
}

/// A page leaving the app opens outside only when the user clicked the link: a web page in the
/// normal browser, e-mail or one of the apps' own programs (apps::APP_SCHEMES). A page can't open
/// either by itself, over and over while a game is in front.
pub fn opens_outside(uri: &str, clicked: bool) -> bool {
    clicked && (uri.starts_with("https://") || uri.starts_with("http://") || apps::app_scheme(uri))
}

/// At most this many links an app opens outside within OUTSIDE_WINDOW_MS (the rest are dropped).
const OUTSIDE_MAX: usize = 5;
const OUTSIDE_WINDOW_MS: i64 = 10_000;

/// Keeps the times of recent events within `window_ms`; false (and this one not kept) when one
/// more would be over `max`.
fn rate_ok(times: &mut Vec<i64>, now: i64, max: usize, window_ms: i64) -> bool {
    times.retain(|t| now - t < window_ms);
    if times.len() >= max {
        return false;
    }
    times.push(now);
    true
}

/// What kind of link it was, for the log (never the address).
fn outside_kind(uri: &str) -> String {
    let scheme = uri.split(':').next().unwrap_or("").to_ascii_lowercase();
    if scheme == "http" || scheme == "https" {
        "a web page".into()
    } else if !scheme.is_empty() && scheme.len() <= 20 && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) {
        format!("a {scheme}: link")
    } else {
        "a link".into()
    }
}

/// A page out of sight that isn't the app in use gives memory back, unless it's in a call.
fn set_memory_level(v: &mut View) {
    let low = !v.visible && !v.in_use && !v.in_call && !v.popup_call && !v.playing; // (music too)
    if v.low_memory != low {
        v.low_memory = low;
        if let Ok(wv19) = v.webview.cast::<ICoreWebView2_19>() {
            let level = if low { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW } else { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL };
            unsafe {
                let _ = wv19.SetMemoryUsageTargetLevel(level);
            }
        }
    }
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

    /// In a call, a voice channel or sharing the screen (as the page says), or in a call window
    /// of its own.
    pub fn in_call(&self, id: &str) -> bool {
        self.views.get(id).is_some_and(|v| v.in_call || v.popup_call)
    }

    /// What the page itself last said: a call, a voice channel or a screen share is live.
    pub fn page_call_live(&self, id: &str) -> bool {
        self.views.get(id).is_some_and(|v| v.in_call)
    }

    pub fn set_popup_call(&mut self, id: &str, on: bool) {
        if let Some(v) = self.views.get_mut(id) {
            v.popup_call = on;
            set_memory_level(v);
        }
    }

    pub fn set_in_call(&mut self, id: &str, on: bool) {
        if let Some(v) = self.views.get_mut(id) {
            v.in_call = on;
            set_memory_level(v);
        }
    }

    /// In a call, and sharing the screen (the edge tab's phone and screen icons).
    pub fn call_state(&self, id: &str) -> (bool, bool) {
        self.views.get(id).map(|v| (v.call, v.share)).unwrap_or_default()
    }

    pub fn set_call_state(&mut self, id: &str, call: bool, share: bool) {
        if let Some(v) = self.views.get_mut(id) {
            v.call = call;
            v.share = share;
        }
    }

    /// The page is giving memory back (for the self-test).
    pub fn memory_low(&self, id: &str) -> bool {
        self.views.get(id).is_some_and(|v| v.low_memory)
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
            v.in_use = in_use;
            set_memory_level(v);
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

    /// Reload without the cache (Shift+F5 / Ctrl+Shift+R).
    pub fn reload_ignoring_cache(&self, id: &str) {
        let Some(v) = self.views.get(id) else { return };
        let wv = v.webview.clone();
        let fallback = wv.clone();
        unsafe {
            let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |r, _| {
                if r.is_err() {
                    let _ = fallback.Reload();
                }
                Ok(())
            }));
            if wv.CallDevToolsProtocolMethod(&HSTRING::from("Page.reload"), &HSTRING::from("{\"ignoreCache\":true}"), &handler).is_err() {
                let _ = wv.Reload();
            }
        }
    }

    /// A DevTools protocol call in an app's page (e.g. Runtime.evaluate as a user gesture); its JSON
    /// answer, or "err:…".
    pub fn cdp(&self, id: &str, method: &str, params: &str, done: impl FnOnce(String) + 'static) {
        let Some(v) = self.views.get(id) else {
            done("err:no view".into());
            return;
        };
        let mut done = Some(done);
        unsafe {
            let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |r, json| {
                if let Some(f) = done.take() {
                    f(if r.is_ok() { json } else { format!("err:{r:?}") });
                }
                Ok(())
            }));
            let _ = v.webview.CallDevToolsProtocolMethod(&HSTRING::from(method), &HSTRING::from(params), &handler);
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

    /// A message to the page's own script (volume).
    pub fn post_json(&self, id: &str, msg: &serde_json::Value) {
        if let Some(v) = self.views.get(id) {
            unsafe {
                let _ = v.webview.PostWebMessageAsJson(&HSTRING::from(msg.to_string()));
            }
        }
    }

    /// The page's sound is off (the self-test).
    pub fn is_muted(&self, id: &str) -> bool {
        let mut on = BOOL(0);
        if let Some(v) = self.views.get(id) {
            if let Ok(wv8) = v.webview.cast::<ICoreWebView2_8>() {
                unsafe {
                    let _ = wv8.IsMuted(&mut on);
                }
            }
        }
        on.as_bool()
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
        let debug = self.args.dev_tools();
        let _ = self.panel.w.with_webview(move |pw| {
            ENV.with(|e| *e.borrow_mut() = Some(pw.environment()));
            PANEL_CTRL.with(|p| *p.borrow_mut() = Some(pw.controller()));
            if let Ok(env5) = pw.environment().cast::<ICoreWebView2Environment5>() {
                let mut token = 0i64;
                unsafe {
                    let _ = env5.add_BrowserProcessExited(
                        &BrowserProcessExitedEventHandler::create(Box::new(|_, args| {
                            let mut kind = COREWEBVIEW2_BROWSER_PROCESS_EXIT_KIND::default();
                            if let Some(args) = args {
                                let _ = args.BrowserProcessExitKind(&mut kind);
                            }
                            if kind == COREWEBVIEW2_BROWSER_PROCESS_EXIT_KIND_FAILED {
                                later(|c| c.restart_after_crash("the WebView2 browser process stopped"));
                            }
                            Ok(())
                        })),
                        &mut token,
                    );
                }
            }
            // the shortcuts work in the panel's own page too (Esc Esc, Ctrl+1..9, ...)
            hook_keys(&pw.controller(), None);
            if let Ok(wv) = unsafe { pw.controller().CoreWebView2() } {
                if let Ok(s) = unsafe { wv.Settings() } {
                    unsafe {
                        let _ = s.SetAreDefaultContextMenusEnabled(debug);
                        let _ = s.SetIsStatusBarEnabled(false);
                        if let Ok(s3) = s.cast::<ICoreWebView2Settings3>() {
                            let _ = s3.SetAreBrowserAcceleratorKeysEnabled(debug);
                        }
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
                                    c.set_load(&a, "error");
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
            self.set_load(id, "error");
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
        if let Err(err) = unsafe { configure(&controller, &webview, id, self.args.dev_tools(), self.args.selftest) } {
            log!("chat view {id} setup: {err}");
        }
        let view = View {
            controller,
            webview,
            playing: false,
            visible: true,
            in_use: false,
            in_call: false,
            call: false,
            share: false,
            popup_call: false,
            low_memory: false,
        };
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
        if self.chats.clear_when_made.remove(id) {
            self.clear_app_data(id);
        }
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
        self.site_counts.insert(id.to_string(), 0);
        self.counted_pages.remove(id);
        self.load_state.insert(id.to_string(), "loading");
        self.first_shown.insert(id.to_string(), false);
    }

    pub fn load_home(&mut self, id: &str) {
        if let Some(a) = apps::get(id) {
            self.chats.navigate(id, a.home);
        }
    }

    pub fn reload_app(&mut self, id: &str, hard: bool) {
        if !self.chats.has(id) {
            // its view could not be made (see create_view): "Try again" makes it again
            if self.is_enabled(id) && !*self.asleep.get(id).unwrap_or(&false) {
                self.create_view(id);
            }
            return;
        }
        if self.load_state.get(id).copied() == Some("error") {
            self.load_home(id);
        } else if hard {
            self.chats.reload_ignoring_cache(id);
        } else {
            self.chats.reload(id);
        }
    }

    pub fn go_home(&mut self, id: &str) {
        let Some(a) = apps::get(id) else { return };
        let source = self.chats.source(id);
        let path = tauri::Url::parse(&source).map(|u| u.path().to_string()).unwrap_or_default();
        // (on another site's page, like a sign-in one, the path says nothing)
        if !path.starts_with(a.home_path) || !apps::owns(id, &source) {
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

    fn on_load_done(&mut self, id: &str, ok: bool, status: i32, http: i32) {
        if self.chats.clear_after_blank.contains(id) && self.chats.source(id) == "about:blank" {
            let a = id.to_string();
            timer(600, move |c| c.clear_now(&a)); // the old page's unload handlers have run by then
            return;
        }
        // The server answered (http > 0): whatever it sent is shown, a "verify you are human" or
        // "try again later" page included, as in 1.4. Cancelled = navigating away, or a link that
        // opened in the browser instead. Only a load with no answer at all is an error.
        // Aborted with a page already on screen: the load was replaced by the next one (a redirect,
        // a second click, a sign-in bouncing between sites) or became a download. The page stays.
        let aborted = status == COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED.0 && *self.first_shown.get(id).unwrap_or(&false);
        if ok
            || http > 0
            || aborted
            || status == COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED.0
            || status == COREWEBVIEW2_WEB_ERROR_STATUS_VALID_AUTHENTICATION_CREDENTIALS_REQUIRED.0
            || status == COREWEBVIEW2_WEB_ERROR_STATUS_VALID_PROXY_AUTHENTICATION_REQUIRED.0
        {
            if !ok {
                log!("page {id} shown although not ok: status {status} http {http}");
            }
            let page = ok || http > 0; // a real page arrived (after an error too: it's over)
            let state = self.load_state.get(id).copied();
            if state == Some("loading") || (state == Some("error") && page) {
                self.set_load(id, "ready");
            }
            if page {
                if let Some(t) = self.retry_timers.remove(id) {
                    rt::cancel(t);
                }
            }
            return;
        }
        log!("load failed {id} status {status} http {http}");
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
        // a new page starts at full volume: it only hears of a lower one (a message at every load
        // would tell the site ChatDock is there)
        if self.app_volume(id) != 100 {
            self.send_volume(id);
        }
        if id == "discord" {
            // its sidebar fills in a moment after the page itself
            for ms in [15_000u64, 60_000] {
                crate::core::timer(ms, |c| c.read_discord_servers());
            }
        }
        if !*self.first_shown.get(id).unwrap_or(&false) {
            self.first_shown.insert(id.to_string(), true);
            self.layout_views();
            self.broadcast_state();
        }
    }

    /// A link a page opens outside (its caller checked it was clicked): through link shims to the
    /// real address, never an address on this PC, and only a few per app in a row.
    pub fn open_external(&self, app: &str, url: &str) {
        let url = apps::unshim(url);
        if apps::is_this_pc(&url) {
            site_log!(app, "{app}: a link to this PC not opened");
            return;
        }
        if !OUTSIDE.with(|o| rate_ok(o.borrow_mut().entry(app.to_string()).or_default(), rt::epoch_ms(), OUTSIDE_MAX, OUTSIDE_WINDOW_MS)) {
            site_log!(app, "{app}: too many links at once, this one not opened");
            return;
        }
        win32::open_url(&url);
    }

    /// Wipe everything an app stored on this PC (login, cookies, cache) and start it fresh.
    pub fn clear_app_data(&mut self, id: &str) {
        self.toasts_dismiss_app(id);
        self.set_count(id, 0);
        // a new login starts from scratch: nothing it will count has been seen yet (the old "seen"
        // would hide the first new messages)
        self.site_counts.insert(id.to_string(), 0);
        self.settings.set_in("seenCounts", id, serde_json::json!(0));
        if id == "discord" {
            self.settings.set("discordServers", serde_json::json!({})); // the servers of the account that left
        }
        self.save_soon();
        self.forget_electron_login(id);
        // and the site's own settings go with the data (Discord's desktop notifications are off
        // again): its count pop-up says where to turn them on until a notification comes
        self.last_content_at.remove(id);
        if self.chats.creating.contains(id) {
            self.chats.clear_when_made.insert(id.to_string()); // done as soon as its view exists
            return;
        }
        let app = id.to_string();
        if self.chats.has(id) {
            // The site's page leaves first: a running chat could otherwise write its login back
            // while (or right after) it is wiped. Cleared once about:blank has loaded.
            self.chats.clear_after_blank.insert(app.clone());
            self.chats.navigate(id, "about:blank");
            let a = app.clone();
            timer(4000, move |c| c.clear_now(&a)); // in case the blank page never reports back
            return;
        }
        // Not loaded (switched off or asleep): open its profile for a moment, without a page.
        if let Err(err) = self.wipe_unloaded_profile(&app) {
            log!("clear data failed {id}: {err}");
        }
    }

    /// The copy of an app's login that the Electron builds (1.4 and older) kept in
    /// <data>\Partitions\<app>: still on disk after the move to WebView2, and still usable.
    fn forget_electron_login(&self, id: &str) {
        let old = self.args.data_dir.join("Partitions").join(id);
        if old.exists() {
            match std::fs::remove_dir_all(&old) {
                Ok(()) => log!("old Electron login removed {id}"),
                Err(err) => log!("old Electron login {id} could not be removed: {err}"),
            }
        }
    }

    /// Everything the Electron builds left of the logins: <data>\Partitions (apps since removed
    /// too) and <data>\Local State, the key their cookies were encrypted with. (WebView2's own
    /// Local State is in <data>\WebView2.)
    pub fn forget_electron_logins(&self) {
        let dir = &self.args.data_dir;
        if dir.join("Partitions").exists() {
            let _ = std::fs::remove_dir_all(dir.join("Partitions"));
        }
        let _ = std::fs::remove_file(dir.join("Local State"));
    }

    fn clear_now(&mut self, id: &str) {
        if !self.chats.clear_after_blank.remove(id) {
            return; // done already
        }
        let result = match self.chats.views.get(id) {
            Some(v) => unsafe { clear_profile(&v.webview, id, None) },
            None => self.wipe_unloaded_profile(id),
        };
        if let Err(err) = result {
            log!("clear data failed {id}: {err}");
        }
    }

    fn wipe_unloaded_profile(&mut self, id: &str) -> windows::core::Result<()> {
        let Some(env) = env() else { return Ok(()) };
        let app = id.to_string();
        unsafe {
            let env10: ICoreWebView2Environment10 = env.cast()?;
            let opts = env10.CreateCoreWebView2ControllerOptions()?;
            opts.SetProfileName(&HSTRING::from(id))?;
            opts.SetIsInPrivateModeEnabled(false)?;
            env10.CreateCoreWebView2ControllerWithOptions(
                win32::h(self.panel.hwnd),
                &opts,
                &CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |err, controller| {
                    match (err, controller) {
                        (Ok(()), Some(ctrl)) => {
                            let _ = ctrl.SetIsVisible(false);
                            let result = ctrl.CoreWebView2().and_then(|wv| clear_profile(&wv, &app, Some(ctrl.clone())));
                            if let Err(err) = result {
                                log!("clear data failed {app}: {err}");
                                let _ = ctrl.Close();
                            }
                        }
                        (err, _) => log!("clear data failed {app}: {err:?}"),
                    }
                    Ok(())
                })),
            )
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
/// Wipe a profile (login, cookies, storage, cache), give the site back its permissions, then load
/// its home page again, or close the stand-in controller used for an app that isn't loaded.
unsafe fn clear_profile(wv: &ICoreWebView2, id: &str, temporary: Option<ICoreWebView2Controller>) -> windows::core::Result<()> {
    let profile = wv.cast::<ICoreWebView2_13>()?.Profile()?;
    let p2 = profile.cast::<ICoreWebView2Profile2>()?;
    let app = id.to_string();
    let keep = profile.clone();
    p2.ClearBrowsingDataAll(&ClearBrowsingDataCompletedHandler::create(Box::new(move |_| {
        grant_site_permissions(&keep, &app);
        if let Some(ctrl) = &temporary {
            let _ = ctrl.Close();
        }
        let a = app.clone();
        later(move |c| {
            if c.chats.has(&a) {
                c.load_home(&a);
            }
            log!("data cleared {a}");
        });
        Ok(())
    })))
}

/// What an app's sites may do from the start, like sites you allowed in a browser (this also
/// covers the call windows WebView2 opens itself): notifications on its own pages; the mic and
/// camera only where its calls and voice messages run (apps::media). 1.5.2–1.7.2 gave the mic and
/// camera to every listed domain (CDNs and sandboxes too): those go back to the default.
unsafe fn grant_site_permissions(profile: &ICoreWebView2Profile, id: &str) {
    let Ok(p4) = profile.cast::<ICoreWebView2Profile4>() else { return };
    let set = |kind, origin: &str, state| {
        let _ =
            p4.SetPermissionState(kind, &HSTRING::from(origin), state, &SetPermissionStateCompletedHandler::create(Box::new(|_| Ok(()))));
    };
    let media = apps::media_origins(id);
    let mut origins = apps::notification_origins(id);
    origins.extend(media.iter().filter(|o| !origins.contains(o)).cloned().collect::<Vec<_>>());
    for origin in &origins {
        set(COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS, origin, COREWEBVIEW2_PERMISSION_STATE_ALLOW);
        let av = if media.contains(origin) { COREWEBVIEW2_PERMISSION_STATE_ALLOW } else { COREWEBVIEW2_PERMISSION_STATE_DEFAULT };
        set(COREWEBVIEW2_PERMISSION_KIND_MICROPHONE, origin, av);
        set(COREWEBVIEW2_PERMISSION_KIND_CAMERA, origin, av);
    }
}

/// A permission a page of the app asks for: the mic and camera only on its call and voice-message
/// pages; notifications, sound without a click and several downloads on any of its pages.
fn permission_allowed(app: &str, kind: COREWEBVIEW2_PERMISSION_KIND, uri: &str) -> bool {
    if kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE || kind == COREWEBVIEW2_PERMISSION_KIND_CAMERA {
        apps::may_use_media(app, uri)
    } else {
        [
            COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS,
            COREWEBVIEW2_PERMISSION_KIND_AUTOPLAY,
            COREWEBVIEW2_PERMISSION_KIND_MULTIPLE_AUTOMATIC_DOWNLOADS,
        ]
        .contains(&kind)
            && apps::owns(app, uri)
    }
}

unsafe fn configure(
    controller: &ICoreWebView2Controller,
    wv: &ICoreWebView2,
    id: &str,
    debug: bool,
    selftest: bool,
) -> windows::core::Result<()> {
    let mut pid = 0u32;
    if wv.BrowserProcessId(&mut pid).is_ok() && pid != 0 {
        BROWSER_PID.store(pid, std::sync::atomic::Ordering::Relaxed); // (a new one after a crash)
    }
    let s = wv.Settings()?;
    s.SetAreDevToolsEnabled(debug)?;
    s.SetIsStatusBarEnabled(false)?;
    s.SetIsZoomControlEnabled(true)?;
    if let Ok(s3) = s.cast::<ICoreWebView2Settings3>() {
        // no find bar / print / save-as on Ctrl+F, Ctrl+P, Ctrl+S (1.4 had none); ChatDock's own
        // keys still arrive through AcceleratorKeyPressed
        s3.SetAreBrowserAcceleratorKeysEnabled(debug)?;
    }
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
        &HSTRING::from(site_script(selftest)),
        &AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(|_, _| Ok(()))),
    )?;
    // What the app's own sites may do from the start (grant_site_permissions)
    if let Ok(profile) = wv.cast::<ICoreWebView2_13>().and_then(|w| w.Profile()) {
        grant_site_permissions(&profile, id);
    }
    // Chat sites have no business talking to programs on this PC. (Discord's page probes the
    // desktop app on localhost and then nags "Discord App Detected".) The filters catch every way
    // this PC can be written, from the pages and from their workers; the handler decides by the
    // parsed host (a filter also matches an address that merely mentions one).
    let wv22 = wv.cast::<ICoreWebView2_22>().ok();
    for f in ["*://127.*", "*://localhost*", "*://*.localhost*", "*://[::1]*", "*://[::]*", "*://0.0.0.0*", "*://[::ffff:*"] {
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
            let (Some(args), Some(env)) = (args, env()) else { return Ok(()) };
            let mut p = PWSTR::null();
            args.Request()?.Uri(&mut p)?;
            if apps::is_this_pc(&take_pwstr(p)) {
                let resp = env.CreateWebResourceResponse(None, 403, &HSTRING::from("Blocked"), &HSTRING::from(""))?;
                args.SetResponse(&resp)?;
                LOCAL_BLOCKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
                let mut clicked = BOOL(0);
                let _ = args.IsUserInitiated(&mut clicked);
                let (a, what) = (app.clone(), outside_kind(&uri));
                if opens_outside(&uri, clicked.as_bool()) {
                    later(move |c| {
                        site_log!(&a, "{a}: {what} opens outside ChatDock");
                        c.open_external(&a, &uri);
                    });
                } else {
                    later(move |_| site_log!(&a, "{a}: {what} not opened (nobody clicked it)"));
                }
            }
            Ok(())
        })),
        &mut token,
    )?;

    let app = id.to_string();
    wv.add_ContentLoading(
        &ContentLoadingEventHandler::create(Box::new(move |_, _| {
            let a = app.clone();
            later(move |c| {
                c.on_call(&a, false, false, false); // a new page: the old one's call ended with it
                c.counted_pages.remove(&a); // and its unread number has to show again (seen_after)
            });
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
            let mut http = 0i32;
            if let Ok(a2) = args.cast::<ICoreWebView2NavigationCompletedEventArgs2>() {
                let _ = a2.HttpStatusCode(&mut http);
            }
            let (a, ok, st) = (app.clone(), ok.as_bool(), status.0);
            later(move |c| c.on_load_done(&a, ok, st, http));
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
            let call = apps::is_call_url(&app, &uri);
            if uri == "about:blank" || call || apps::is_auth_popup(&app, &uri) {
                let (a, kind) = (
                    app.clone(),
                    if call {
                        "a call"
                    } else if uri == "about:blank" {
                        "a blank page"
                    } else {
                        "a sign-in"
                    },
                );
                later(move |c| {
                    site_log!(&a, "{a} opens a window of its own: {kind}");
                    if call {
                        c.call_window_opening(&a);
                    }
                });
                return Ok(());
            }
            args.SetHandled(true)?; // no window of WebView2's own for it
            let mut clicked = BOOL(0);
            let _ = args.IsUserInitiated(&mut clicked);
            let (a, what) = (app.clone(), outside_kind(&uri));
            if opens_outside(&uri, clicked.as_bool()) {
                later(move |c| {
                    site_log!(&a, "{a}: {what} opens outside ChatDock (a new window)");
                    c.open_external(&a, &uri);
                });
            } else {
                later(move |_| site_log!(&a, "{a}: {what} not opened (a new window nobody clicked)"));
            }
            Ok(())
        })),
        &mut token,
    )?;

    // A frame (an ad, an embed) heading for a program's link (ms-msdt:, discord://, …): only a
    // clicked link to e-mail or one of the apps' own programs (top-level pages go through
    // NavigationStarting above). Handling it also stops WebView2's own "open this app?" dialog.
    if let Ok(wv18) = wv.cast::<ICoreWebView2_18>() {
        let app = id.to_string();
        let _ = wv18.add_LaunchingExternalUriScheme(
            &LaunchingExternalUriSchemeEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut p = PWSTR::null();
                args.Uri(&mut p)?;
                let uri = take_pwstr(p);
                let mut clicked = BOOL(0);
                let _ = args.IsUserInitiated(&mut clicked);
                let open = opens_outside(&uri, clicked.as_bool()) && apps::app_scheme(&uri);
                args.SetCancel(!open)?;
                let (a, what) = (app.clone(), outside_kind(&uri));
                later(move |_| site_log!(&a, "{a}: {what} from a frame {}", if open { "opens its program" } else { "not opened" }));
                Ok(())
            })),
            &mut token,
        );
    }

    let app = id.to_string();
    wv.add_PermissionRequested(
        &PermissionRequestedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
            args.PermissionKind(&mut kind)?;
            let mut p = PWSTR::null();
            args.Uri(&mut p)?;
            let uri = take_pwstr(p);
            let allow = permission_allowed(&app, kind, &uri);
            args.SetState(if allow { COREWEBVIEW2_PERMISSION_STATE_ALLOW } else { COREWEBVIEW2_PERMISSION_STATE_DENY })?;
            if !allow {
                site_log!(&app, "permission {} refused for {app}", kind.0);
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
            log!("renderer problem {app} kind {}", kind.0);
            if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED {
                let a = app.clone();
                later(move |c| {
                    if !c.reload_pending.insert(a.clone()) {
                        return; // already coming
                    }
                    let b = a.clone();
                    timer(1500, move |c| {
                        c.reload_pending.remove(&b);
                        c.chats.reload(&b);
                    });
                });
            } else if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED {
                later(|c| c.restart_after_crash("the WebView2 browser process stopped"));
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
                            set_memory_level(v);
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
                    // the page script's messages are short: anything longer is a page's own, not read
                    if msg.len() > MESSAGE_MAX {
                        return Ok(());
                    }
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&msg) {
                        if v["type"] == "passkey" {
                            site_log!(&app, "passkey request blocked {app}: {}", passkey_line(&v));
                        } else if v["type"] == "volume-state" {
                            let (a, m) = (app.clone(), msg.clone());
                            later(move |c| {
                                c.volume_states.insert(a, m);
                            });
                        } else if v["type"] == "call" {
                            let a = app.clone();
                            let (live, call, share) = (v["live"].as_u64().unwrap_or(0) > 0, v["call"] == true, v["share"] == true);
                            later(move |c| c.on_call(&a, live, call, share));
                        }
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

/// A passkey request the page script refused, as the log says it: only fields ChatDock knows (what
/// was asked, the site's name, how), never text of the page's own.
fn passkey_line(v: &serde_json::Value) -> String {
    let kind = v["kind"].as_str().filter(|k| ["get", "create"].contains(k)).unwrap_or("?");
    let site = v["origin"].as_str().and_then(|o| tauri::Url::parse(o).ok()).and_then(|u| u.host_str().map(str::to_string));
    let site = site.filter(|h| h.len() <= 100).unwrap_or_else(|| "?".into());
    let how = match v["mediation"].as_str() {
        Some("") | None => String::new(),
        Some(m @ ("silent" | "optional" | "conditional" | "required")) => format!(" ({m})"),
        Some(_) => " (?)".into(),
    };
    format!("{kind} on {site}{how}")
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

#[cfg(test)]
mod tests {
    use webview2_com::Microsoft::Web::WebView2::Win32::*;

    use super::{opens_outside, passkey_line, permission_allowed, rate_ok, site_script, volume_key};

    #[test]
    fn the_page_script_never_names_chatdock() {
        let plain = site_script(false);
        assert!(!plain.to_lowercase().contains("chatdock"), "a page could tell ChatDock is there");
        assert!(plain.contains(volume_key()) && volume_key().len() == 17);
        assert!(!plain.contains("%KEY%") && !plain.contains("/*SELFTEST*/"));
        assert!(site_script(true).contains("Symbol.for('chatdock.volume')")); // what the self-test reads
    }

    #[test]
    fn a_passkey_line_holds_no_page_text() {
        let v = serde_json::json!({ "type": "passkey", "kind": "get", "origin": "https://www.instagram.com", "mediation": "conditional" });
        assert_eq!(passkey_line(&v), "get on www.instagram.com (conditional)");
        let v = serde_json::json!({ "type": "passkey", "kind": "steal\nthe log", "origin": "not a url", "mediation": "x\ny" });
        assert_eq!(passkey_line(&v), "? on ? (?)");
        let v = serde_json::json!({ "type": "passkey", "kind": "create", "origin": "https://x.com" });
        assert_eq!(passkey_line(&v), "create on x.com");
    }

    #[test]
    fn nothing_opens_outside_without_a_click() {
        assert!(!opens_outside("https://example.com/", false));
        assert!(!opens_outside("mailto:a@b.c", false));
        assert!(opens_outside("https://example.com/", true));
        assert!(opens_outside("spotify:track:1", true));
        assert!(!opens_outside("ms-msdt:/id x", true)); // a program outside the allowlist, clicked or not
    }

    #[test]
    fn a_few_links_in_a_row_then_no_more() {
        let mut times = Vec::new();
        for i in 0..5 {
            assert!(rate_ok(&mut times, 1000 + i, 5, 10_000));
        }
        assert!(!rate_ok(&mut times, 2000, 5, 10_000));
        assert!(rate_ok(&mut times, 12_000, 5, 10_000)); // the window has moved on
    }

    #[test]
    fn permissions_follow_the_catalog() {
        assert!(permission_allowed("facebook", COREWEBVIEW2_PERMISSION_KIND_MICROPHONE, "https://www.facebook.com/groupcall/1"));
        assert!(!permission_allowed("facebook", COREWEBVIEW2_PERMISSION_KIND_CAMERA, "https://www.fbsbx.com/"));
        assert!(!permission_allowed("spotify", COREWEBVIEW2_PERMISSION_KIND_MICROPHONE, "https://open.spotify.com/"));
        assert!(permission_allowed("spotify", COREWEBVIEW2_PERMISSION_KIND_AUTOPLAY, "https://open.spotify.com/"));
        assert!(permission_allowed("facebook", COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS, "https://www.facebook.com/"));
        assert!(!permission_allowed("discord", COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION, "https://discord.com/"));
    }
}
