'use strict';

// ChatDock's own message pop-ups: a small always-on-top window in a screen corner that also shows
// over borderless games (Windows' own toasts are muted while you play). One window holds a stack
// of cards; it is shown without taking focus, so a game keeps its keyboard and mouse.

const { BrowserWindow, ipcMain } = require('electron');

const CARD_W = 360;
const PAD = 16; // transparent room around the cards for their shadow (keep in sync with toast.css)
const EDGE = 12; // gap between the cards and the screen edge / the open chat panel
const MAX_QUEUE = 30;
const lifetimeMs = () => Math.max(0, Number(d.duration()) || 0) * 1000; // 0 = stays until dismissed
const maxVisible = () => Math.min(5, Math.max(1, Number(d.maxVisible()) || 3));

let d = null; // hooks from main.js
let win = null;
let winHwnd = 0;
let items = []; // newest first
let contentHeight = 0;
let hovered = false;
let fgBefore = 0; // window that was in front of the pop-ups (usually the game)
let lastFg = 0;
let watchTimer = null;
let seq = 0;
let pendingShow = false;
let showFallback = null;
let hideFallback = null;

function init(deps) {
  d = deps;
  win = new BrowserWindow({
    width: CARD_W + PAD * 2, height: 120, show: false, frame: false, transparent: true, resizable: false,
    minimizable: false, maximizable: false, fullscreenable: false, skipTaskbar: true,
    // Focusable on purpose: Chromium drops clicks on non-focusable windows once they have been
    // hidden and shown again (the same bug the edge tab had). Shown with showInactive(), so a
    // pop-up appearing never takes focus from the game.
    focusable: true,
    alwaysOnTop: true, hasShadow: false, thickFrame: false, roundedCorners: false,
    webPreferences: d.webPreferences({ backgroundThrottling: false }),
  });
  win.setAlwaysOnTop(true, 'screen-saver');
  d.lockDown(win.webContents);
  win.on('close', (e) => { if (!d.quitting()) e.preventDefault(); });
  // Safety net: a click on the pop-ups that didn't open the chat must not leave the game unfocused.
  win.on('focus', () => setTimeout(giveFocusBack, 250));
  win.loadURL(d.uiUrl('toast.html'));
  winHwnd = d.hwndOf(win);

  const fromToast = (e) => e.sender === win.webContents;
  ipcMain.on('toast:size', (e, h) => {
    if (!fromToast(e) || !Number.isFinite(h)) return;
    contentHeight = Math.ceil(h);
    place();
    if (pendingShow) reveal();
  });
  // The last card has finished sliding away.
  ipcMain.on('toast:empty', (e) => { if (fromToast(e) && !items.length) hide(); });
  ipcMain.on('toast:click', (e, key) => { if (fromToast(e)) openFrom(remove(key)); });
  ipcMain.on('toast:more', (e) => { if (fromToast(e)) openFrom(items[0] ? remove(items[0].key) : null); });
  ipcMain.on('toast:dismiss', (e, key) => {
    if (!fromToast(e)) return;
    const wasInFront = d.foreground() === winHwnd;
    remove(key);
    if (wasInFront) returnFocus();
  });
  ipcMain.on('toast:hover', (e, on) => { if (fromToast(e)) setHovered(!!on); });
}

function hwnd() {
  return winHwnd;
}

// toast: { appId, appName, iconName, accent, title, body, icon, meta, hint, tag, sourceId, chime, action?, sticky? }
function push(toast) {
  if (!win) return;
  if (toast.tag) {
    const same = items.find((x) => x.tag === toast.tag);
    if (same) remove(same.key, false); // an update of the same conversation replaces the old card
  }
  const item = { ...toast, key: `t${++seq}`, at: Date.now(), remaining: lifetimeMs(), timer: null, startedAt: 0 };
  items.unshift(item);
  while (items.length > MAX_QUEUE) clearTimeout(items.pop().timer);
  if (!hovered) start(item);
  render(true);
}

function send(visible, more, isNew) {
  win.webContents.send('toasts', {
    items: visible.map(({ key, appId, appName, iconName, accent, title, body, icon, meta, hint }) => ({
      key, appId, appName, iconName, accent, title, body, icon, meta, hint,
    })),
    more,
    labels: { more: d.t('toast.more', { n: more }), close: d.t('toast.close') },
    lang: d.lang(),
    position: d.position(),
    shown: win.isVisible(), // cards only slide in / out / along while the window is on screen
    chime: !!(isNew && items[0] && items[0].chime), // the newest card decides (per-app chime switch)
  });
}

function render(isNew) {
  if (!win) return;
  if (!items.length) {
    if (win.isVisible() && !pendingShow) {
      // The last cards slide away first; the page says 'toast:empty' when they are gone.
      send([], 0, false);
      clearTimeout(hideFallback);
      hideFallback = setTimeout(hide, 900);
    } else {
      hide();
    }
    return;
  }
  clearTimeout(hideFallback);
  const visible = items.slice(0, maxVisible());
  const more = items.length - visible.length;
  send(visible, more, isNew);
  if (!win.isVisible() && !pendingShow) {
    // Wait for the renderer to report the stack's height, so the window never shows at a wrong size.
    const fg = d.foreground();
    if (fg && fg !== winHwnd) fgBefore = fg;
    pendingShow = true;
    clearTimeout(showFallback);
    showFallback = setTimeout(reveal, 150); // in case the size report is slow
  } else if (win.isVisible()) {
    d.raise(win);
  }
}

