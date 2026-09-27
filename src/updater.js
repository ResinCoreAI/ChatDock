'use strict';

// Updates from the project's GitHub Releases (electron-updater).
// Checks quietly (on start + every 6 h), can fetch the installer in the background, and only
// installs when the user presses the button. The download is checked against the SHA-512 in the
// release's latest.yml before it runs. Installing shows ChatDock's "Updating" window, then the
// installer's own progress window, and ChatDock starts again by itself.
// Releases are called "Beta Build N" and carry version 1.N.0 (see buildName in ui/i18n.js).

const { app } = require('electron');

const RECHECK_MS = 6 * 60 * 60 * 1000;

let d = null; // hooks from main.js
let autoUpdater = null;
let firstCheckTimer = null;
let recheckTimer = null;
let state = { status: 'idle', version: '', percent: 0, error: '', checkedAt: 0, notes: '' };

function set(patch) {
  state = { ...state, ...patch };
  if (d) d.onState(getState());
}

function getState() {
  return { ...state, current: app.getVersion(), enabled: !!autoUpdater };
}

const ENTITIES = {
  amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: ' ', rsquo: '’', lsquo: '‘', rarr: '→', mdash: '—', ndash: '–', hellip: '…',
};

// GitHub hands the release notes over as HTML: keep headings and list items as lines of text.
function notesOf(info) {
  const n = info && info.releaseNotes;
  if (!n) return '';
  const html = Array.isArray(n) ? n.map((x) => x.note || '').join('\n') : String(n);
  return html
    .replace(/<li[^>]*>/gi, '• ')
    .replace(/<\/(li|p|h\d|ul|ol|div)>|<br\s*\/?>/gi, '\n')
    .replace(/<[^>]+>/g, '')
    .replace(/&(#\d+|#x[0-9a-f]+|\w+);/gi, (m, e) => {
      if (e[0] === '#') return String.fromCodePoint(e[1].toLowerCase() === 'x' ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10));
      return ENTITIES[e.toLowerCase()] ?? m;
    })
    .replace(/[ \t]+\n/g, '\n')
    .replace(/\n(?:[ \t]*\n)+/g, '\n')
    .replace(/^\s+|\s+$/g, '')
    .slice(0, 1500);
}

function init(deps) {
  d = deps;
  // Release checks can point at a feed on this PC: CHATDOCK_UPDATE_FEED=http://127.0.0.1:8765/
  // (localhost only; anything else is ignored and GitHub is used).
  const feed = process.env.CHATDOCK_UPDATE_FEED || '';
  const testFeed = /^http:\/\/(127\.0\.0\.1|localhost)(:\d+)?\//.test(feed) ? feed : '';
  if (!app.isPackaged && !testFeed) {
    set({ status: 'dev' });
    return;
  }
  try {
    ({ autoUpdater } = require('electron-updater'));
  } catch (err) {
    set({ status: 'error', error: `electron-updater: ${err.message}` });
    return;
  }
  autoUpdater.logger = {
    info: (m) => d.log('updater:', String(m)),
    warn: (m) => d.log('updater warn:', String(m)),
    error: (m) => d.log('updater error:', String(m)),
    debug: () => {},
  };
  autoUpdater.autoDownload = !!d.autoDownload();
  autoUpdater.autoInstallOnAppQuit = false; // installing is the user's call
  autoUpdater.allowPrerelease = false;
  autoUpdater.allowDowngrade = false;
  autoUpdater.disableWebInstaller = true; // releases ship the full installer only
  if (testFeed) {
    autoUpdater.forceDevUpdateConfig = !app.isPackaged;
    autoUpdater.setFeedURL({ provider: 'generic', url: testFeed });
  }

  autoUpdater.on('checking-for-update', () => set({ status: 'checking', error: '' }));
  autoUpdater.on('update-available', (info) => {
    set({ status: autoUpdater.autoDownload ? 'downloading' : 'available', version: info.version, notes: notesOf(info), percent: 0 });
    d.onAvailable(getState());
  });
  autoUpdater.on('update-not-available', () => set({ status: 'latest', checkedAt: Date.now(), version: '' }));
  autoUpdater.on('download-progress', (p) => set({ status: 'downloading', percent: Math.round((p && p.percent) || 0) }));
  autoUpdater.on('update-downloaded', (info) => {
    set({ status: 'ready', version: info.version, percent: 100, checkedAt: Date.now() });
    d.onReady(getState());
    // Release check with a local feed: install straight away instead of waiting for the button.
    if (testFeed && process.env.CHATDOCK_UPDATE_AUTOINSTALL === '1') setTimeout(install, 1500);
  });
  autoUpdater.on('error', (err) => set({ status: 'error', error: String((err && err.message) || err).slice(0, 300) }));

  scheduleChecks();
}

function scheduleChecks() {
  clearTimeout(firstCheckTimer);
  clearInterval(recheckTimer);
  firstCheckTimer = null;
  recheckTimer = null;
  if (!autoUpdater || !d.autoCheck()) return;
  firstCheckTimer = setTimeout(() => check(), 20000); // let the chats load first
  recheckTimer = setInterval(() => check(), RECHECK_MS);
}

async function check() {
  if (!autoUpdater) return getState();
  if (['checking', 'downloading', 'ready', 'installing'].includes(state.status)) return getState();
  try {
    await autoUpdater.checkForUpdates();
  } catch (err) {
    set({ status: 'error', error: String((err && err.message) || err).slice(0, 300) });
  }
  return getState();
}

async function download() {
  if (!autoUpdater || state.status !== 'available') return;
  set({ status: 'downloading', percent: 0 });
  try {
    await autoUpdater.downloadUpdate();
  } catch (err) {
    set({ status: 'error', error: String((err && err.message) || err).slice(0, 300) });
  }
}

// Show the "Updating" window, quit, and let the installer run with its progress window showing;
// it starts the new version when it is done.
async function install() {
  if (!autoUpdater || state.status !== 'ready') return false;
  set({ status: 'installing' });
  try {
    await d.beforeInstall(getState());
  } catch (err) {
    d.log('before install:', String(err));
  }
  autoUpdater.quitAndInstall(false, true);
  // Normally ChatDock has quit long before this. If not, the installer didn't start (its 'error'
  // event says why): close the "Updating" window and let the user try again.
  setTimeout(() => {
    if (state.status === 'installing') set({ status: 'ready' });
    d.installFailed();
  }, 20000);
  return true;
}

function setAutoDownload(on) {
  if (autoUpdater) autoUpdater.autoDownload = !!on;
}

function setAutoCheck() {
  scheduleChecks();
}

module.exports = { init, check, download, install, getState, setAutoDownload, setAutoCheck };
