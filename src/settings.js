'use strict';

// Tiny JSON settings store kept in the app's userData folder.

const fs = require('node:fs');
const path = require('node:path');
const { app } = require('electron');
const apps = require('./apps');

// Per-app switches (Settings → Notifications → Per app, and Apps). Missing = these defaults.
const APP_PREFS = {
  popups: true,  // pop-ups from this app
  preview: true, // show the message text in them
  chime: true,   // ChatDock's chime with them
  badge: true,   // unread count on the tab/header/tray and the edge glow
  sound: true,   // the site's own sounds
  sleep: false,  // unload the app after a while unused, to save RAM
};

const DEFAULTS = {
  lang: 'auto',             // 'auto' (Windows language) | 'en' | 'th' | 'zh' | 'ja' | 'de'
  apps: apps.defaultsEnabled(), // which chat services are switched on
  width: 460,               // panel width (DIP) for apps without their own
  widths: {},               // per-app panel width the user dragged to
  active: 'instagram',      // last app shown in the panel
  pinned: false,            // keep the panel open when clicking elsewhere
  hotkey: 'Control+Alt+C',  // global toggle
  side: 'right',            // which screen edge the dock lives on: 'right' | 'left'
  edgeMode: 'always',       // 'always' | 'no-fullscreen' | 'off'
  displayId: null,          // null = primary display
  glow: true,               // light strip on the screen edge when there are unread chats
  popups: true,             // ChatDock's own always-on-top pop-up when someone messages you
  popupText: true,          // show the message text in the pop-up (off = only who wrote)
  popupAvatar: true,        // show the sender's profile picture
  popupSound: false,        // soft chime with each pop-up (the chat sites usually play their own)
  popupPosition: 'top-right', // 'top-right' | 'bottom-right' | 'top-left' | 'bottom-left'
  popupDuration: 8,         // seconds on screen; 0 = until clicked / closed
  popupMax: 3,              // cards shown at once (the rest are summed up as "+N more")
  appPrefs: {},             // per-app switches, see APP_PREFS
  popupQuietFullscreen: false, // hold pop-ups while a fullscreen game / video is in front
  dndUntil: 0,              // do-not-disturb: 0 = off, -1 = until switched off, else epoch ms
  updateAutoCheck: true,    // look for new versions on GitHub
  updateAutoDownload: true, // fetch them in the background (installing always waits for a click)
  cookiesMigrated: false,   // one-time rewrite so cookies saved before encryption get encrypted too
  muted: false,             // mute sounds coming from the chat pages
  opacity: 1,               // panel opacity (1 = solid)
  theme: 'system',          // 'system' | 'dark' | 'light'
  hideFromCapture: false,   // exclude the panel from screenshots / OBS / Discord streams
  zoom: {},                 // per-app zoom factor (missing = 100%)
  onboarded: false,
};

let data = null;
let saveTimer = null;

function file() {
  return path.join(app.getPath('userData'), 'settings.json');
}

function load() {
  let saved = {};
  try {
    saved = JSON.parse(fs.readFileSync(file(), 'utf8'));
  } catch {
    // first run or unreadable file -> defaults
  }
  const appPrefs = {};
  for (const id of apps.ALL_IDS) {
    appPrefs[id] = { ...APP_PREFS, ...((saved.appPrefs || {})[id] || {}) };
    // 1.1 only had a per-app pop-up switch
    if (!(saved.appPrefs || {})[id] && (saved.popupApps || {})[id] === false) appPrefs[id].popups = false;
  }
  data = {
    ...DEFAULTS,
    ...saved,
    apps: { ...DEFAULTS.apps, ...(saved.apps || {}) },
    widths: { ...(saved.widths || {}) },
    appPrefs,
    zoom: { ...DEFAULTS.zoom, ...(saved.zoom || {}) },
  };
  // 1.0 and 1.1 spoke Thai only: people updating from them keep Thai; new installs follow Windows.
  if (!saved.lang && saved.onboarded) data.lang = 'th';
  delete data.notifications; // replaced by ChatDock's own pop-ups
  delete data.popupApps; // moved into appPrefs
  return data;
}

function get(key) {
  if (!data) load();
  return data[key];
}

function set(key, value) {
  if (!data) load();
  data[key] = value;
  clearTimeout(saveTimer);
  saveTimer = setTimeout(flush, 400);
}

function flush() {
  clearTimeout(saveTimer);
  saveTimer = null;
  if (!data) return;
  try {
    fs.mkdirSync(path.dirname(file()), { recursive: true });
    const tmp = `${file()}.tmp`;
    fs.writeFileSync(tmp, JSON.stringify(data, null, 2));
    fs.renameSync(tmp, file());
  } catch (err) {
    console.error('[settings] save failed:', err);
  }
}

module.exports = { load, get, set, flush, DEFAULTS, APP_PREFS };
