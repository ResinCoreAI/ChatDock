'use strict';

// ChatDock — chat apps (Instagram, Facebook, X, Discord, ...) hidden on the right or left edge of the screen.
//
//   hidden ──(cursor rests on the dock edge)──> white tab ──(click)──> panel slides in
//   hidden ──(global hotkey / tray click)───────────────────────────> panel slides in
//   panel  ──(click elsewhere / hotkey / Esc Esc / hide button)──────> slides out, focus goes back to the game

const fs = require('node:fs');
const path = require('node:path');
const { execFile } = require('node:child_process');
const {
  app, BrowserWindow, WebContentsView, Menu, Tray, globalShortcut,
  ipcMain, nativeTheme, protocol, screen, session, shell, clipboard, desktopCapturer,
} = require('electron');
const settings = require('./settings');
const win32 = require('./win32');
const logger = require('./logger');
const apps = require('./apps');
const toasts = require('./toasts');
const updater = require('./updater');
const i18n = require('./ui/i18n');
const pkg = require('../package.json');

// ---------------------------------------------------------------------------
// Command line
// ---------------------------------------------------------------------------
const argv = process.argv.slice(1);
const argValue = (name) => {
  const hit = argv.find((a) => a.startsWith(`--${name}=`));
  return hit ? hit.slice(name.length + 3) : null;
};
const profileDir = argValue('profile'); // separate userData folder (used for testing)
if (profileDir) app.setPath('userData', path.resolve(profileDir));
const SELFTEST = argv.includes('--selftest');
const STRESS = argv.includes('--stress') ? 'full' : argValue('stress'); // dev: real-input stress test
const DEMO = argValue('demo-frames'); // dev: record the README animation (see src/demo.js)
const DEBUG = SELFTEST || STRESS || DEMO || argv.includes('--debug');
const START_HIDDEN = argv.includes('--hidden'); // launched by "start with Windows"
// WebAuthenticationUseNativeWinApi off: no site can ever open Windows' own passkey dialog
// ("Windows Security"), whatever frame or trick it uses. Login pages (Discord, Meta) request passkeys
// on their own, which popped that dialog mid-game. preload-site.js also refuses them up front.
// WebRtcHideLocalIpsWithMdns off: it answers mDNS on UDP 5353, which sets off a Windows Firewall
// prompt. Not needed here: the chat views never reveal local addresses to WebRTC at all
// (setWebRTCIPHandlingPolicy 'default_public_interface_only' in createView).
const disabledFeatures = ['WebAuthenticationUseNativeWinApi', 'WebRtcHideLocalIpsWithMdns'];
// Less RAM: no spare renderer process kept warm, no back/forward cache of pages we navigated away
// from, and ChatDock's own small pages (panel, tab, glow, pop-ups) share one process.
// (--no-ram-tweaks switches these off, to measure the difference.)
const RAM_TWEAKS = !argv.includes('--no-ram-tweaks');
if (RAM_TWEAKS) {
  disabledFeatures.push('SpareRendererForSitePerProcess', 'BackForwardCache');
  app.commandLine.appendSwitch('process-per-site');
}
// Testing aid: Chromium treats every window as covered while the PC is locked; this turns that off.
if (argv.includes('--no-occlusion')) disabledFeatures.push('CalculateNativeWinOcclusion');
app.commandLine.appendSwitch('disable-features', disabledFeatures.join(','));
app.enableSandbox(); // every renderer sandboxed, including call / sign-in windows the sites open
// ChatDock's own pages come from chatdock://ui/ instead of file://, so file:// keeps Chromium's
// normal restrictions (release builds switch off the GrantFileProtocolExtraPrivileges fuse).
protocol.registerSchemesAsPrivileged([{ scheme: 'chatdock', privileges: { standard: true, secure: true } }]);

logger.init({ console: DEBUG });
const log = logger.log;
process.on('uncaughtException', (err) => log('UNCAUGHT', err));

// ChatDock's own text, in the chosen language ('auto' = the Windows language); see src/ui/i18n.js
let t = i18n.make('en');

function uiLang() {
  const chosen = settings.get('lang');
  if (i18n.IDS.includes(chosen)) return chosen;
  return i18n.pick(app.getPreferredSystemLanguages().concat(app.getLocale()));
}

function applyLanguage() {
  t = i18n.make(uiLang());
}

// Releases are called "Beta Build N" (version 1.N.0)
const buildName = (version) => i18n.buildName(t, version);
process.on('unhandledRejection', (err) => log('UNHANDLED REJECTION', err));

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------
const AUMID = 'com.chatdock.app'; // = build.appId, so the installer's shortcuts belong to this app
const REPO_URL = 'https://github.com/ResinCoreAI/ChatDock';
// Set by the release build (electron-builder extraMetadata) together with the EnableCookieEncryption fuse.
const COOKIE_ENCRYPTION = app.isPackaged && pkg.cookieEncryption === true;
const ASSETS = path.join(__dirname, '..', 'assets');
const UI = path.join(__dirname, 'ui');
const PRELOAD = path.join(__dirname, 'preload.js');
const SITE_PRELOAD = path.join(__dirname, 'preload-site.js');

// Chat services live in apps.js; the user switches them on/off in the menu.
const A = apps.get;

function enabledApps() {
  const on = settings.get('apps') || {};
  return apps.ALL_IDS.filter((id) => on[id]);
}

function isEnabled(id) {
  return !!A(id) && !!(settings.get('apps') || {})[id];
}

const HOTKEYS = [
  { acc: 'Control+Alt+C', label: 'Ctrl + Alt + C' },
  { acc: 'Control+Alt+Space', label: 'Ctrl + Alt + Space' },
  { acc: 'Control+Shift+Space', label: 'Ctrl + Shift + Space' },
  { acc: 'Control+Alt+Z', label: 'Ctrl + Alt + Z' },
];

// Layout (keep in sync with --header-h / --banner-h / --grip-w in ui/panel.css)
const HEADER_H = 48;
const BANNER_H = 52;
const GRIP_W = 6;
const MIN_W = 340;

const TAB_W = 60; // tab window: the white pill plus room for its shadow
const tabHeight = () => 74 + 40 * enabledApps().length; // one 32px icon + 8px gap per app (see tab.css)
const GLOW_W = 32; // Windows won't make these windows narrower than 32px; the strip is drawn at its right edge
const GLOW_H = 128;

const EDGE_PX = 2; // cursor within this many px of the dock edge counts as "on the edge"
const DWELL_MS = 150; // how long it has to rest there before the tab appears
const TAB_LINGER_MS = 900; // tab stays this long after the cursor wanders off
const TAB_FOLLOW_MARGIN = 26; // sliding along the edge, the cursor stays this far inside the pill
const TAB_GLIDE = 0.35; // share of the remaining distance the tab covers per frame (~60 fps)
const OPEN_MS = 200;
const CLOSE_MS = 160;
const AUTO_RETRY_MS = 15000;
const ZOOM_STEPS = [0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5];

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------
let panelWin = null;
let tabWin = null;
let glowWin = null;
let tray = null;
let ownHwnds = [];
const views = {}; // only switched-on apps have a view
const perApp = (value) => Object.fromEntries(apps.ALL_IDS.map((id) => [id, value]));
const counts = perApp(0);
const loadState = perApp('loading'); // loading | ready | error
const firstShown = perApp(false); // page has rendered at least once
const zeroTimers = {};
const retryTimers = {};
const lastFlash = {}; // "Somchai sent you a message" titles, used when a site gives no notification text
const lastContentAt = {}; // when the site last handed us a real notification (sender + text)
const fallbackTimers = {};
const loadStartedAt = {}; // a freshly (re)loaded page shows old unread counts; that's not a new message
const COUNT_POPUP_GRACE_MS = 20000;
// RAM saver: a sleeping app has no page loaded (see sleepApp)
const asleep = perApp(false);
const lastUsed = perApp(Date.now()); // last time the app was on screen, playing sound or in a call
const childWindows = perApp(0); // open call / sign-in windows
const SLEEP_AFTER_MS = 10 * 60 * 1000;
let settingsTimer = null; // refreshes the RAM figures while the settings screen is open
let updateWin = null; // "Updating ChatDock" window

let panelState = 'hidden'; // hidden | opening | open | closing
let helpMode = false; // welcome / how-to screen instead of the chats
let settingsMode = false; // settings screen instead of the chats
let bannerShown = false; // "exclusive fullscreen game" hint
let bannerDismissed = false;
let prevForeground = 0; // window to give focus back to when the panel closes
let lastAutoHideAt = 0;
let blurredWhileOpening = false;
let lastEscAt = 0;
let anim = null;
let quitting = false;
let hotkeyOk = false;
let autostartCache = false;

let toastForeground = 0; // what was in front when a pop-up was clicked
let dndTimer = null;
let updateAnnounced = ''; // version the update pop-up was already shown for

let tabShown = false;
let tabShownAt = 0;
let tabForeground = 0;
let tabLastForeground = 0;
let tabCenterY = null;
let tabY = 0; // where the tab is drawn (top edge, can be fractional while it glides)
let tabTargetY = 0; // where it is heading
let tabDrawnY = null;
let lastInsideAt = 0;
let dwellStart = 0;
let tabHideTimer = null;
let edgeTimer = null;
let trayUnread = null;

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------
app.setAppUserModelId(AUMID);
// Meta serves its normal desktop site to a plain Chrome user agent.
app.userAgentFallback = app.userAgentFallback.replace(/\s(?:Electron|chatdock)\/\S+/gi, '');

if (!app.requestSingleInstanceLock()) {
  app.quit();
} else {
  app.on('second-instance', () => openPanel(null, 'launch'));
  app.whenReady().then(init);
}

app.on('window-all-closed', () => {}); // keep living in the tray
app.on('before-quit', () => {
  log('before-quit');
  quitting = true;
  settings.flush();
});
app.on('will-quit', () => {
  log('will-quit');
  globalShortcut.unregisterAll();
});
app.on('quit', () => log('quit'));