function reveal() {
  clearTimeout(showFallback);
  if (!pendingShow) return;
  pendingShow = false;
  if (!items.length) return;
  place();
  win.showInactive(); // never takes focus from the game
  d.raise(win);
  startWatch();
}

function place() {
  if (!win || !items.length) return;
  const wa = d.targetDisplay().workArea;
  const w = CARD_W + PAD * 2;
  const h = Math.max(60, Math.min(contentHeight || 120, wa.height));
  const pos = d.position(); // 'top-right' | 'bottom-right' | 'top-left' | 'bottom-left'
  const onLeft = pos.endsWith('left');
  const panel = d.panelBounds(); // the open chat: pop-ups go beside it, never on top of it
  let x;
  if (onLeft) {
    x = wa.x + EDGE - PAD; // cards start EDGE px from the left screen edge
    if (panel && panel.x <= wa.x + 1) x = Math.max(x, panel.x + panel.width + EDGE - PAD);
  } else {
    let right = wa.x + wa.width - EDGE + PAD; // cards end EDGE px from the right screen edge
    if (panel && panel.x + panel.width >= wa.x + wa.width - 1) right = Math.min(right, panel.x - EDGE + PAD);
    x = right - w;
  }
  const y = pos.startsWith('bottom') ? wa.y + wa.height - EDGE + PAD - h : wa.y + EDGE - PAD;
  win.setBounds({ x: Math.round(x), y: Math.round(y), width: w, height: Math.round(h) });
}

function start(item) {
  clearTimeout(item.timer);
  item.timer = null;
  item.startedAt = Date.now();
  if (item.sticky || item.remaining <= 0) return; // stays until clicked or closed
  item.timer = setTimeout(() => remove(item.key), Math.max(1500, item.remaining));
}

function setHovered(on) {
  if (on === hovered) return;
  hovered = on;
  for (const it of items) {
    if (on) {
      if (it.timer) {
        clearTimeout(it.timer);
        it.timer = null;
        it.remaining -= Date.now() - it.startedAt;
      }
    } else {
      if (it.remaining > 0) it.remaining = Math.max(it.remaining, 2500);
      start(it);
    }
  }
}

function remove(key, rerender = true) {
  const i = items.findIndex((x) => x.key === key);
  if (i < 0) return null;
  const [it] = items.splice(i, 1);
  clearTimeout(it.timer);
  if (rerender) render(false);
  return it;
}

function dismissApp(appId) {
  const before = items.length;
  items = items.filter((it) => {
    if (it.appId !== appId) return true;
    clearTimeout(it.timer);
    return false;
  });
  if (items.length !== before) render(false);
}

function dismissSource(appId, sourceId) {
  const it = items.find((x) => x.appId === appId && x.sourceId === sourceId);
  if (it) remove(it.key);
}

function openFrom(it) {
  if (it) d.open(it, fgBefore);
}

function dismissAll() {
  for (const it of items) clearTimeout(it.timer);
  items = [];
  render(false);
}

function hide() {
  stopWatch();
  clearTimeout(showFallback);
  clearTimeout(hideFallback);
  pendingShow = false;
  hovered = false;
  contentHeight = 0;
  if (win && win.isVisible()) win.hide();
}

function returnFocus() {
  if (fgBefore && fgBefore !== winHwnd) {
    d.restoreFocus(fgBefore);
    if (win.isVisible()) d.raise(win);
  }
}

function giveFocusBack() {
  if (d.panelActive() || d.foreground() !== winHwnd) return;
  returnFocus();
}

// While pop-ups are up: remember the latest window in front, and stay above topmost games.
function startWatch() {
  stopWatch();
  lastFg = d.foreground();
  watchTimer = setInterval(() => {
    const fg = d.foreground();
    if (fg && fg !== winHwnd) fgBefore = fg;
    if (fg !== lastFg) {
      lastFg = fg;
      d.raise(win);
    }
  }, 400);
}

function stopWatch() {
  clearInterval(watchTimer);
  watchTimer = null;
}

function reposition() {
  if (win && win.isVisible()) place();
}

// Pop-up settings changed (corner, how many at once): redraw what is on screen.
function refresh() {
  if (win && items.length) render(false);
}

function setContentProtection(on) {
  if (win) win.setContentProtection(!!on);
}

// For tests
function snapshot() {
  return { visible: !!(win && win.isVisible()), bounds: win ? win.getBounds() : null, count: items.length };
}

function webContents() {
  return win ? win.webContents : null;
}

module.exports = {
  init, push, dismissApp, dismissSource, dismissAll, reposition, refresh, setContentProtection, hwnd, snapshot, webContents,
};