function init() {
  settings.load();
  applyLanguage();
  registerUiProtocol();
  nativeTheme.themeSource = settings.get('theme');
  Menu.setApplicationMenu(null);
  autostartCache = readOpenAtLogin();

  if (!enabledApps().length) settings.set('apps', { ...settings.get('apps'), instagram: true });
  if (!isEnabled(settings.get('active'))) settings.set('active', enabledApps()[0]);

  createPanel();
  createTab();
  createGlow();
  toasts.init({
    webPreferences: uiWebPreferences,
    lockDown,
    uiUrl,
    hwndOf: win32.hwndOf,
    quitting: () => quitting,
    raise,
    targetDisplay,
    panelBounds: () => (panelState === 'open' || panelState === 'opening' ? panelGeometry() : null),
    panelActive: () => panelState === 'open' || panelState === 'opening',
    position: () => settings.get('popupPosition'),
    duration: () => settings.get('popupDuration'),
    maxVisible: () => settings.get('popupMax'),
    t: (...args) => t(...args),
    lang: () => uiLang(),
    foreground: () => win32.foregroundWindow(),
    restoreFocus: (hwnd) => win32.restoreForeground(hwnd),
    open: openFromToast,
  });
  toasts.setContentProtection(!!settings.get('hideFromCapture'));
  ownHwnds = [panelWin, tabWin, glowWin].map(win32.hwndOf).concat(toasts.hwnd());
  for (const id of enabledApps()) {
    if (sleepEligible(id)) asleep[id] = true; // loads when it is first opened
    else createView(id);
  }
  setInterval(sleepCheck, 60 * 1000);
  createTray();
  registerIpc();
  registerHotkey();
  scheduleDndEnd();
  migrateLoginItem();
  updater.init({
    log,
    autoCheck: () => !!settings.get('updateAutoCheck'),
    autoDownload: () => !!settings.get('updateAutoDownload'),
    onState: () => broadcastState(),
    onAvailable: (s) => { if (!settings.get('updateAutoDownload')) announceUpdate(s); },
    onReady: (s) => announceUpdate(s),
    beforeInstall: async (s) => {
      log('installing update', s.version);
      // Remembered so the new version can say "updated" and show what's new when it starts.
      settings.set('pendingUpdate', { from: app.getVersion(), to: s.version, notes: s.notes || '' });
      settings.flush();
      toasts.dismissAll();
      hideTab(true);
      if (panelState !== 'hidden') closePanel(false, 'update');
      await showUpdateWindow(s);
      quitting = true;
      settings.flush();
    },
    // Still running well after handing over to the installer: it never started. Carry on as before.
    installFailed: () => {
      log('update install did not start');
      quitting = false;
      settings.set('pendingUpdate', null);
      if (updateWin && !updateWin.isDestroyed()) updateWin.destroy();
      updateWin = null;
      broadcastState();
    },
  });
  const updatedFrom = detectUpdate();
  if (updatedFrom) setTimeout(announceUpdated, 3500); // once the chats have started loading
  if (COOKIE_ENCRYPTION && !settings.get('cookiesMigrated')) {
    setTimeout(() => encryptOldCookies().catch((err) => log('cookie re-encryption failed', err)), 45000);
  }

  screen.on('display-metrics-changed', onDisplaysChanged);
  screen.on('display-added', onDisplaysChanged);
  screen.on('display-removed', onDisplaysChanged);
  nativeTheme.on('updated', onThemeUpdated);

  if (!win32.available()) log('win32 helpers unavailable:', win32.loadError());
  edgeTick();

  if (DEMO) settings.set('onboarded', true);
  if (!settings.get('onboarded')) {
    helpMode = true;
    panelWin.webContents.once('did-finish-load', () => openPanel(null, 'startup'));
  }
  if (SELFTEST) runSelfTest().catch((err) => log('selftest crashed', err));
  if (DEMO) startDemo();
  if (STRESS) {
    const ctx = {
      log,
      display: () => targetDisplay(),
      geometry: () => panelGeometry(),
      tabBounds: () => tabWin.getBounds(),
      state: () => ({ ...snap(), x: panelWin.getBounds().x }),
      hwnds: () => ownHwnds,
      mode: STRESS,
      enabledApps,
      side: () => settings.get('side'),
      setPref,
      openSettings: (section) => openSettings(section || '', 'stress'),
      // screen point of an element of the panel's own page (header buttons, settings controls)
      panelPoint: async (selector) => {
        const r = await panelWin.webContents.executeJavaScript(
          `(() => { const el = document.querySelector(${JSON.stringify(selector)}); if (!el) return null;
             el.scrollIntoView({ block: 'nearest' }); const r = el.getBoundingClientRect();
             return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
        if (!r) return null;
        const b = panelWin.getContentBounds();
        return { x: Math.round(b.x + r.x), y: Math.round(b.y + r.y) };
      },
      // screen point of an app icon on the edge tab (layout depends on how many apps are on)
      tabIconPoint: async (id) => {
        const r = await tabWin.webContents.executeJavaScript(
          `(() => { const b = document.querySelector('.app[data-app="${id}"]'); if (!b) return null;
             const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
        if (!r) return null;
        const t = tabWin.getBounds();
        return { x: Math.round(t.x + r.x), y: Math.round(t.y + r.y) };
      },
      // make the site itself raise a web notification, exactly like a new message would
      notify: (id, title, body) => views[id] && views[id].webContents.executeJavaScript(
        `(() => { const n = new Notification(${JSON.stringify(title)}, { body: ${JSON.stringify(body)} });
             n.onclick = () => { window.__chatdockClicked = (window.__chatdockClicked || 0) + 1; }; return true; })()`),
      pageClicks: (id) => views[id].webContents.executeJavaScript('window.__chatdockClicked || 0'),
      setCount,
      capture: async (name) => { // whole-screen grab (use only over the FAKE GAME backdrop)
        const dir = argValue('shots');
        if (!dir) return;
        const d = targetDisplay();
        const sources = await desktopCapturer.getSources({
          types: ['screen'],
          thumbnailSize: { width: Math.round(d.size.width * d.scaleFactor), height: Math.round(d.size.height * d.scaleFactor) },
        });
        const src = sources.find((s) => s.display_id === String(d.id)) || sources[0];
        if (!src) return;
        fs.mkdirSync(dir, { recursive: true });
        fs.writeFileSync(path.join(dir, `${name}.png`), src.thumbnail.toPNG());
        log('captured', name);
      },
      toast: () => toasts.snapshot(),
      toastCardPoint: async (which) => {
        const wc = toasts.webContents();
        const r = await wc.executeJavaScript(
          `(() => { const c = document.querySelector('.card'); if (!c) return null;
             const el = ${which === 'close' ? "c.querySelector('.close')" : 'c'}; const r = el.getBoundingClientRect();
             return { x: r.x + ${which === 'close' ? 'r.width / 2' : '60'}, y: r.y + r.height / 2 }; })()`);
        if (!r) return null;
        const b = toasts.snapshot().bounds;
        return { x: Math.round(b.x + r.x), y: Math.round(b.y + r.y) };
      },
    };
    if (argValue('side')) setPref('side', argValue('side')); // run the stress test on the left edge too
    setTimeout(() => {
      let stress;
      try {
        stress = require('./stress'); // dev checkouts only; not shipped in the packaged app
      } catch {
        log('stress test is not included in this build');
        return;
      }
      stress.run(ctx)
        .catch((err) => log('stress crashed', err))
        .finally(() => { if (!argv.includes('--keep')) quit(); });
    }, 9000);
  }
  log('ready', {
    version: app.getVersion(), packaged: app.isPackaged, cookieEncryption: COOKIE_ENCRYPTION,
    hidden: START_HIDDEN, profile: app.getPath('userData'), exe: process.execPath,
  });
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------
const UI_TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
};

// Serves src/ui only, on the default session only: the chat sites (own sessions) can't reach it.
function registerUiProtocol() {
  protocol.handle('chatdock', async (req) => {
    try {
      const u = new URL(req.url);
      const file = path.normalize(path.join(UI, decodeURIComponent(u.pathname)));
      const type = UI_TYPES[path.extname(file).toLowerCase()];
      if (u.host !== 'ui' || !type || !file.startsWith(UI + path.sep)) return new Response('Not found', { status: 404 });
      return new Response(await fs.promises.readFile(file), { headers: { 'content-type': type } });
    } catch {
      return new Response('Not found', { status: 404 });
    }
  });
}

const uiUrl = (page) => `chatdock://ui/${page}`;

function uiWebPreferences(extra = {}) {
  return { preload: PRELOAD, contextIsolation: true, sandbox: true, nodeIntegration: false, spellcheck: false, ...extra };
}

function lockDown(wc) {
  wc.on('will-navigate', (e) => e.preventDefault());
  wc.setWindowOpenHandler(() => ({ action: 'deny' }));
}

function createPanel() {
  const g = panelGeometry();
  panelWin = new BrowserWindow({
    x: hiddenX(g), y: g.y, width: g.width, height: g.height,
    show: false, frame: false, resizable: false, minimizable: false, maximizable: false, fullscreenable: false,
    skipTaskbar: true, alwaysOnTop: true, thickFrame: false, roundedCorners: false, hasShadow: false,
    backgroundColor: themeBg(), title: 'ChatDock', icon: path.join(ASSETS, 'icon.ico'),
    webPreferences: uiWebPreferences(),
  });
  panelWin.setAlwaysOnTop(true, 'screen-saver');
  panelWin.setContentProtection(!!settings.get('hideFromCapture'));
  lockDown(panelWin.webContents);
  panelWin.webContents.on('before-input-event', (e, input) => onPanelKey(e, input, 'host'));
  panelWin.on('blur', onPanelBlur);
  panelWin.on('close', (e) => {
    if (quitting) return;
    e.preventDefault();
    closePanel(true, 'alt-f4');
  });
  panelWin.loadURL(uiUrl('panel.html'));
}

function createTab() {
  tabWin = new BrowserWindow({
    width: TAB_W, height: tabHeight(), show: false, frame: false, transparent: true, resizable: false,
    minimizable: false, maximizable: false, fullscreenable: false, skipTaskbar: true,
    // Must stay focusable: Chromium throws away clicks on a non-focusable window once it has been
    // hidden and shown again (only the very first click worked). It is shown with showInactive(),
    // so hovering never takes focus from the game; only a real click does, and that opens the panel.
    focusable: true,
    alwaysOnTop: true, hasShadow: false, thickFrame: false, roundedCorners: false,
    webPreferences: uiWebPreferences({ backgroundThrottling: false }),
  });
  tabWin.setAlwaysOnTop(true, 'screen-saver');
  lockDown(tabWin.webContents);
  tabWin.on('close', (e) => { if (!quitting) e.preventDefault(); });
  tabWin.on('focus', () => {
    // A click that didn't open the panel (e.g. the too-early click guard) must not leave the game unfocused.
    setTimeout(() => {
      if (panelState !== 'hidden' || win32.foregroundWindow() !== ownHwnds[1]) return;
      if (tabForeground && !isOurs(tabForeground)) {
        win32.restoreForeground(tabForeground);
        if (tabShown) raise(tabWin); // a topmost game just came to the front; stay above it
        log('tab took focus without opening the panel; focus given back');
      }
    }, 200);
  });
  tabWin.loadURL(uiUrl('tab.html'));
}

function createGlow() {
  glowWin = new BrowserWindow({
    width: GLOW_W, height: GLOW_H, show: false, frame: false, transparent: true, resizable: false,
    minimizable: false, maximizable: false, fullscreenable: false, skipTaskbar: true, focusable: false,
    alwaysOnTop: true, hasShadow: false, thickFrame: false, roundedCorners: false,
    webPreferences: uiWebPreferences({ backgroundThrottling: false }),
  });
  glowWin.setAlwaysOnTop(true, 'screen-saver');
  glowWin.setIgnoreMouseEvents(true); // purely visual, clicks go straight through
  lockDown(glowWin.webContents);
  glowWin.on('close', (e) => { if (!quitting) e.preventDefault(); });
  glowWin.loadURL(uiUrl('glow.html'));
}

function raise(win) {
  // Each call re-inserts the window at the top of the topmost band (above games that are topmost
  // themselves). 'screen-saver' is one of the levels Electron keeps above the taskbar on Windows.
  win.setAlwaysOnTop(true, 'screen-saver');
}

// ---------------------------------------------------------------------------
// Chat views (the real web apps: Instagram, Facebook, X, Discord, ...)
// ---------------------------------------------------------------------------
const configuredSessions = new WeakSet();
const ALLOWED_PERMISSIONS = new Set([
  // notifications: the sites announce new messages; preload-site.js turns each one into a
  // ChatDock pop-up instead of a Windows toast
  'notifications', 'media', 'fullscreen', 'clipboard-sanitized-write', 'speaker-selection', 'pointerLock',
  'storage-access', 'top-level-storage-access', 'persistent-storage',
]);

function configureSession(ses, id) {
  if (configuredSessions.has(ses)) return;
  configuredSessions.add(ses);
  // Chat sites have no business talking to programs on this PC. (Discord's page probes the
  // desktop app on localhost and then nags "Discord App Detected".)
  const local = ['127.0.0.1', 'localhost', '[::1]'];
  const urls = local.flatMap((h) => ['http', 'https', 'ws', 'wss'].map((s) => `${s}://${h}/*`)); // any port
  ses.webRequest.onBeforeRequest({ urls }, (_details, callback) => callback({ cancel: true }));
  // Passkeys off for every page and every frame inside it (a frame can't turn back on what its
  // page switched off). Login pages otherwise pop a "Windows Security" passkey dialog by themselves.
  const noPasskeys = 'publickey-credentials-get=(), publickey-credentials-create=()';
  ses.webRequest.onHeadersReceived((details, callback) => {
    if (details.resourceType !== 'mainFrame' && details.resourceType !== 'subFrame') {
      callback({});
      return;
    }
    const headers = { ...details.responseHeaders };
    const key = Object.keys(headers).find((k) => k.toLowerCase() === 'permissions-policy');
    if (key) headers[key] = [`${[].concat(headers[key]).join(', ')}, ${noPasskeys}`]; // last entry wins
    else headers['Permissions-Policy'] = [noPasskeys];
    callback({ responseHeaders: headers });
  });
  ses.setPermissionRequestHandler((wc, permission, callback, details) => {
    const url = (details && details.requestingUrl) || (wc && wc.getURL()) || '';
    callback(ALLOWED_PERMISSIONS.has(permission) && apps.owns(id, url));
  });
  ses.setPermissionCheckHandler((_wc, _permission, origin) => apps.owns(id, origin));
}

// Wipe everything an app stored on this PC (login, cookies, cache) and start it fresh.
async function clearAppData(id) {
  const a = A(id);
  if (!a) return;
  const ses = session.fromPartition(a.partition);
  toasts.dismissApp(id);
  try {
    await ses.clearStorageData();
    await ses.clearCache();
    await ses.clearAuthCache();
  } catch (err) {
    log('clear data failed', id, err);
  }
  setCount(id, 0);
  if (views[id]) loadHome(id);
  log('data cleared', id);
}

// Cookies saved before cookie encryption was switched on (the portable 1.0 build) are still plain
// text on disk: Chromium only encrypts a cookie when it writes it. Write each app's own cookies
// once more so all of them are stored encrypted with this Windows account's key (DPAPI).
// A cookie that can't be written back simply stays as it was.
async function encryptOldCookies() {
  let done = 0;
  let failed = 0;
  for (const a of apps.CATALOG) {
    const ses = session.fromPartition(`persist:${a.id}`);
    const own = (domain) => {
      const host = String(domain || '').replace(/^\./, '');
      return a.domains.some((d) => host === d || host.endsWith(`.${d}`));
    };
    let list = [];
    try {
      list = await ses.cookies.get({});
    } catch {
      continue;
    }
    for (const c of list) {
      if (!own(c.domain)) continue; // third-party cookies inside frames: leave them to the sites
      const host = c.domain.replace(/^\./, '');
      const details = {
        url: `${c.secure ? 'https' : 'http'}://${host}${c.path || '/'}`,
        name: c.name,
        value: c.value,
        path: c.path,
        secure: c.secure,
        httpOnly: c.httpOnly,
        sameSite: c.sameSite,
      };
      if (!c.hostOnly) details.domain = c.domain;
      if (!c.session && c.expirationDate) details.expirationDate = c.expirationDate;
      try {
        await ses.cookies.set(details);
        done += 1;
      } catch {
        failed += 1;
      }
    }
    try {
      await ses.cookies.flushStore();
    } catch {
      // written on the next flush
    }
  }
  settings.set('cookiesMigrated', true);
  log('cookies re-encrypted', { done, failed });
  broadcastState();
}

function createView(id) {
  if (views[id]) return;
  const cfg = A(id);
  configureSession(session.fromPartition(cfg.partition), id);
  const view = new WebContentsView({
    webPreferences: {
      partition: cfg.partition, preload: SITE_PRELOAD, contextIsolation: true, sandbox: true,
      nodeIntegration: false, spellcheck: false, zoomFactor: zoomOf(id),
      nodeIntegrationInSubFrames: true, // run the passkey guard inside login iframes too (no Node, still sandboxed)
    },
  });
  view.setBackgroundColor(themeBg());
  view.setVisible(false);
  panelWin.contentView.addChildView(view);
  views[id] = view;

  const wc = view.webContents;
  // WebRTC may only use the default route and never lists this PC's local addresses. This also
  // keeps Chromium from opening an mDNS listener (UDP 5353), which set off Windows Firewall prompts.
  wc.setWebRTCIPHandlingPolicy('default_public_interface_only');
  applyAudio(id);
  wc.on('audio-state-changed', () => { lastUsed[id] = Date.now(); }); // playing sound counts as in use
  wc.setWindowOpenHandler((details) => onWindowOpen(id, details));
  wc.on('did-create-window', (child) => guardPopup(id, child));
  wc.on('will-navigate', (e, url) => {
    if (keepInside(id, url)) return;
    e.preventDefault();
    openExternal(url);
  });
  wc.on('page-title-updated', (_e, title) => onTitle(id, title));
  wc.on('did-start-loading', () => {
    loadStartedAt[id] = Date.now();
    setLoad(id, 'loading');
  });
  wc.on('did-stop-loading', () => { if (loadState[id] === 'loading') setLoad(id, 'ready'); });
  wc.on('dom-ready', () => {
    applyZoom(id);
    if (!firstShown[id]) {
      firstShown[id] = true;
      layoutViews();
      broadcastState();
    }
  });
  wc.on('did-fail-load', (_e, code, desc, url, isMainFrame) => {
    if (!isMainFrame || code === -3) return; // -3 = aborted, normal when navigating away
    log('load failed', id, code, desc, url);
    setLoad(id, 'error');
    clearTimeout(retryTimers[id]);
    retryTimers[id] = setTimeout(() => { if (loadState[id] === 'error') reloadApp(id); }, AUTO_RETRY_MS);
  });
  wc.on('render-process-gone', (_e, details) => {
    log('renderer gone', id, details.reason);
    if (details.reason !== 'clean-exit') setTimeout(() => { if (!wc.isDestroyed()) wc.reload(); }, 1500);
  });
  wc.on('before-input-event', (e, input) => onPanelKey(e, input, id));
  wc.on('context-menu', (_e, params) => showContextMenu(wc, params));
  wc.on('zoom-changed', (_e, dir) => zoomStep(id, dir === 'in' ? 1 : -1));
  loadHome(id);
}

function destroyView(id) {
  const view = views[id];
  if (!view) return;
  delete views[id];
  panelWin.contentView.removeChildView(view);
  if (!view.webContents.isDestroyed()) view.webContents.close();
  clearTimeout(retryTimers[id]);
  clearTimeout(zeroTimers[id]);
  zeroTimers[id] = null;
  clearTimeout(fallbackTimers[id]);
  counts[id] = 0;
  loadState[id] = 'loading';
  firstShown[id] = false;
}

function setAppEnabled(id, on) {
  if (!A(id) || isEnabled(id) === on) return;
  if (!on && enabledApps().length <= 1) return; // keep at least one app
  settings.set('apps', { ...settings.get('apps'), [id]: on });
  asleep[id] = false;
  if (on) {
    createView(id);
  } else {
    destroyView(id);
    toasts.dismissApp(id);
    if (settings.get('active') === id) settings.set('active', enabledApps()[0]);
  }
  log('app', id, on ? 'on' : 'off', enabledApps());
  if (tabShown) placeTab(tabCenterY); // the tab grows / shrinks with the number of apps
  layoutViews();
  broadcastState();
  updateGlow(false);
}

// Failures surface through 'did-fail-load' (error screen + auto retry); the promise is just noise.
function loadHome(id) {
  if (views[id]) views[id].webContents.loadURL(A(id).home).catch(() => {});
}

function setLoad(id, value) {
  if (loadState[id] === value) return;
  loadState[id] = value;
  if (value !== 'error') clearTimeout(retryTimers[id]);
  layoutViews();
  broadcastState();
}

function reloadApp(id, hard = false) {
  const wc = views[id] && views[id].webContents;
  if (!wc) return;
  if (loadState[id] === 'error') loadHome(id);
  else if (hard) wc.reloadIgnoringCache();
  else wc.reload();
}

function goHome(id) {
  if (!views[id]) return;
  const wc = views[id].webContents;
  try {
    if (new URL(wc.getURL()).pathname.startsWith(A(id).homePath)) return;
  } catch {
    // no URL yet
  }
  loadHome(id);
}

function isLinkShim(url) {
  try {
    const u = new URL(url);
    return /^(l|lm)\.(facebook|instagram|messenger)\.com$/i.test(u.hostname) || u.pathname === '/l.php';
  } catch {
    return false;
  }
}

function keepInside(id, url) {
  return apps.owns(id, url) && !isLinkShim(url);
}

function isCallUrl(id, url) {
  try {
    return apps.owns(id, url) && /(^|\/)(videocall|groupcall|call|calls|rtc)(\/|$)/i.test(new URL(url).pathname);
  } catch {
    return false;
  }
}

function openExternal(url) {
  let target = url;
  if (isLinkShim(url)) {
    // l.facebook.com/l.php?u=<real link> -> open the real link directly
    try {
      const inner = new URL(url).searchParams.get('u');
      if (inner && /^https?:/i.test(inner)) target = inner;
    } catch {
      // keep the shim URL
    }
  }
  if (/^(https?:|mailto:)/i.test(target)) shell.openExternal(target).catch(() => {});
}

function onWindowOpen(id, { url }) {
  if (url === 'about:blank' || isCallUrl(id, url) || apps.isAuthPopup(id, url)) {
    // Voice / video calls and "Sign in with Google/Apple" need their own small window.
    return {
      action: 'allow',
      overrideBrowserWindowOptions: {
        width: 1024, height: 720, autoHideMenuBar: true, backgroundColor: '#000000',
        icon: path.join(ASSETS, 'icon.ico'), title: `${A(id).name} — ChatDock`,
      },
    };
  }
  openExternal(url);
  return { action: 'deny' };
}

function guardPopup(id, child) {
  childWindows[id] += 1; // a call is going on: the app mustn't fall asleep under it
  child.on('closed', () => { childWindows[id] = Math.max(0, childWindows[id] - 1); });
  child.setMenuBarVisibility(false);
  child.webContents.setWebRTCIPHandlingPolicy('default_public_interface_only'); // call windows too
  child.webContents.setWindowOpenHandler((details) => onWindowOpen(id, details));
  child.webContents.on('did-start-navigation', (details) => {
    const { url } = details;
    if (!details.isMainFrame || url === 'about:blank' || keepInside(id, url) || apps.isAuthPopup(id, url)) return;
    openExternal(details.url);
    if (!child.isDestroyed()) child.close();
  });
}

// ---------------------------------------------------------------------------
// Unread counts (read from the page title, e.g. "(3) Instagram")
// ---------------------------------------------------------------------------
// Titles like "Somchai sent you a message" / "สมชาย ส่งข้อความถึงคุณ" (not the site's normal title)
const MESSAGE_WORDS = /messag|sent|wrote|replied|mention|ส่ง|ข้อความ|ทัก|ตอบกลับ|กล่าวถึง/i;

function onTitle(id, title) {
  const m = /^\s*\((\d+)\+?\)/.exec(title || '');
  const n = m ? parseInt(m[1], 10) : 0;
  if (!m) {
    const text = (title || '').trim();
    if (text && !A(id).plainTitle.test(text) && MESSAGE_WORDS.test(text)) lastFlash[id] = { text, at: Date.now() };
  }
  if (n > 0) {
    clearTimeout(zeroTimers[id]);
    zeroTimers[id] = null;
    setCount(id, n);
  } else if (counts[id] > 0 && !zeroTimers[id]) {
    // Titles flash ("Name sent you a message" <-> "(1) Facebook"), so only trust a zero that sticks.
    zeroTimers[id] = setTimeout(() => {
      zeroTimers[id] = null;
      setCount(id, 0);
    }, 3000);
  }
}

function setCount(id, n) {
  const before = counts[id];
  if (before === n) return;
  counts[id] = n;
  log('unread', id, before, '->', n);
  broadcastState();
  updateGlow(n > before && appPref(id, 'badge'));
  if (n > before) scheduleCountPopup(id);
  else if (n === 0) toasts.dismissApp(id); // read elsewhere (e.g. on the phone)
}

// Unread count as shown on the tab, header, tray and glow (0 when the app's count is switched off)
function shownCount(id) {
  return appPref(id, 'badge') ? counts[id] : 0;
}

function totalUnread() {
  return enabledApps().reduce((sum, id) => sum + shownCount(id), 0);
}

function preferredApp() {
  const withUnread = enabledApps().filter((id) => shownCount(id) > 0);
  return withUnread.length === 1 ? withUnread[0] : settings.get('active');
}

// ---------------------------------------------------------------------------
// Per-app switches (settings.appPrefs) and the RAM saver
// ---------------------------------------------------------------------------
function appPref(id, key) {
  const own = (settings.get('appPrefs') || {})[id];
  return own && typeof own[key] === 'boolean' ? own[key] : settings.APP_PREFS[key];
}

function setAppPref(id, key, value) {
  if (!A(id) || !Object.hasOwn(settings.APP_PREFS, key) || typeof value !== 'boolean') return false;
  const all = settings.get('appPrefs') || {};
  settings.set('appPrefs', { ...all, [id]: { ...settings.APP_PREFS, ...(all[id] || {}), [key]: value } });
  if (key === 'popups' && !value) toasts.dismissApp(id);
  if (key === 'sound') applyAudio(id);
  lastUsed[id] = Date.now(); // a changed setting starts the idle clock over
  if (asleep[id] && !sleepEligible(id)) wakeApp(id); // it has notifications to deliver again
  log('app setting', id, key, value);
  broadcastState();
  updateGlow(false);
  return true;
}

function applyAudio(id) {
  const view = views[id];
  if (view && !view.webContents.isDestroyed()) view.webContents.setAudioMuted(!!settings.get('muted') || !appPref(id, 'sound'));
}

// An app sleeps (its page is unloaded, which frees its RAM) when the user asked for it, or when
// it couldn't alert them anyway: pop-ups and unread count both switched off.
function sleepEligible(id) {
  return appPref(id, 'sleep') || (!appPref(id, 'popups') && !appPref(id, 'badge'));
}

function sleepApp(id) {
  if (asleep[id] || !views[id]) return;
  destroyView(id);
  asleep[id] = true;
  toasts.dismissApp(id);
  log('app sleeping', id);
  broadcastState();
  updateGlow(false);
}

function wakeApp(id) {
  if (!asleep[id] || !isEnabled(id)) return;
  asleep[id] = false;
  lastUsed[id] = Date.now();
  createView(id);
  log('app awake', id);
  broadcastState();
}

function sleepCheck() {
  const now = Date.now();
  const showing = panelState === 'open' || panelState === 'opening';
  for (const id of enabledApps()) {
    const view = views[id];
    if (asleep[id] || !view || !sleepEligible(id)) continue;
    if ((showing && settings.get('active') === id) || childWindows[id] > 0 || view.webContents.isCurrentlyAudible()) {
      lastUsed[id] = now; // on screen, in a call, or playing sound
      continue;
    }
    if (now - lastUsed[id] >= SLEEP_AFTER_MS) sleepApp(id);
  }
}

// RAM in MB: all of ChatDock, and each awake app's own processes (its page and the frames in it)
function memoryStats() {
  const byPid = new Map();
  let total = 0;
  for (const m of app.getAppMetrics()) {
    const mb = (m.memory.privateBytes || m.memory.workingSetSize || 0) / 1024;
    byPid.set(m.pid, mb);
    total += mb;
  }
  const perApp = {};
  for (const [id, view] of Object.entries(views)) {
    const pids = new Set();
    try {
      pids.add(view.webContents.getOSProcessId());
      for (const f of view.webContents.mainFrame.framesInSubtree) pids.add(f.osProcessId);
    } catch {
      // page is (re)starting
    }
    perApp[id] = Math.round([...pids].reduce((sum, pid) => sum + (byPid.get(pid) || 0), 0));
  }
  return { total: Math.round(total), perApp, processes: byPid.size };
}

// ---------------------------------------------------------------------------
// Message pop-ups
// ---------------------------------------------------------------------------
const cleanText = (value, max) => {
  const s = String(value ?? '').replace(/\s+/g, ' ').trim();
  return s.length > max ? `${s.slice(0, max - 1)}…` : s;
};

function safeIcon(url) {
  const s = String(url || '');
  if (s.length > 200000) return '';
  return /^https:\/\//i.test(s) || /^data:image\/(png|jpe?g|gif|webp);/i.test(s) ? s : '';
}

// Is the user already looking at this app's chat?
function appOnScreen(id) {
  return panelState === 'open' && !settingsMode && settings.get('active') === id && panelWin.isFocused();
}

// Do not disturb: 0 = off, -1 = until switched off, else until that time (epoch ms)
function dndActive() {
  const until = settings.get('dndUntil');
  return until === -1 || (until > 0 && until > Date.now());
}

function setDnd(minutes) {
  const until = minutes === -1 ? -1 : minutes > 0 ? Date.now() + minutes * 60000 : 0;
  settings.set('dndUntil', until);
  if (until) toasts.dismissAll();
  scheduleDndEnd();
  broadcastState();
  log('do not disturb', until === -1 ? 'on' : until ? `until ${new Date(until).toISOString()}` : 'off');
}

function scheduleDndEnd() {
  clearTimeout(dndTimer);
  dndTimer = null;
  const until = settings.get('dndUntil');
  if (!(until > 0)) return;
  const end = () => {
    settings.set('dndUntil', 0);
    broadcastState();
    log('do not disturb over');
  };
  const ms = until - Date.now();
  if (ms <= 0) end();
  else dndTimer = setTimeout(end, Math.min(ms, 2 ** 31 - 1));
}

// Every "should this pop up?" rule the user can set, except "already reading that chat".
function popupAllowed(id) {
  if (!settings.get('popups') || dndActive()) return false;
  if (id && !appPref(id, 'popups')) return false;
  if (settings.get('popupQuietFullscreen') && win32.isFullscreenAppActive()) return false;
  return true;
}

function popup(id, fields) {
  const a = A(id);
  toasts.push({
    appId: id,
    appName: a.name,
    iconName: a.icon,
    accent: a.colors[a.colors.length - 1],
    meta: t('toast.justMessaged'),
    hint: t('toast.clickToOpen'),
    chime: !!settings.get('popupSound') && appPref(id, 'chime'),
    ...fields,
  });
}

// A site raised a web notification (see preload-site.js): who wrote, what, and their picture.
function onSiteNotification(id, n) {
  if (n.closed) {
    toasts.dismissSource(id, n.id);
    return;
  }
  lastContentAt[id] = Date.now();
  log('site notification', id, { title: String(n.title || '').length, body: String(n.body || '').length });
  if (!popupAllowed(id) || appOnScreen(id)) return;
  const showText = !!settings.get('popupText') && appPref(id, 'preview');
  popup(id, {
    title: cleanText(n.title, 90) || A(id).name,
    body: showText ? cleanText(n.body, 300) : t('toast.sentYou'),
    icon: settings.get('popupAvatar') ? safeIcon(n.icon) : '',
    tag: n.tag ? `${id}:${cleanText(n.tag, 80)}` : '',
    sourceId: Number.isInteger(n.id) && !n.sw ? n.id : 0,
  });
}

// The unread count went up. If the site didn't tell us who wrote, still pop something up.
function scheduleCountPopup(id) {
  clearTimeout(fallbackTimers[id]);
  fallbackTimers[id] = setTimeout(() => {
    if (!popupAllowed(id) || appOnScreen(id) || counts[id] <= 0) return;
    if (Date.now() - (lastContentAt[id] || 0) < 8000) return; // already shown with name + text
    if (Date.now() - (loadStartedAt[id] || 0) < COUNT_POPUP_GRACE_MS) return; // page just (re)loaded
    const flash = lastFlash[id] && Date.now() - lastFlash[id].at < 15000 ? cleanText(lastFlash[id].text, 120) : '';
    popup(id, {
      title: flash || t('toast.newMessage'),
      body: t('toast.unread', { n: counts[id] }),
      icon: '',
      tag: `${id}:count`, // one "new messages" card per app, updated in place
      sourceId: 0,
    });
  }, 2500);
}

// A pop-up was clicked: open the chat on that app and let the site open that conversation.
function openFromToast(it, foreground) {
  toastForeground = foreground;
  if (it.action === 'update' || it.action === 'whatsnew') {
    openSettings('updates', 'toast');
    return;
  }
  const id = it.appId;
  if (!isEnabled(id)) return;
  if (it.sourceId && views[id]) views[id].webContents.send('site:notification-click', it.sourceId);
  if (settingsMode) {
    settingsMode = false;
    layoutViews();
  }
  openPanel(id, 'toast');
}

function testPopup() {
  const id = settings.get('active');
  const avatar = settings.get('popupAvatar')
    ? `data:image/png;base64,${fs.readFileSync(path.join(ASSETS, 'icon.png')).toString('base64')}` : '';
  popup(id, {
    title: t('toast.testTitle'),
    body: settings.get('popupText') ? t('toast.testBody') : t('toast.sentYou'),
    icon: avatar,
    tag: 'test',
    sourceId: 0,
  });
}

// ---------------------------------------------------------------------------
// Installing an update, and saying so afterwards
// ---------------------------------------------------------------------------
// "Updating ChatDock" window, shown for a moment before ChatDock quits for the installer.
function showUpdateWindow(s) {
  const SHOW_MS = 1800;
  return new Promise((resolve) => {
    const wa = targetDisplay().workArea;
    const w = 452;
    const h = 196;
    updateWin = new BrowserWindow({
      width: w, height: h, x: Math.round(wa.x + (wa.width - w) / 2), y: Math.round(wa.y + (wa.height - h) / 2),
      show: false, frame: false, transparent: true, resizable: false, minimizable: false, maximizable: false,
      fullscreenable: false, skipTaskbar: true, alwaysOnTop: true, hasShadow: false, thickFrame: false,
      title: 'ChatDock', webPreferences: uiWebPreferences(),
    });
    updateWin.setAlwaysOnTop(true, 'screen-saver');
    lockDown(updateWin.webContents);
    let done = false;
    const finish = () => {
      if (done) return;
      done = true;
      resolve();
    };
    updateWin.webContents.once('did-finish-load', () => {
      updateWin.webContents.send('update:show', {
        lang: uiLang(), from: buildName(app.getVersion()), to: buildName(s.version), ms: SHOW_MS,
      });
      updateWin.show();
      setTimeout(finish, SHOW_MS + 200);
    });
    setTimeout(finish, SHOW_MS + 2500); // never hold the update up
    updateWin.loadURL(uiUrl('update.html'));
  });
}

// Did this start come right after an update? Returns the version we came from (or 'older').
function detectUpdate() {
  const now = app.getVersion();
  const pending = settings.get('pendingUpdate');
  const last = settings.get('lastVersion');
  settings.set('lastVersion', now);
  if (pending) settings.set('pendingUpdate', null);
  let from = null;
  if (pending && pending.to === now) from = pending.from;
  else if (last && last !== now) from = last;
  else if (!last && settings.get('onboarded')) from = 'older'; // builds before 3 didn't record their version
  if (!from) return null;
  settings.set('whatsNew', { version: now, from, notes: pending && pending.to === now ? pending.notes : '', at: Date.now() });
  log('updated', from, '->', now);
  return from;
}

// What's new in the version running now: our own translated list, else the release notes.
function whatsNewState() {
  const now = app.getVersion();
  const n = i18n.build(now);
  const key = `whatsnew.b${n}`;
  const local = n ? t(key) : key;
  const w = settings.get('whatsNew');
  const mine = w && w.version === now;
  const text = local !== key ? local : mine ? w.notes : '';
  if (!text) return null;
  return {
    title: t('upd.notesFor', { version: buildName(now) }),
    text,
    justUpdated: !!(mine && Date.now() - (w.at || 0) < 24 * 60 * 60 * 1000),
  };
}

// First start after an update: say so, and offer what's new.
function announceUpdated() {
  const version = buildName(app.getVersion());
  const news = whatsNewState();
  toasts.push({
    appId: 'chatdock',
    appName: 'ChatDock',
    iconName: 'logo',
    accent: '#22c55e',
    meta: t('toast.updated'),
    title: t('toast.updatedTitle', { version }),
    body: news ? news.text.split('\n')[0].replace(/^•\s*/, '') : t('toast.updatedBody'),
    hint: news ? t('toast.updatedBody') : '',
    icon: '',
    tag: 'chatdock:updated',
    sourceId: 0,
    action: 'whatsnew',
    chime: false,
  });
}

// A new version is downloaded (or found, when downloading is left to the user): one pop-up per
// version. The update button in the panel header stays until it is installed.
function announceUpdate(s) {
  if (!s.version || updateAnnounced === s.version) return;
  updateAnnounced = s.version;
  broadcastState();
  if (!popupAllowed(null)) return;
  const ready = s.status === 'ready';
  toasts.push({
    appId: 'chatdock',
    appName: 'ChatDock',
    iconName: 'logo',
    accent: '#8b5cf6',
    meta: t('toast.update'),
    title: t(ready ? 'toast.updReadyTitle' : 'toast.updAvailTitle', { version: buildName(s.version) }),
    body: t(ready ? 'toast.updReadyBody' : 'toast.updAvailBody'),
    hint: t('toast.updHint'),
    chime: !!settings.get('popupSound'),
    icon: '',
    tag: 'chatdock:update',
    sourceId: 0,
    action: 'update',
  });
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------
const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));

function targetDisplay() {
  const id = settings.get('displayId');
  if (id != null) {
    const d = screen.getAllDisplays().find((x) => x.id === id);
    if (d) return d;
  }
  return screen.getPrimaryDisplay();
}

// Which screen edge the dock lives on (settings: 'right' | 'left')
const onLeft = () => settings.get('side') === 'left';

// Another monitor right against the dock edge? Then the panel fades in and out instead of
// sliding across into that monitor.
function hasNeighbourOnDockSide(d) {
  const left = onLeft();
  const edge = left ? d.bounds.x : d.bounds.x + d.bounds.width;
  return screen.getAllDisplays().some((o) => o.id !== d.id
    && Math.abs((left ? o.bounds.x + o.bounds.width : o.bounds.x) - edge) <= 1
    && o.bounds.y < d.bounds.y + d.bounds.height
    && o.bounds.y + o.bounds.height > d.bounds.y);
}

function maxWidth(d) {
  return Math.max(MIN_W, Math.round(d.workArea.width * 0.7));
}

// Each app keeps its own width: whatever the user dragged it to, else the app's own need
// (Discord's sidebars), else the general default.
function appWidth(id) {
  const saved = (settings.get('widths') || {})[id];
  return saved || (A(id) && A(id).width) || settings.get('width');
}

function panelGeometry(d = targetDisplay()) {
  const wa = d.workArea;
  const width = clamp(Math.round(appWidth(settings.get('active'))), MIN_W, maxWidth(d));
  return { x: onLeft() ? wa.x : wa.x + wa.width - width, y: wa.y, width, height: wa.height };
}

// x of a panel of this size tucked away just past the dock edge
function hiddenX(g, d = targetDisplay()) {
  return onLeft() ? d.workArea.x - g.width : d.workArea.x + d.workArea.width;
}

// Switching to an app that wants a different width while the panel is open.
function fitPanelToApp() {
  if (panelState !== 'open') return;
  const g = panelGeometry();
  const b = panelWin.getBounds();
  if (b.width === g.width && b.x === g.x) return;
  panelWin.setBounds(g);
  layoutViews();
  toasts.reposition();
}

function layoutViews() {
  if (!panelWin) return;
  const [w, h] = panelWin.getContentSize();
  const top = HEADER_H + (bannerShown ? BANNER_H : 0);
  // the resize grip runs along the inner edge: left side of the panel when docked right, and vice versa
  const bounds = { x: onLeft() ? 0 : GRIP_W, y: top, width: Math.max(1, w - GRIP_W), height: Math.max(1, h - top) };
  const active = settings.get('active');
  for (const [id, view] of Object.entries(views)) {
    view.setBounds(bounds);
    view.setVisible(id === active && viewShowable(id));
  }
}

function viewShowable(id) {
  return !helpMode && !settingsMode && settings.get('onboarded') && firstShown[id] && loadState[id] !== 'error';
}

function themeBg() {
  return nativeTheme.shouldUseDarkColors ? '#18191d' : '#ffffff';
}

function onThemeUpdated() {
  const bg = themeBg();
  panelWin.setBackgroundColor(bg);
  for (const view of Object.values(views)) view.setBackgroundColor(bg);
}

function onDisplaysChanged() {
  hideTab(true);
  if (panelState === 'open' || panelState === 'opening') {
    panelWin.setBounds(panelGeometry());
    layoutViews();
  }
  updateGlow(false);
  toasts.reposition();
}

// ---------------------------------------------------------------------------
// Panel open / close
// ---------------------------------------------------------------------------
const easeOutCubic = (t) => 1 - (1 - t) ** 3;
const easeInCubic = (t) => t ** 3;

function animate(ms, ease, step, done) {
  if (anim) clearInterval(anim);
  const t0 = performance.now();
  const tick = () => {
    const t = Math.min(1, (performance.now() - t0) / ms);
    step(ease(t));
    if (t >= 1) {
      clearInterval(anim);
      anim = null;
      done();
    }
  };
  anim = setInterval(tick, 8);
  tick();
}

function isOurs(hwnd) {
  return !!hwnd && (ownHwnds.includes(hwnd) || ownHwnds.includes(win32.rootOwner(hwnd)));
}

// Compact state for the log
function snap() {
  const fg = win32.foregroundWindow();
  return {
    st: panelState,
    vis: panelWin.isVisible(),
    foc: panelWin.isFocused(),
    x: panelWin.getBounds().x,
    fg: fg === ownHwnds[0] ? 'PANEL' : win32.className(fg),
    tab: tabShown,
  };
}

function rememberForeground(source) {
  let fg = win32.foregroundWindow();
  // Clicking the tab / a pop-up activates it, so use whatever was in front before that.
  if (source === 'tab' && isOurs(fg)) fg = tabForeground;
  if (source === 'toast' && isOurs(fg)) fg = toastForeground;
  prevForeground = source === 'tray' || isOurs(fg) ? 0 : fg;
  log('foreground before open:', fg, win32.className(fg), '->', prevForeground);
}

function openPanel(appId, source = 'user') {
  log('open request', source, appId || '', snap());
  if (appId && isEnabled(appId)) setActive(appId, false);
  wakeApp(settings.get('active')); // a sleeping app loads again when it is opened
  if (panelState === 'open' || panelState === 'opening') {
    focusPanel();
    broadcastState();
    return;
  }
  // hidden, or re-opened while still sliding out (focus may already be back in the game)
  rememberForeground(source);
  blurredWhileOpening = false;
  bannerShown = !bannerDismissed && win32.isExclusiveFullscreen();
  hideTab(true);
  glowWin.hide();

  const d = targetDisplay();
  const g = panelGeometry(d);
  const slide = !hasNeighbourOnDockSide(d); // don't slide across into the next monitor
  const opacity = settings.get('opacity');
  let fromX = hiddenX(g, d);
  let fromOpacity = 0;
  if (panelWin.isVisible()) { // re-opened while it was still sliding out
    fromX = panelWin.getBounds().x;
    fromOpacity = panelWin.getOpacity();
  }
  panelWin.setBounds({ x: slide ? fromX : g.x, y: g.y, width: g.width, height: g.height });
  layoutViews();
  // setOpacity turns the window into a layered window for good, so only touch it when it matters.
  if (!slide) panelWin.setOpacity(fromOpacity);
  else if (opacity < 1 || panelWin.getOpacity() < 1) panelWin.setOpacity(opacity);
  panelState = 'opening';
  if (!panelWin.isVisible()) panelWin.show();
  raise(panelWin);
  focusPanel();
  broadcastState();
  toasts.dismissApp(settings.get('active')); // reading it now
  toasts.reposition(); // other pop-ups move beside the chat
  log('open', { source, slide, prevForeground });

  animate(OPEN_MS, easeOutCubic, (p) => {
    if (slide) panelWin.setBounds({ x: Math.round(fromX + (g.x - fromX) * p), y: g.y, width: g.width, height: g.height });
    else panelWin.setOpacity(fromOpacity + (opacity - fromOpacity) * p);
  }, () => {
    panelState = 'open';
    fitPanelToApp(); // the app or the dock side changed while it was sliding in
    if (panelWin.isFocused()) {
      // Already in front: only move the keyboard into the page. Re-activating here could undo a
      // click the user made on the game in this very moment.
      focusContent();
    } else if (blurredWhileOpening || win32.mouseButtonDown()) {
      // The user clicked back into the game while it was still sliding in: treat it like any
      // click elsewhere instead of grabbing focus back.
      log('clicked elsewhere during the slide-in');
      checkAutoHide(false);
    } else {
      focusPanel(); // Windows didn't let us take focus when it started; try once more
    }
    broadcastState();
    log('opened', snap());
  });
}

function closePanel(restoreFocus, reason = '') {
  log('close request', reason, snap());
  if (panelState === 'hidden' || panelState === 'closing') return;
  panelState = 'closing';
  const target = restoreFocus ? prevForeground : 0;
  prevForeground = 0;
  if (target) { // straight back into the game, no extra click needed
    const ok = win32.restoreForeground(target);
    log('focus back to', target, win32.className(target), ok ? 'ok' : 'FAILED', '| now', win32.className(win32.foregroundWindow()));
  }

  const d = targetDisplay();
  const b = panelWin.getBounds();
  const slide = !hasNeighbourOnDockSide(d);
  const toX = hiddenX(b, d);
  const fromOpacity = panelWin.getOpacity();
  broadcastState();
  log('close', { restoreFocus, target });

  animate(CLOSE_MS, easeInCubic, (p) => {
    if (slide) panelWin.setBounds({ x: Math.round(b.x + (toX - b.x) * p), y: b.y, width: b.width, height: b.height });
    else panelWin.setOpacity(fromOpacity * (1 - p));
  }, () => {
    panelWin.hide();
    panelState = 'hidden';
    lastUsed[settings.get('active')] = Date.now();
    stopSettingsTimer();
    if ((helpMode && settings.get('onboarded')) || settingsMode) { // next time it opens on the chats
      helpMode = helpMode && !settings.get('onboarded');
      settingsMode = false;
      layoutViews();
    }
    broadcastState();
    updateGlow(false);
    toasts.reposition();
    log('closed', snap());
  });
}

function togglePanel(source) {
  if (panelState === 'open' || panelState === 'opening') closePanel(true, source);
  else openPanel(null, source);
}

// Make the panel the foreground window (Windows may refuse; then we force it).
function activatePanel() {
  if (!panelWin.isVisible()) return;
  panelWin.focus();
  const hwnd = ownHwnds[0];
  if (win32.available() && win32.foregroundWindow() !== hwnd) {
    const ok = win32.forceForeground(hwnd);
    log('focus: forced', ok ? 'ok' : 'FAILED', '| fg now', win32.className(win32.foregroundWindow()));
  }
}

// Put the keyboard into the chat page (no change to which window is in front).
function focusContent() {
  const view = views[settings.get('active')];
  if (view && view.getVisible()) view.webContents.focus();
  else panelWin.webContents.focus();
}

function focusPanel() {
  if (!panelWin.isVisible()) return;
  activatePanel();
  focusContent();
}

function onPanelBlur() {
  log('blur', snap());
  if (DEMO) return; // the recorder runs while the PC is in use: other windows getting focus mustn't close it
  if (panelState === 'opening') {
    // Only a real mouse click counts (some games grab focus back on their own); decided when the
    // slide-in finishes.
    blurredWhileOpening = win32.mouseButtonDown();
    return;
  }
  if (panelState !== 'open') return;
  if (settings.get('pinned')) {
    setTimeout(() => { if (panelState === 'open') raise(panelWin); }, 80); // stay above a topmost game
    return;
  }
  setTimeout(() => checkAutoHide(false), 120);
}

function cursorOverPanel() {
  const pt = screen.getCursorScreenPoint();
  const b = panelWin.getBounds();
  return pt.x >= b.x && pt.x < b.x + b.width && pt.y >= b.y && pt.y < b.y + b.height;
}

function checkAutoHide(dragging) {
  if (panelState !== 'open' || settings.get('pinned') || panelWin.isFocused()) {
    log('autohide: skip', { st: panelState, pinned: settings.get('pinned'), foc: panelWin.isFocused() });
    return;
  }
  if (!panelWin.isEnabled()) return log('autohide: skip, dialog open'); // a file picker / save dialog is open
  if (isOurs(win32.foregroundWindow())) return log('autohide: skip, our window has focus'); // e.g. our own dialog or menu
  if (win32.mouseButtonDown()) {
    // Still pressing: maybe dragging a file from Explorer into the chat. Decide on release.
    setTimeout(() => checkAutoHide(true), 100);
    return;
  }
  if (dragging && cursorOverPanel()) {
    log('autohide: skip, released over the panel (drop)');
    focusPanel(); // something was dropped onto the chat — keep it open
    return;
  }
  lastAutoHideAt = Date.now();
  closePanel(false, 'clicked elsewhere'); // the user already clicked where they wanted to go
}

// Settings screen inside the panel (gear button, tray menu, update pop-up).
function openSettings(section = '', source = 'menu') {
  settingsMode = true;
  helpMode = false;
  startSettingsTimer();
  layoutViews();
  if (panelState === 'open' || panelState === 'opening') {
    activatePanel();
    panelWin.webContents.focus();
    broadcastState();
  } else {
    openPanel(null, source);
  }
  if (section) panelWin.webContents.send('settings:goto', section);
}

function closeSettings() {
  if (!settingsMode) return;
  settingsMode = false;
  stopSettingsTimer();
  layoutViews();
  focusPanel();
  broadcastState();
}

// While the settings screen is open, its RAM figures refresh every few seconds.
function startSettingsTimer() {
  clearInterval(settingsTimer);
  settingsTimer = setInterval(() => {
    if (settingsMode && panelState === 'open') broadcastState();
  }, 4000);
}

function stopSettingsTimer() {
  clearInterval(settingsTimer);
  settingsTimer = null;
}

function setActive(id, focus = true) {
  if (!isEnabled(id)) return;
  lastUsed[settings.get('active')] = Date.now(); // the app we leave was in use until now
  settings.set('active', id);
  if (panelState !== 'hidden') wakeApp(id);
  fitPanelToApp();
  layoutViews();
  if (panelState === 'open' || panelState === 'opening') toasts.dismissApp(id);
  if (focus && panelState !== 'hidden') focusPanel();
  broadcastState();
}

function otherApp() {
  const list = enabledApps();
  const i = list.indexOf(settings.get('active'));
  return list[(i + 1) % list.length];
}

// edge: where the user dragged the panel's inner edge to (screen DIP)
function resizeTo(edge) {
  if (panelState !== 'open' || typeof edge !== 'number' || !Number.isFinite(edge)) return;
  const d = targetDisplay();
  const wa = d.workArea;
  const right = wa.x + wa.width;
  const width = clamp(Math.round(onLeft() ? edge - wa.x : right - edge), MIN_W, maxWidth(d));
  if (width === panelWin.getBounds().width) return;
  panelWin.setBounds({ x: onLeft() ? wa.x : right - width, y: wa.y, width, height: wa.height });
  layoutViews();
  settings.set('widths', { ...settings.get('widths'), [settings.get('active')]: width }); // remembered per app
  toasts.reposition();
}

// The dock moved to the other screen edge. An open panel slides in again from its new side.
function setSide(side) {
  if (settings.get('side') === side) return;
  hideTab(true);
  settings.set('side', side);
  log('dock side', side);
  broadcastState(); // panel, tab and glow mirror themselves
  if (panelState === 'open') {
    const d = targetDisplay();
    const g = panelGeometry(d);
    const fromX = hiddenX(g, d);
    const slide = !hasNeighbourOnDockSide(d);
    panelWin.setBounds(slide ? { ...g, x: fromX } : g);
    layoutViews();
    if (slide) {
      animate(OPEN_MS, easeOutCubic, (p) => {
        panelWin.setBounds({ x: Math.round(fromX + (g.x - fromX) * p), y: g.y, width: g.width, height: g.height });
      }, () => toasts.reposition());
    }
  }
  updateGlow(false);
  toasts.reposition();
}

function showHelp() {
  helpMode = true;
  settingsMode = false;
  autostartCache = readOpenAtLogin();
  layoutViews();
  openPanel(null, 'menu');
  broadcastState();
}

function finishOnboarding(opts = {}) {
  const firstTime = !settings.get('onboarded');
  settings.set('onboarded', true);
  if (firstTime && typeof opts.autostart === 'boolean' && app.isPackaged) setOpenAtLogin(opts.autostart);
  helpMode = false;
  layoutViews();
  focusPanel();
  broadcastState();
  if (firstTime && tray) {
    tray.displayBalloon({
      iconType: 'info',
      noSound: true,
      title: t('balloon.hereTitle'),
      content: t(onLeft() ? 'balloon.here.left' : 'balloon.here.right'),
    });
  }
}

// ---------------------------------------------------------------------------
// Right-edge tab + unread glow
// ---------------------------------------------------------------------------
function edgeTick() {
  let next = 150;
  try {
    next = edgeStep();
  } catch (err) {
    log('edge error', err);
  }
  edgeTimer = setTimeout(edgeTick, next);
}

function edgeStep() {
  const mode = settings.get('edgeMode');
  if (DEMO) return 200; // the demo recorder drives the tab itself
  if (panelState !== 'hidden' || mode === 'off') {
    dwellStart = 0;
    return 200;
  }
  const pt = screen.getCursorScreenPoint();
  const d = targetDisplay();
  const b = d.bounds;
  const left = onLeft();
  const right = b.x + b.width;
  const now = Date.now();
  // Leave the corners alone: they hold window close buttons, menus, the Start button and the clock.
  const margin = Math.max(90, Math.round(b.height * 0.1));
  const zoneTop = b.y + margin;
  const zoneBottom = d.workArea.y + d.workArea.height - margin;
  const atEdge = left ? pt.x >= b.x && pt.x < b.x + EDGE_PX : pt.x >= right - EDGE_PX && pt.x <= right;
  const onEdge = atEdge && pt.y >= zoneTop && pt.y <= zoneBottom;
  // A game that has taken the mouse (pointer hidden, or held inside the game) pushes the pointer
  // against the screen edge whenever you aim or turn. That must never bring the tab out.
  const captured = (onEdge || tabShown) && win32.mouseCaptured();

  if (tabShown) {
    if (captured) {
      hideTab(true);
      return 60;
    }
    const fg = win32.foregroundWindow();
    if (fg !== tabLastForeground) { // e.g. a topmost game was clicked and rose above the tab
      tabLastForeground = fg;
      raise(tabWin);
    }
    const r = tabWin.getBounds();
    const insideX = left ? pt.x >= b.x && pt.x <= r.x + r.width + 30 : pt.x >= r.x - 30 && pt.x <= right;
    const inside = insideX && pt.y >= r.y - 30 && pt.y <= r.y + r.height + 30;
    if (onEdge) {
      followCursor(pt.y); // slide along the edge with the cursor
      lastInsideAt = now;
    } else if (inside) {
      lastInsideAt = now;
    } else if (now - lastInsideAt > TAB_LINGER_MS) {
      hideTab(false);
    }
    glideTab();
    return 16; // ~60 fps while the tab is out, so it glides instead of jumping
  }

  if (onEdge && !captured && !win32.mouseButtonDown() && !(mode === 'no-fullscreen' && win32.isFullscreenAppActive())) {
    if (!dwellStart) dwellStart = now;
    else if (now - dwellStart >= DWELL_MS) {
      dwellStart = 0;
      showTab(pt.y);
    }
    return 30;
  }
  dwellStart = 0;
  const dist = left ? pt.x - b.x : right - pt.x;
  const near = dist >= 0 && dist < 250 && pt.y >= b.y && pt.y <= b.y + b.height;
  return near ? 40 : 110; // poll faster only while the cursor is near the edge
}

function clampTabY(y, h) {
  const d = targetDisplay();
  return clamp(y, d.bounds.y + 4, d.workArea.y + d.workArea.height - h - 4);
}

// Put the tab centred on the cursor right away (when it appears, or when it grows / shrinks).
function placeTab(cursorY) {
  const h = tabHeight();
  tabY = clampTabY(cursorY - h / 2, h);
  tabTargetY = tabY;
  tabDrawnY = null;
  moveTab();
}

// The cursor slides along the edge: the pill is pushed along so the cursor stays inside it (with a
// margin), instead of jumping to re-centre itself on the cursor.
function followCursor(cursorY) {
  const h = tabHeight();
  const top = tabTargetY + 16 + TAB_FOLLOW_MARGIN;
  const bottom = tabTargetY + h - 16 - TAB_FOLLOW_MARGIN;
  if (cursorY < top) tabTargetY -= top - cursorY;
  else if (cursorY > bottom) tabTargetY += cursorY - bottom;
  tabTargetY = clampTabY(tabTargetY, h);
}

// One animation frame: cover part of the way to the target, so fast moves stay smooth.
function glideTab() {
  const diff = tabTargetY - tabY;
  if (Math.abs(diff) < 0.5) tabY = tabTargetY;
  else tabY += diff * TAB_GLIDE;
  moveTab();
}

function moveTab() {
  const b = targetDisplay().bounds;
  const h = tabHeight();
  const y = Math.round(tabY);
  if (y === tabDrawnY) return;
  tabDrawnY = y;
  tabWin.setBounds({ x: onLeft() ? b.x : b.x + b.width - TAB_W, y, width: TAB_W, height: h });
  tabCenterY = y + h / 2;
}

function showTab(cursorY) {
  clearTimeout(tabHideTimer);
  tabForeground = win32.foregroundWindow(); // the game / app the user is in right now
  tabLastForeground = tabForeground;
  placeTab(cursorY);
  tabWin.webContents.send('tab:show', uiState());
  tabWin.showInactive();
  raise(tabWin);
  tabShown = true;
  tabShownAt = Date.now();
  lastInsideAt = tabShownAt;
  glowWin.hide();
  log('tab show', tabWin.getBounds());
}

function hideTab(instant) {
  if (!tabShown) return;
  tabShown = false;
  log('tab hide', instant ? 'instant' : 'linger');
  tabWin.webContents.send('tab:hide', !!instant);
  clearTimeout(tabHideTimer);
  // Let the renderer paint its "tucked away" frame before the window disappears,
  // so the next show never flashes a stale frame.
  tabHideTimer = setTimeout(() => {
    if (!tabShown) tabWin.hide();
    updateGlow(false);
  }, instant ? 50 : 170);
}

function updateGlow(pulse) {
  if (!glowWin) return;
  const show = settings.get('glow') && totalUnread() > 0 && panelState === 'hidden' && !tabShown;
  if (!show) {
    if (glowWin.isVisible()) glowWin.hide();
    return;
  }
  const d = targetDisplay();
  const b = d.bounds;
  const cy = tabCenterY ?? (b.y + b.height / 2);
  const y = clamp(Math.round(cy - GLOW_H / 2), b.y, d.workArea.y + d.workArea.height - GLOW_H);
  glowWin.setBounds({ x: onLeft() ? b.x : b.x + b.width - GLOW_W, y, width: GLOW_W, height: GLOW_H });
  const colors = enabledApps().filter((id) => shownCount(id) > 0).flatMap((id) => A(id).colors);
  glowWin.webContents.send('glow:state', { colors, pulse: !!pulse, side: settings.get('side') });
  if (!glowWin.isVisible()) glowWin.showInactive();
  raise(glowWin);
}

// ---------------------------------------------------------------------------
// Keyboard
// ---------------------------------------------------------------------------
function hotkeyLabel() {
  const acc = settings.get('hotkey');
  if (!acc) return '';
  const preset = HOTKEYS.find((h) => h.acc === acc);
  const label = preset ? preset.label : acc.replace(/Control/g, 'Ctrl').split('+').join(' + ');
  return label.replace(/Ctrl/g, t('key.ctrl'));
}

function registerHotkey() {
  globalShortcut.unregisterAll();
  const acc = settings.get('hotkey');
  hotkeyOk = false;
  if (!acc) return;
  try {
    hotkeyOk = globalShortcut.register(acc, onHotkey);
  } catch (err) {
    log('hotkey error', err);
  }
  if (!hotkeyOk && tray && !SELFTEST && !DEMO) {
    tray.displayBalloon({
      iconType: 'warning',
      title: t('balloon.hotkeyTitle'),
      content: t('balloon.hotkeyBody', { hotkey: hotkeyLabel() }),
    });
  }
  log('hotkey', acc, hotkeyOk ? 'registered' : 'FAILED');
}

function onHotkey() {
  log('hotkey', snap());
  if (panelState === 'open' && !panelWin.isFocused()) {
    // pinned panel sitting behind the game -> bring it forward for typing
    rememberForeground('hotkey');
    focusPanel();
    return;
  }
  togglePanel('hotkey');
}

function onPanelKey(event, input, where = '') {
  if (input.type !== 'keyDown') return;
  if (DEBUG) log('key', where, input.code || '(no code)', input.key, { ctrl: input.control, alt: input.alt });
  const ctrl = input.control || input.meta;
  const active = settings.get('active');
  // Match the physical key (works with the Thai layout) or, when a keyboard tool sends no scan code
  // and `code` is empty, the key name (Latin layouts).
  const key = (input.key || '').toLowerCase();
  const is = (code, ...names) => input.code === code || (!input.code && names.includes(key));
  const code = input.code;

  if ((code === 'Escape' || input.key === 'Escape') && !ctrl && !input.alt && !input.shift) {
    // one Esc still reaches the page (closes its pop-ups); two quick ones hide the panel
    const now = Date.now();
    if (now - lastEscAt < 450) {
      lastEscAt = 0;
      event.preventDefault();
      closePanel(true, 'esc esc');
    } else {
      lastEscAt = now;
      if (settingsMode && where === 'host') closeSettings(); // one Esc leaves the settings screen
    }
    return;
  }
  lastEscAt = 0;

  // Ctrl+1..9 -> the n-th switched-on app
  const digit = /^(?:Digit|Numpad)([1-9])$/.exec(code || '') || (!code && /^([1-9])$/.exec(key));
  if (ctrl && !input.alt && !input.shift && digit) {
    const target = enabledApps()[Number(digit[1]) - 1];
    if (target) {
      event.preventDefault();
      setActive(target);
    }
  } else if (ctrl && (code === 'Tab' || input.key === 'Tab')) {
    event.preventDefault();
    setActive(otherApp());
  } else if ((ctrl && is('KeyR', 'r')) || code === 'F5' || input.key === 'F5') {
    event.preventDefault();
    reloadApp(active, input.shift);
  } else if (ctrl && !input.alt && is('KeyW', 'w')) {
    event.preventDefault();
    closePanel(true, 'ctrl+w');
  } else if (ctrl && (is('Equal', '=', '+') || code === 'NumpadAdd')) {
    event.preventDefault();
    zoomStep(active, 1);
  } else if (ctrl && (is('Minus', '-') || code === 'NumpadSubtract')) {
    event.preventDefault();
    zoomStep(active, -1);
  } else if (ctrl && (is('Digit0', '0') || code === 'Numpad0')) {
    event.preventDefault();
    zoomStep(active, 0);
  } else if (input.alt && !ctrl && (code === 'ArrowLeft' || input.key === 'ArrowLeft')) {
    event.preventDefault();
    const h = views[active] && views[active].webContents.navigationHistory;
    if (h && h.canGoBack()) h.goBack();
  } else if (input.alt && !ctrl && (code === 'ArrowRight' || input.key === 'ArrowRight')) {
    event.preventDefault();
    const h = views[active] && views[active].webContents.navigationHistory;
    if (h && h.canGoForward()) h.goForward();
  } else if (DEBUG && ctrl && input.shift && is('KeyI', 'i')) {
    event.preventDefault();
    if (views[active]) views[active].webContents.openDevTools({ mode: 'detach' });
  }
}

// ---------------------------------------------------------------------------
// Zoom
// ---------------------------------------------------------------------------
function zoomOf(id) {
  return (settings.get('zoom') || {})[id] || 1;
}

function applyZoom(id) {
  const view = views[id];
  if (view && !view.webContents.isDestroyed()) view.webContents.setZoomFactor(zoomOf(id));
}

function zoomStep(id, dir) {
  const current = zoomOf(id);
  let next = 1;
  if (dir !== 0) {
    let idx = ZOOM_STEPS.findIndex((z) => Math.abs(z - current) < 0.001);
    if (idx < 0) idx = ZOOM_STEPS.indexOf(1);
    next = ZOOM_STEPS[clamp(idx + dir, 0, ZOOM_STEPS.length - 1)];
  }
  settings.set('zoom', { ...settings.get('zoom'), [id]: next });
  applyZoom(id);
  broadcastState();
}

// ---------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------
function showContextMenu(wc, p) {
  const items = [];
  const sep = () => { if (items.length && items[items.length - 1].type !== 'separator') items.push({ type: 'separator' }); };
  if (p.isEditable) {
    items.push(
      { label: t('ctx.cut'), accelerator: 'CmdOrCtrl+X', enabled: p.editFlags.canCut, click: () => wc.cut() },
      { label: t('ctx.copy'), accelerator: 'CmdOrCtrl+C', enabled: p.editFlags.canCopy, click: () => wc.copy() },
      { label: t('ctx.paste'), accelerator: 'CmdOrCtrl+V', enabled: p.editFlags.canPaste, click: () => wc.paste() },
      { label: t('ctx.selectAll'), accelerator: 'CmdOrCtrl+A', click: () => wc.selectAll() },
    );
  } else if (p.selectionText && p.selectionText.trim()) {
    items.push({ label: t('ctx.copy'), accelerator: 'CmdOrCtrl+C', click: () => wc.copy() });
  }
  if (p.linkURL && /^https?:/i.test(p.linkURL)) {
    sep();
    items.push(
      { label: t('ctx.openLink'), click: () => openExternal(p.linkURL) },
      { label: t('ctx.copyLink'), click: () => clipboard.writeText(p.linkURL) },
    );
  }
  if (p.mediaType === 'image' && p.srcURL) {
    sep();
    items.push(
      { label: t('ctx.copyImage'), click: () => wc.copyImageAt(p.x, p.y) },
      { label: t('ctx.saveImage'), click: () => wc.downloadURL(p.srcURL) },
    );
  }
  sep();
  items.push(
    { label: t('ctx.back'), accelerator: 'Alt+Left', enabled: wc.navigationHistory.canGoBack(), click: () => wc.navigationHistory.goBack() },
    { label: t('ctx.reload'), accelerator: 'CmdOrCtrl+R', click: () => wc.reload() },
  );
  Menu.buildFromTemplate(items).popup({ window: panelWin });
}

// Tray right-click menu: the everyday switches. Everything else lives on the settings screen.
function buildMenu() {
  const radio = (label, checked, click) => ({ label, type: 'radio', checked, click });
  const hotkey = hotkeyOk ? settings.get('hotkey') : '';
  const up = updater.getState();
  const until = settings.get('dndUntil');
  const dnd = dndActive();

  return Menu.buildFromTemplate([
    ...(up.status === 'ready' ? [
      { label: t('tray.updateNow', { version: buildName(up.version) }), click: () => updater.install() },
      { type: 'separator' },
    ] : []),
    {
      label: t(panelState === 'hidden' ? 'tray.open' : 'tray.hide'),
      ...(hotkey ? { accelerator: hotkey, registerAccelerator: false } : {}),
      click: () => togglePanel('menu'),
    },
    ...enabledApps().map((id) => ({
      label: shownCount(id) > 0 ? `${A(id).name}  (${shownCount(id)})` : A(id).name,
      click: () => {
        if (settingsMode) closeSettings();
        openPanel(id, 'menu');
      },
    })),
    { type: 'separator' },
    { label: t('tray.popups'), type: 'checkbox', checked: !!settings.get('popups'), click: (mi) => setPref('popups', mi.checked) },
    {
      label: !dnd ? t('tray.dnd') : until === -1 ? t('tray.dndOn') : t('tray.dndUntil', { time: clockTime(until) }),
      submenu: [
        radio(t('dnd.off'), !dnd, () => setDnd(0)),
        radio(t('dnd.30'), false, () => setDnd(30)),
        radio(t('dnd.60'), false, () => setDnd(60)),
        radio(t('dnd.120'), false, () => setDnd(120)),
        radio(t('dnd.480'), false, () => setDnd(480)),
        radio(t('dnd.forever'), until === -1, () => setDnd(-1)),
      ],
    },
    { label: t('tray.pin'), type: 'checkbox', checked: !!settings.get('pinned'), click: (mi) => setPref('pinned', mi.checked) },
    { type: 'separator' },
    { label: t('tray.settings'), click: () => openSettings('', 'menu') },
    { label: t('tray.help'), click: () => showHelp() },
    { label: t('tray.quit'), click: () => quit() },
  ]);
}

function clockTime(ms) {
  const t = new Date(ms);
  return `${String(t.getHours()).padStart(2, '0')}:${String(t.getMinutes()).padStart(2, '0')}`;
}

// ---------------------------------------------------------------------------
// Settings: every value the settings screen may change, checked before it is used
// ---------------------------------------------------------------------------
const PREFS = {
  lang: { values: ['auto', ...i18n.IDS], apply: setLanguage },
  side: { values: ['right', 'left'], apply: setSide },
  edgeMode: { values: ['always', 'no-fullscreen', 'off'], apply: (v) => { settings.set('edgeMode', v); if (v === 'off') hideTab(true); } },
  displayId: { check: (v) => screen.getAllDisplays().some((d) => d.id === v), apply: setDisplay },
  glow: { type: 'boolean', apply: (v) => { settings.set('glow', v); updateGlow(false); } },
  theme: { values: ['system', 'dark', 'light'], apply: setTheme },
  opacity: { range: [0.6, 1], apply: setOpacity },
  pinned: { type: 'boolean' },
  muted: { type: 'boolean', apply: setMuted },
  hotkey: { values: ['', ...HOTKEYS.map((h) => h.acc)], apply: setHotkey },
  autostart: { type: 'boolean', apply: setOpenAtLogin },
  hideFromCapture: { type: 'boolean', apply: setHideFromCapture },
  popups: { type: 'boolean', apply: (v) => { settings.set('popups', v); if (!v) toasts.dismissAll(); } },
  popupText: { type: 'boolean' },
  popupAvatar: { type: 'boolean' },
  popupSound: { type: 'boolean' },
  popupPosition: { values: ['top-right', 'bottom-right', 'top-left', 'bottom-left'], apply: (v) => { settings.set('popupPosition', v); toasts.refresh(); } },
  popupDuration: { values: [0, 5, 8, 12, 20, 30] },
  popupMax: { values: [1, 2, 3, 4, 5], apply: (v) => { settings.set('popupMax', v); toasts.refresh(); } },
  popupQuietFullscreen: { type: 'boolean' },
  updateAutoCheck: { type: 'boolean', apply: (v) => { settings.set('updateAutoCheck', v); updater.setAutoCheck(); } },
  updateAutoDownload: { type: 'boolean', apply: (v) => { settings.set('updateAutoDownload', v); updater.setAutoDownload(v); } },
};

function setPref(key, value) {
  const spec = Object.hasOwn(PREFS, key) ? PREFS[key] : null;
  let ok = false;
  if (spec && spec.type === 'boolean') ok = typeof value === 'boolean';
  else if (spec && spec.values) ok = spec.values.includes(value);
  else if (spec && spec.range) ok = typeof value === 'number' && value >= spec.range[0] && value <= spec.range[1];
  else if (spec && spec.check) ok = spec.check(value);
  if (!ok) {
    log('setting rejected', String(key).slice(0, 40));
    return false;
  }
  if (spec.apply) spec.apply(value);
  else settings.set(key, value);
  broadcastState();
  return true;
}

function setDisplay(id) {
  settings.set('displayId', id === screen.getPrimaryDisplay().id ? null : id);
  onDisplaysChanged();
}

function setLanguage(lang) {
  settings.set('lang', lang);
  applyLanguage();
  toasts.refresh();
  log('language', lang, '->', uiLang());
}

function settingsAction(name, arg) {
  switch (name) {
    case 'close': return closeSettings();
    case 'test-popup': return testPopup();
    case 'dnd': return [0, 30, 60, 120, 480, -1].includes(arg) ? setDnd(arg) : undefined;
    case 'clear-app': return A(arg) ? clearAppData(arg) : undefined;
    case 'clear-all': return Promise.all(apps.ALL_IDS.map(clearAppData));
    case 'reset-widths':
      settings.set('widths', {});
      return fitPanelToApp();
    case 'check-update': return updater.check();
    case 'download-update': return updater.download();
    case 'install-update': return updater.install();
    case 'open-releases': return shell.openExternal(`${REPO_URL}/releases`);
    case 'open-repo': return shell.openExternal(REPO_URL);
    case 'open-license': return shell.openExternal(`${REPO_URL}/blob/main/LICENSE`);
    case 'open-logs': return shell.showItemInFolder(logger.path());
    case 'help': return showHelp();
    case 'quit': return quit();
    default: return log('unknown settings action', String(name).slice(0, 40));
  }
}

function setHotkey(acc) {
  settings.set('hotkey', acc);
  registerHotkey();
  broadcastState();
}

function setOpacity(v) {
  settings.set('opacity', v);
  if (panelState === 'open') panelWin.setOpacity(v);
}

function setTheme(theme) {
  settings.set('theme', theme);
  nativeTheme.themeSource = theme;
}

function setMuted(on) {
  settings.set('muted', on);
  for (const id of Object.keys(views)) applyAudio(id);
}

function setHideFromCapture(on) {
  settings.set('hideFromCapture', on);
  panelWin.setContentProtection(on);
  toasts.setContentProtection(on);
}

// ---------------------------------------------------------------------------
// Tray
// ---------------------------------------------------------------------------
function createTray() {
  tray = new Tray(path.join(ASSETS, 'tray.ico'));
  tray.on('click', () => {
    log('tray click', { sinceAutoHide: Date.now() - lastAutoHideAt }, snap());
    if (Date.now() - lastAutoHideAt < 600) return; // the click that just hid the panel
    togglePanel('tray');
  });
  tray.on('right-click', () => tray.popUpContextMenu(buildMenu()));
  updateTray();
}

function updateTray() {
  if (!tray) return;
  const unread = totalUnread() > 0;
  if (unread !== trayUnread) {
    trayUnread = unread;
    tray.setImage(path.join(ASSETS, unread ? 'tray-unread.ico' : 'tray.ico'));
  }
  const parts = enabledApps().filter((id) => shownCount(id) > 0).map((id) => `${A(id).name} ${shownCount(id)}`);
  const hk = hotkeyLabel();
  const until = settings.get('dndUntil');
  const up = updater.getState();
  tray.setToolTip([
    'ChatDock',
    parts.length ? t('tip.new', { list: parts.join(' · ') }) : t('tip.noNew'),
    !dndActive() ? '' : until === -1 ? t('tip.dnd') : t('tip.dndUntil', { time: clockTime(until) }),
    up.status === 'ready' ? t('tip.update', { version: buildName(up.version) }) : '',
    hk && hotkeyOk ? t('tip.hotkey', { hotkey: hk }) : '',
  ].filter(Boolean).join('\n').slice(0, 127)); // Windows cuts tooltips at 127 characters
}

// ---------------------------------------------------------------------------
// UI state + IPC
// ---------------------------------------------------------------------------
function uiState() {
  const active = settings.get('active');
  const up = updater.getState();
  return {
    apps: enabledApps().map((id) => ({ id, name: A(id).name, icon: A(id).icon, asleep: asleep[id] })),
    active,
    counts: Object.fromEntries(apps.ALL_IDS.map((id) => [id, shownCount(id)])),
    lang: uiLang(),
    langPref: settings.get('lang'), // 'auto' or a language
    load: { ...loadState },
    firstShown: { ...firstShown },
    pinned: !!settings.get('pinned'),
    help: helpMode,
    settingsOpen: settingsMode,
    onboarded: !!settings.get('onboarded'),
    banner: bannerShown,
    hotkey: hotkeyOk ? hotkeyLabel() : '',
    zoom: zoomOf(active),
    canAutostart: canAutostart(),
    autostart: autostartCache,
    panel: panelState,
    side: settings.get('side'),
    dnd: dndActive(),
    update: { status: up.status, version: up.version, percent: up.percent, build: i18n.build(up.version), name: buildName(up.version) },
  };
}

// Everything the settings screen shows (only sent to the panel).
function settingsState() {
  const primary = screen.getPrimaryDisplay();
  const current = targetDisplay();
  const keys = Object.keys(PREFS).filter((k) => k !== 'autostart' && k !== 'displayId');
  const memory = memoryStats();
  return {
    prefs: {
      ...Object.fromEntries(keys.map((k) => [k, settings.get(k)])),
      autostart: autostartCache,
      displayId: current.id,
    },
    catalog: apps.CATALOG.map((a) => ({
      id: a.id,
      name: a.name,
      icon: a.icon,
      on: isEnabled(a.id),
      asleep: asleep[a.id],
      mb: memory.perApp[a.id] || 0,
      prefs: Object.fromEntries(Object.keys(settings.APP_PREFS).map((k) => [k, appPref(a.id, k)])),
    })),
    memory: memory.total,
    langs: i18n.LANGS.map(({ id, name }) => ({ id, name })),
    displays: screen.getAllDisplays().map((d, i) => ({
      id: d.id,
      label: `${t('display.label', { n: i + 1 })}${d.id === primary.id ? t('display.primary') : ''} — ${Math.round(d.size.width * d.scaleFactor)}×${Math.round(d.size.height * d.scaleFactor)}`,
    })),
    hotkeys: HOTKEYS.map((h) => ({ acc: h.acc, label: h.label.replace(/Ctrl/g, t('key.ctrl')) })),
    hotkeyOk,
    dndUntil: settings.get('dndUntil'),
    update: updater.getState(),
    version: app.getVersion(),
    build: buildName(app.getVersion()),
    updateName: buildName(updater.getState().version),
    whatsNew: whatsNewState(),
    packaged: app.isPackaged,
    autostartAvailable: canAutostart(),
    cookieEncryption: COOKIE_ENCRYPTION,
    cookiesMigrated: !!settings.get('cookiesMigrated'),
    repo: REPO_URL,
  };
}

function broadcastState() {
  const s = uiState();
  if (panelWin && !panelWin.isDestroyed()) {
    panelWin.webContents.send('state', settingsMode ? { ...s, settings: settingsState() } : s);
  }
  if (tabWin && !tabWin.isDestroyed()) tabWin.webContents.send('state', s);
  updateTray();
}

function registerIpc() {
  const from = (...wins) => (e) => wins.some((w) => w && !w.isDestroyed() && e.sender === w.webContents);
  const panelOnly = from(panelWin);
  const handlers = {
    'ui:ready': [from(panelWin, tabWin, glowWin), (e) => e.sender.send('state', uiState())],
    'app:select': [panelOnly, (_e, id) => {
      if (!isEnabled(id)) return;
      if (settingsMode) { // an app tab leaves the settings screen
        settings.set('active', id);
        fitPanelToApp();
        closeSettings();
        return;
      }
      if (helpMode) { // clicking an app on the welcome / help screen means "take me there"
        settings.set('active', id);
        finishOnboarding({});
        return;
      }
      if (id === settings.get('active') && viewShowable(id)) goHome(id);
      else setActive(id);
    }],
    'panel:reload': [panelOnly, () => reloadApp(settings.get('active'))],
    'panel:retry': [panelOnly, () => reloadApp(settings.get('active'))],
    'panel:pin': [panelOnly, () => setPref('pinned', !settings.get('pinned'))],
    'panel:hide': [panelOnly, () => closePanel(true, 'hide button')],
    'panel:settings': [panelOnly, () => (settingsMode ? closeSettings() : openSettings('', 'header'))],
    'panel:update': [panelOnly, () => {
      const s = updater.getState().status;
      if (s === 'ready') updater.install();
      else openSettings('updates', 'header');
    }],
    'settings:set': [panelOnly, (_e, key, value) => setPref(key, value)],
    'settings:app': [panelOnly, (_e, id, on) => { if (typeof on === 'boolean') setAppEnabled(id, on); }],
    'settings:app-pref': [panelOnly, (_e, id, key, value) => setAppPref(id, key, value)],
    'settings:action': [panelOnly, (_e, name, arg) => {
      Promise.resolve(settingsAction(name, arg)).catch((err) => log('settings action failed', name, err));
    }],
    'panel:resize': [panelOnly, (_e, edge) => resizeTo(edge)],
    'panel:zoom-reset': [panelOnly, () => zoomStep(settings.get('active'), 0)],
    'onboarding:done': [panelOnly, (_e, opts) => finishOnboarding(opts || {})],
    'banner:dismiss': [panelOnly, () => {
      bannerShown = false;
      bannerDismissed = true;
      layoutViews();
      broadcastState();
    }],
    'tab:log': [from(tabWin), (_e, msg) => log('tab renderer:', String(msg).slice(0, 200))],
    'tab:open': [from(tabWin), (_e, id) => {
      log('tab click', id || '', { sinceShown: Date.now() - tabShownAt }, snap());
      if (!tabShown || Date.now() - tabShownAt < 150) return log('tab click ignored');
      openPanel(isEnabled(id) ? id : preferredApp(), 'tab');
    }],
  };
  for (const [channel, [allowed, fn]] of Object.entries(handlers)) {
    ipcMain.on(channel, (e, ...args) => {
      if (allowed(e)) fn(e, ...args);
    });
  }

  // From the chat sites (preload-site.js): a notification the site just raised.
  const appOf = (e) => Object.keys(views).find((k) => views[k].webContents === e.sender);
  ipcMain.on('site:notification', (e, n) => {
    const id = appOf(e);
    if (!id || !n || typeof n !== 'object') return;
    // Clicks can only be passed back to the page's main frame.
    const fromMainFrame = !e.senderFrame || e.senderFrame === e.sender.mainFrame;
    onSiteNotification(id, fromMainFrame ? n : { ...n, sw: true });
  });
  ipcMain.on('site:passkey-blocked', (e, info) => {
    const id = appOf(e);
    if (id) log('passkey request blocked', id, JSON.stringify(info || {}).slice(0, 200));
  });
}

// ---------------------------------------------------------------------------
// Start with Windows (the installer makes the Start-menu and desktop shortcuts)
// ---------------------------------------------------------------------------
// Only a real install touches "start with Windows" (one registry value shared by every copy);
// test runs with their own --profile never do.
const canAutostart = () => app.isPackaged && !profileDir;

function loginItemOptions() {
  return { path: process.execPath, args: ['--hidden'] };
}

function readOpenAtLogin() {
  if (!canAutostart()) return false;
  try {
    return app.getLoginItemSettings(loginItemOptions()).openAtLogin;
  } catch {
    return false;
  }
}

function setOpenAtLogin(on) {
  if (!canAutostart()) return;
  app.setLoginItemSettings({ openAtLogin: on, ...loginItemOptions() });
  autostartCache = readOpenAtLogin();
  broadcastState();
}

// "Start with Windows" set up by an older copy of ChatDock (e.g. the portable 1.0 folder) points at
// that copy's ChatDock.exe. Point it at this installed one instead, so it keeps working.
function migrateLoginItem() {
  if (!canAutostart()) return;
  const key = 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run';
  execFile('reg', ['query', key, '/v', AUMID], { windowsHide: true }, (err, stdout) => {
    if (err) return; // not set up
    const m = /REG_SZ\s+(.+)$/m.exec(String(stdout || ''));
    if (!m) return;
    const command = m[1].trim();
    const exe = (/^"([^"]+)"/.exec(command) || [])[1] || command.split(/\s+--/)[0];
    const same = (a, b) => path.resolve(a).toLowerCase() === path.resolve(b).toLowerCase();
    if (!exe || same(exe, process.execPath)) return;
    if (path.basename(exe).toLowerCase() !== path.basename(process.execPath).toLowerCase()) return;
    app.setLoginItemSettings({ openAtLogin: true, ...loginItemOptions() });
    autostartCache = readOpenAtLogin();
    log('start with Windows moved from', exe, 'to', process.execPath);
    broadcastState();
  });
}

function quit() {
  quitting = true;
  clearTimeout(edgeTimer);
  app.quit();
}

// ---------------------------------------------------------------------------
// README animation recorder (dev only): electron . --demo-frames=<dir> --profile=<dir> --no-occlusion
// ---------------------------------------------------------------------------
function startDemo() {
  // A free preset, so the settings screen doesn't show "hotkey taken" while another ChatDock runs.
  settings.set('hotkey', 'Control+Alt+Z');
  registerHotkey();
  const wcOf = (target) => (target === 'panel' ? panelWin.webContents : target === 'tab' ? tabWin.webContents : toasts.webContents());
  const boundsOf = (target) => (target === 'panel' ? panelWin.getContentBounds() : target === 'tab' ? tabWin.getBounds() : toasts.snapshot().bounds);
  const lastGood = {}; // per layer: the last frame that captured fine
  let captureErrors = 0;
  const ctx = {
    log,
    dir: path.resolve(DEMO),
    display: () => targetDisplay(),
    captureErrors: () => captureErrors,
    // what is on screen right now, bottom to top, each window rendered by itself
    layers: async () => {
      const out = [];
      const grab = async (name, wc, b, opacity = 1) => {
        let img = null;
        try {
          img = await wc.capturePage();
          if (img.isEmpty()) img = null;
          else lastGood[name] = img;
        } catch (err) {
          captureErrors += 1;
          if (captureErrors <= 3) log('demo capture failed', name, err.message);
          img = lastGood[name] || null; // keep showing the window; the next frame catches up
        }
        if (img) out.push({ name, img, x: b.x, y: b.y, w: b.width, h: b.height, opacity });
      };
      if (panelWin.isVisible()) {
        const b = panelWin.getBounds();
        const opacity = panelWin.getOpacity();
        const view = views[settings.get('active')];
        const vb = view && view.getVisible() ? view.getBounds() : null;
        await grab('panel', panelWin.webContents, b, opacity);
        if (vb) await grab('view', view.webContents, { x: b.x + vb.x, y: b.y + vb.y, width: vb.width, height: vb.height }, opacity);
      }
      if (glowWin.isVisible()) await grab('glow', glowWin.webContents, glowWin.getBounds());
      if (tabWin.isVisible()) await grab('tab', tabWin.webContents, tabWin.getBounds());
      const t = toasts.snapshot();
      if (t.visible) await grab('toast', toasts.webContents(), t.bounds);
      return out;
    },
    notify: (id, title, body, icon) => views[id].webContents.executeJavaScript(
      `(() => { new Notification(${JSON.stringify(title)}, { body: ${JSON.stringify(body)}, icon: ${JSON.stringify(icon || '')} }); return true; })()`),
    // centre of an element in one of our windows: page coordinates + screen coordinates
    point: async (target, selector) => {
      const r = await wcOf(target).executeJavaScript(
        `(() => { const el = document.querySelector(${JSON.stringify(selector)}); if (!el) return null;
           el.scrollIntoView({ block: 'nearest' }); const r = el.getBoundingClientRect();
           return { x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2) }; })()`);
      if (!r) return null;
      const b = boundsOf(target);
      return { local: r, screen: { x: b.x + r.x, y: b.y + r.y } };
    },
    click: (target, local) => {
      const wc = wcOf(target);
      for (const type of ['mouseDown', 'mouseUp']) wc.sendInputEvent({ type, ...local, button: 'left', clickCount: 1 });
      if (target === 'toast') wc.sendInputEvent({ type: 'mouseLeave', x: -1, y: -1 }); // the pointer moves on
    },
    gotoSettings: (section) => panelWin.webContents.send('settings:goto', section),
    showTab: (y) => showTab(y),
    hideTab: () => hideTab(false),
    clickElsewhere: () => closePanel(true, 'clicked elsewhere'),
    setPref,
  };
  setTimeout(() => {
    let demo;
    try {
      demo = require('./demo'); // dev checkouts only
    } catch {
      log('demo recorder is not included in this build');
      return;
    }
    demo.run(ctx).catch((err) => log('demo crashed', err)).finally(() => quit());
  }, 12000);
}

// ---------------------------------------------------------------------------
// Self-test (npm start -- --selftest --profile=<dir> [--shots=<dir>] [--keep])
// ---------------------------------------------------------------------------
async function runSelfTest() {
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const vis = async (id) => {
    try {
      return await views[id].webContents.executeJavaScript('document.visibilityState');
    } catch (err) {
      return `err:${err.message}`;
    }
  };
  // Screen grab (black while the PC is locked) plus each window's own rendering, which still works then.
  const shot = async (name) => {
    const dir = argValue('shots');
    if (!dir) return;
    fs.mkdirSync(dir, { recursive: true });
    const grab = async (wc, suffix) => {
      try {
        const img = await wc.capturePage();
        if (img.isEmpty()) return 'empty';
        fs.writeFileSync(path.join(dir, `${name}-${suffix}.png`), img.toPNG());
        return img.getSize();
      } catch (err) {
        return `err:${err.message}`;
      }
    };
    const out = {};
    if (tabWin.isVisible()) out.tab = await grab(tabWin.webContents, 'tab'); // first: the tab tucks itself away quickly
    if (panelWin.isVisible()) {
      out.host = await grab(panelWin.webContents, 'host');
      const view = views[settings.get('active')];
      if (view.getVisible()) {
        out.view = await grab(view.webContents, 'view');
        out.viewBounds = view.getBounds();
      }
      out.panelBounds = panelWin.getBounds();
    }
    if (glowWin.isVisible()) out.glow = await grab(glowWin.webContents, 'glow');
    if (toasts.snapshot().visible) out.toast = await grab(toasts.webContents(), 'toast');
    if (argv.includes('--screen')) { // whole-screen grab; only meaningful while the PC is unlocked
      const d = targetDisplay();
      const sources = await desktopCapturer.getSources({
        types: ['screen'],
        thumbnailSize: { width: Math.round(d.size.width * d.scaleFactor), height: Math.round(d.size.height * d.scaleFactor) },
      });
      const src = sources.find((s) => s.display_id === String(d.id)) || sources[0];
      if (src) {
        fs.writeFileSync(path.join(dir, `${name}-screen.png`), src.thumbnail.toPNG());
        out.screen = src.thumbnail.getSize();
      }
    }
    log('shot', name, JSON.stringify(out));
  };

  log('selftest: win32 available =', win32.available(), '| hotkey ok =', hotkeyOk, '| apps', enabledApps());
  await wait(11000);
  for (const id of enabledApps()) {
    const wc = views[id].webContents;
    log('page', id, '| url', wc.getURL(), '| title', JSON.stringify(wc.getTitle()), '| load', loadState[id], '| firstShown', firstShown[id]);
  }
  log('UA', views[enabledApps()[0]].webContents.getUserAgent());
  log('RAM after start:', JSON.stringify(memoryStats()), RAM_TWEAKS ? '(RAM tweaks on)' : '(RAM tweaks off)');
  const byType = {};
  for (const m of app.getAppMetrics()) {
    const k = m.type === 'Utility' ? `Utility:${m.serviceName || m.name || '?'}` : m.type;
    byType[k] = byType[k] || { n: 0, mb: 0 };
    byType[k].n += 1;
    byType[k].mb += Math.round((m.memory.privateBytes || m.memory.workingSetSize || 0) / 1024);
  }
  const ours = [panelWin, tabWin, glowWin].map((w) => w.webContents.getOSProcessId()).concat(toasts.webContents().getOSProcessId());
  log('RAM by process type:', JSON.stringify(byType), '| ChatDock page processes', JSON.stringify(ours));
  for (const id of enabledApps()) {
    // Inspect the guards without making a real passkey request (that could pop a dialog).
    log('site hooks', id, await views[id].webContents.executeJavaScript(
      `Promise.all([CredentialsContainer.prototype.get.name, CredentialsContainer.prototype.create.name,
        window.PublicKeyCredential ? PublicKeyCredential.isConditionalMediationAvailable() : 'n/a',
        Notification.permission, navigator.permissions.query({ name: 'notifications' }).then((p) => p.state),
        document.featurePolicy ? document.featurePolicy.allowsFeature('publickey-credentials-get') : 'n/a'])
        .then((r) => JSON.stringify(r))`).catch((err) => `err:${err.message}`), '(expect guarded, guarded, false, granted, granted, false)');
  }

  if (!settings.get('onboarded')) { // first run: finish the welcome screen (it may have been dismissed meanwhile)
    if (panelState === 'hidden') {
      openPanel(null, 'selftest');
      await wait(600);
    }
    await shot('01-welcome');
    finishOnboarding({});
    for (const id of enabledApps()) {
      setActive(id);
      await wait(1500);
      await shot(`02-app-${id}`);
    }
    log('visibility (panel open, last app active):', JSON.stringify(Object.fromEntries(
      await Promise.all(enabledApps().map(async (id) => [id, await vis(id)])))));
    closePanel(true);
    await wait(600);
  }
  log('panel after close:', panelState, '| visible', panelWin.isVisible());
  log('visibility (panel hidden): ig =', await vis('instagram'), '| fb =', await vis('facebook'));
  const popupsWere = settings.get('popups');
  settings.set('popups', false); // the count checks below shouldn't pop anything up

  onTitle('instagram', '(3) Instagram');
  log('count after "(3) Instagram" =', counts.instagram, '(expect 3)');
  onTitle('instagram', 'Somchai sent you a message');
  log('count right after flash title =', counts.instagram, '(expect 3)');
  await wait(3400);
  log('count 3.4s after a plain title =', counts.instagram, '(expect 0)');
  onTitle('facebook', '(2) Facebook');
  await wait(300);
  log('glow visible =', glowWin.isVisible(), glowWin.getBounds(), '(expect true)');
  await shot('04-glow');

  const d = targetDisplay();
  showTab(d.bounds.y + d.bounds.height / 2);
  await wait(300);
  log('tab visible =', tabWin.isVisible(), tabWin.getBounds(), '| glow hidden =', !glowWin.isVisible());
  log('tab renderer', await tabWin.webContents.executeJavaScript(`JSON.stringify({
    cls: document.getElementById('pill').className,
    transform: getComputedStyle(document.getElementById('pill')).transform,
    visibility: document.visibilityState })`));
  await shot('05-tab');

  // Click the last app's icon on the tab (renderer-level click; real clicks are covered by stress.js)
  const lastApp = enabledApps()[enabledApps().length - 1];
  setActive(enabledApps()[0], false);
  const icon = await tabWin.webContents.executeJavaScript(
    `(() => { const r = document.querySelector('.app[data-app="${lastApp}"]').getBoundingClientRect();
       return { x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2) }; })()`);
  for (const type of ['mouseDown', 'mouseUp']) tabWin.webContents.sendInputEvent({ type, ...icon, button: 'left', clickCount: 1 });
  await wait(700);
  log(`after clicking the tab ${lastApp} icon: panel`, panelState, '| active', settings.get('active'), `(expect open, ${lastApp})`);
  if (panelState === 'hidden') openPanel(lastApp, 'selftest');
  await wait(300);
  log('panel:', panelState, '| bounds', panelWin.getBounds(), '| focused', panelWin.isFocused(), '| tab hidden', !tabWin.isVisible());
  await shot('06-open-last-app');
  closePanel(true);
  await wait(600);
  for (const id of enabledApps()) setCount(id, 0);
  settings.set('popups', popupsWere);

  // Pop-ups: the site raises a real web notification -> ChatDock pop-up -> click -> that chat opens
  const popApp = isEnabled('discord') ? 'discord' : enabledApps()[0];
  const avatar = `data:image/png;base64,${fs.readFileSync(path.join(ASSETS, 'icon.png')).toString('base64')}`;
  await views[popApp].webContents.executeJavaScript(`(() => {
    const n = new Notification('สมชาย ใจดี', { body: 'ว่างไหม เย็นนี้ลงแรงค์ด้วยกัน 5 ตาพอ 🎮', icon: ${JSON.stringify(avatar)}, tag: 'dm-1' });
    n.onclick = () => { window.__cdClicked = (window.__cdClicked || 0) + 1; };
  })()`);
  await wait(900);
  log('pop-up after a site notification:', JSON.stringify(toasts.snapshot()), '(expect visible, count 1)');
  await shot('08-popup');
  const card = await toasts.webContents().executeJavaScript(
    `(() => { const r = document.querySelector('.card').getBoundingClientRect();
       return { x: Math.round(r.x + 70), y: Math.round(r.y + r.height / 2) }; })()`);
  for (const type of ['mouseDown', 'mouseUp']) toasts.webContents().sendInputEvent({ type, ...card, button: 'left', clickCount: 1 });
  await wait(900);
  log('after clicking the pop-up: panel', panelState, '| active', settings.get('active'),
    '| site onclick ran', await views[popApp].webContents.executeJavaScript('window.__cdClicked || 0'),
    '| pop-ups left', toasts.snapshot().count, `(expect open, ${popApp}, 1, 0)`);
  closePanel(true);
  await wait(600);

  // A page that just (re)loaded shows its old unread count: that must not pop up as "new message"
  const quietApp = isEnabled('x') ? 'x' : enabledApps()[0];
  loadStartedAt[quietApp] = Date.now();
  onTitle(quietApp, '(4) Messages / X');
  await wait(2900);
  log('pop-up right after a page load:', JSON.stringify(toasts.snapshot()), '(expect not visible)');
  setCount(quietApp, 0);

  // A site that gives no notification text: the unread count + title flash still pop something up
  loadStartedAt[quietApp] = 0;
  onTitle(quietApp, 'Somchai sent you a message');
  onTitle(quietApp, '(1) Messages / X');
  await wait(2900);
  log('pop-up from the unread count:', JSON.stringify(toasts.snapshot()), '(expect visible, count 1)');
  await shot('09-popup-fallback');
  setCount(quietApp, 0);
  await wait(400);
  log('pop-up gone once read elsewhere:', JSON.stringify(toasts.snapshot()), '(expect not visible)');

  // Switching an app on and off at runtime
  setAppEnabled('telegram', true);
  await wait(500);
  log('telegram on: view', !!views.telegram, '| tab height', tabHeight(), '| apps', enabledApps());
  setAppEnabled('telegram', false);
  await wait(300);
  log('telegram off: view', !!views.telegram, '| apps', enabledApps());
  openPanel(enabledApps()[0], 'selftest');
  await wait(600);

  // Failed load -> error screen -> retry -> recovers
  // (Instagram's service worker hides real offline blips, so force a DNS failure instead.)
  setActive('instagram', false);
  views.instagram.webContents.loadURL('https://chatdock-selftest.invalid/').catch(() => {});
  await wait(2500);
  log('failed load: load state =', loadState.instagram, '| view visible =', views.instagram.getVisible(), '(expect error, false)');
  await shot('07-error');
  reloadApp('instagram'); // what the "Try again" button does
  await wait(7000);
  log('after retry: load state =', loadState.instagram, '| view visible =', views.instagram.getVisible(),
    '| url', views.instagram.webContents.getURL(), '(expect ready, true)',
    JSON.stringify({ helpMode, onboarded: settings.get('onboarded'), firstShown: firstShown.instagram, active: settings.get('active'), panelState }));

  closePanel(true);
  await wait(600);
  log('panel after close:', panelState, '| visible', panelWin.isVisible(), '| glow visible', glowWin.isVisible());

  // Settings screen, and values it must refuse
  openSettings('', 'selftest');
  await wait(700);
  log('settings screen: open', settingsMode, '| panel', panelState, '| chat view hidden', !views[settings.get('active')].getVisible(),
    '| page shows it', await panelWin.webContents.executeJavaScript("!document.getElementById('settings').hidden"), '(expect true, open, true, true)');
  await shot('10-settings');
  log('bad values refused:', JSON.stringify([setPref('side', 'up'), setPref('opacity', 5), setPref('nope', 1),
    setPref('__proto__', {}), setPref('popupMax', '3'), setPref('hotkey', 'Alt+F4')]), '(expect all false)');
  panelWin.webContents.send('settings:goto', 'popups');
  await wait(900);
  log('jump to the pop-up section: its top is at', await panelWin.webContents.executeJavaScript(
    "Math.round(document.querySelector('[data-section=\"popups\"]').getBoundingClientRect().top)"), 'px (expect about 100-160)');
  await shot('11-settings-popups');
  for (const section of ['look', 'security', 'updates']) {
    panelWin.webContents.send('settings:goto', section);
    await wait(900);
    await shot(`12-settings-${section}`);
  }

  // Dock on the left edge: panel, views, tab and glow all move over
  setPref('side', 'left');
  await wait(700);
  const wa = targetDisplay().workArea;
  const lb = panelWin.getBounds();
  log('left side: panel x', lb.x, '(expect', `${wa.x})`, '| view x', views[settings.get('active')].getBounds().x, '(expect 0)',
    '| body.left', await panelWin.webContents.executeJavaScript("document.body.classList.contains('left')"));
  await shot('13-settings-left');
  closeSettings();
  await wait(500);
  await shot('14-left-chat');
  closePanel(true);
  await wait(600);
  log('left side: closed panel parked at x', panelWin.getBounds().x, '(expect', `${wa.x - lb.width})`);
  const db = targetDisplay().bounds;
  showTab(db.y + db.height / 2);
  await wait(400);
  log('left side: tab x', tabWin.getBounds().x, '(expect', `${db.x})`,
    '| tab mirrored', await tabWin.webContents.executeJavaScript("document.body.classList.contains('left')"));
  await shot('15-left-tab');
  hideTab(true);
  await wait(300);
  setCount(enabledApps()[0], 2);
  await wait(400);
  log('left side: glow x', glowWin.getBounds().x, '(expect', `${db.x})`, '| visible', glowWin.isVisible());
  await shot('16-left-glow');
  setCount(enabledApps()[0], 0);

  // Pop-up settings: corner, do-not-disturb, per-app switch, avatar, duration
  setPref('popupPosition', 'bottom-left');
  testPopup();
  await wait(900);
  const tb = toasts.snapshot().bounds;
  log('pop-up bottom-left:', JSON.stringify(tb), '(expect x near', wa.x - 4, 'and bottom near', `${wa.y + wa.height + 4})`);
  await shot('17-popup-bottom-left');
  toasts.dismissAll();
  const notifyFrom = (id) => views[id].webContents.executeJavaScript(
    "(() => { new Notification('ทดสอบ', { body: 'ข้อความทดสอบ' }); return true; })()");
  setDnd(30);
  await notifyFrom(popApp);
  await wait(900);
  log('do not disturb: pop-up visible', toasts.snapshot().visible, '(expect false)');
  setDnd(0);
  setAppPref(popApp, 'popups', false);
  await notifyFrom(popApp);
  await wait(900);
  log(`${popApp} pop-ups off: pop-up visible`, toasts.snapshot().visible, '(expect false)');
  setAppPref(popApp, 'popups', true);
  setPref('popupDuration', 5);
  await notifyFrom(popApp);
  await wait(900);
  log('pop-ups back on: visible', toasts.snapshot().visible, '(expect true)');
  await wait(5600);
  log('5 s duration: still visible', toasts.snapshot().visible, '(expect false)');
  setPref('popupDuration', 0);
  await notifyFrom(popApp);
  await wait(6500);
  log('"until closed": still visible after 6.5 s', toasts.snapshot().visible, '(expect true)');
  toasts.dismissAll();
  setPref('popupDuration', 8);
  setPref('popupPosition', 'top-right');

  // Update pop-up -> click -> settings opens on the update section
  announceUpdate({ version: '9.9.9', status: 'ready' });
  await wait(900);
  await shot('18-update-popup');
  const ucard = await toasts.webContents().executeJavaScript(
    `(() => { const r = document.querySelector('.card').getBoundingClientRect();
       return { x: Math.round(r.x + 70), y: Math.round(r.y + r.height / 2) }; })()`);
  for (const type of ['mouseDown', 'mouseUp']) toasts.webContents().sendInputEvent({ type, ...ucard, button: 'left', clickCount: 1 });
  await wait(1200);
  log('after clicking the update pop-up: panel', panelState, '| settings', settingsMode, '(expect open, true)');
  updateAnnounced = '';
  setPref('side', 'right');
  await wait(500);
  closePanel(true);
  await wait(600);
  log('back on the right: panel parked at x', panelWin.getBounds().x, '| side', settings.get('side'));

  // Languages: the settings screen, the tray tooltip and pop-ups follow the chosen language
  const langBefore = settings.get('lang');
  openSettings('', 'selftest');
  await wait(700);
  for (const lang of ['en', 'th', 'zh', 'ja', 'de']) {
    setPref('lang', lang);
    await wait(500);
    const seen = await panelWin.webContents.executeJavaScript(
      `JSON.stringify([document.querySelector('#settings h1').textContent, document.documentElement.lang,
        document.querySelector('[data-goto="popups"]').textContent, document.querySelector('[data-text="memory"]').textContent])`);
    log(`language ${lang}:`, seen, '| tray says', JSON.stringify(t('tray.open')));
    await shot(`20-settings-${lang}`);
  }
  panelWin.webContents.send('settings:goto', 'apps');
  await wait(900);
  await shot('21-settings-apps-de');
  await panelWin.webContents.executeJavaScript("document.querySelector('[data-perapp]').scrollIntoView({ block: 'center' })");
  await wait(500);
  await shot('22-settings-perapp-de');
  setPref('lang', langBefore);
  closeSettings();
  await wait(300);
  closePanel(true);
  await wait(600);

  // Per-app switches: message text off for one app, unread count off for another
  toasts.dismissAll();
  setAppPref(popApp, 'preview', false);
  await notifyFrom(popApp);
  await wait(900);
  const hidden = await toasts.webContents().executeJavaScript("document.querySelector('.card .body')?.textContent || ''");
  log(`${popApp} message text off: pop-up body`, JSON.stringify(hidden), '(expect', JSON.stringify(t('toast.sentYou')), ')');
  toasts.dismissAll();
  setAppPref(popApp, 'preview', true);
  const badgeApp = enabledApps()[0];
  setAppPref(badgeApp, 'badge', false);
  setCount(badgeApp, 3);
  await wait(300);
  log(`${badgeApp} unread count off: counted`, counts[badgeApp], '| shown', uiState().counts[badgeApp], '| glow', glowWin.isVisible(), '(expect 3, 0, false)');
  setCount(badgeApp, 0);
  setAppPref(badgeApp, 'badge', true);

  // RAM saver: an app set to sleep unloads after being unused, and wakes up when opened
  const sleeper = enabledApps().find((id) => id !== settings.get('active')) || enabledApps()[0];
  const memBefore = memoryStats();
  setAppPref(sleeper, 'sleep', true);
  lastUsed[sleeper] = 0;
  sleepCheck();
  await wait(2500);
  const memAfter = memoryStats();
  log(`${sleeper} sleep: asleep`, asleep[sleeper], '| page loaded', !!views[sleeper], `| RAM ${memBefore.total} MB -> ${memAfter.total} MB`,
    `(${memBefore.processes} -> ${memAfter.processes} processes)`, '(expect true, false, less)');
  openPanel(sleeper, 'selftest');
  await wait(1500);
  log(`${sleeper} opened: asleep`, asleep[sleeper], '| page loaded', !!views[sleeper], '(expect false, true)');
  setAppPref(sleeper, 'sleep', false);
  closePanel(true);
  await wait(600);
  log('RAM now:', JSON.stringify(memoryStats()));

  // The tab glides along the edge: the cursor slides 300 px down; no frame may jump
  const dsp = targetDisplay().bounds;
  showTab(dsp.y + dsp.height / 2 - 150);
  await wait(250);
  let cursorY = dsp.y + dsp.height / 2 - 150;
  let biggest = 0;
  let prev = tabWin.getBounds().y;
  for (let i = 0; i < 60; i += 1) {
    if (i < 30) cursorY += 10; // 10 px per frame for 30 frames, then stop
    followCursor(cursorY);
    glideTab();
    const y = tabWin.getBounds().y;
    biggest = Math.max(biggest, Math.abs(y - prev));
    prev = y;
    await wait(16);
  }
  const tb2 = tabWin.getBounds();
  log('tab glide: biggest step', biggest, 'px | cursor inside the pill at the end',
    cursorY >= tb2.y + 16 && cursorY <= tb2.y + tb2.height - 16, '(expect <= 15, true)');
  hideTab(true);
  log('mouse taken by a game right now:', win32.mouseCaptured(), '| pointer hidden', win32.cursorHidden(), '| confined', win32.cursorConfined());

  // Updating: the "Updating ChatDock" window, then the start after an update
  log('build names:', buildName(app.getVersion()), '|', buildName('1.4.0'), '|', buildName('2.0.0'));
  const shown = showUpdateWindow({ version: '1.4.0', notes: '' });
  await wait(1300);
  if (updateWin && !updateWin.isDestroyed()) {
    await shot('30-update-window');
    const img = await updateWin.webContents.capturePage();
    if (argValue('shots')) fs.writeFileSync(path.join(argValue('shots'), '30-update-window-page.png'), img.toPNG());
    log('update window:', JSON.stringify(await updateWin.webContents.executeJavaScript(
      "[document.querySelector('.top b').textContent, document.getElementById('route').textContent, document.getElementById('step').textContent]")));
  }
  await shown;
  if (updateWin && !updateWin.isDestroyed()) updateWin.destroy();
  settings.set('whatsNew', { version: app.getVersion(), from: '1.2.0', notes: '', at: Date.now() });
  announceUpdated();
  await wait(900);
  await shot('31-updated-popup');
  log('updated pop-up:', JSON.stringify(await toasts.webContents().executeJavaScript(
    "[document.querySelector('.card .title')?.textContent, document.querySelector('.card .body')?.textContent]")));
  const upd = await toasts.webContents().executeJavaScript(
    `(() => { const r = document.querySelector('.card').getBoundingClientRect();
       return { x: Math.round(r.x + 70), y: Math.round(r.y + r.height / 2) }; })()`);
  for (const type of ['mouseDown', 'mouseUp']) toasts.webContents().sendInputEvent({ type, ...upd, button: 'left', clickCount: 1 });
  await wait(1500);
  log('after clicking it: settings', settingsMode, '| notes box', JSON.stringify(await panelWin.webContents.executeJavaScript(
    "[!document.querySelector('.uc-notes').hidden, document.querySelector('[data-text=notesTitle]').textContent]")), '(expect true, [true, what\'s new…])');
  await shot('32-whats-new');
  closePanel(true);
  await wait(600);
  log('selftest done');
  if (!argv.includes('--keep')) quit();
}
