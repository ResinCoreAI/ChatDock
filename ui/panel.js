'use strict';

const $ = (sel) => document.querySelector(sel);
const nav = $('.apps');
let tabsKey = '';
let side = '';
let lang = '';
let glider = null;
const counted = {}; // each app's unread number as last shown

// One tab per switched-on app: the active one shows its name, the others just the icon + unread count.
function renderTabs(s) {
  const key = s.apps.map((a) => a.id).join(',');
  const rebuilt = key !== tabsKey;
  if (rebuilt) {
    tabsKey = key;
    nav.textContent = '';
    for (const a of s.apps) {
      const btn = document.createElement('button');
      btn.className = 'app';
      btn.dataset.app = a.id;
      btn.setAttribute('role', 'tab');
      const ico = document.createElement('span');
      ico.className = 'ico';
      ico.innerHTML = window.iconHTML(a.icon);
      const name = document.createElement('span');
      name.className = 'name';
      name.textContent = a.name;
      const badge = document.createElement('span');
      badge.className = 'badge';
      badge.hidden = true;
      btn.append(ico, name, badge);
      btn.addEventListener('click', () => chatdock.send('app:select', a.id));
      nav.append(btn);
    }
    glider = document.createElement('i'); // last, so nav.children[i] stays the i-th app
    glider.className = 'glider';
    nav.append(glider);
  }
  s.apps.forEach((a, i) => {
    const btn = nav.children[i];
    const active = a.id === s.active;
    btn.classList.toggle('active', active);
    btn.classList.toggle('asleep', !!a.asleep); // RAM saver: dimmed until opened
    btn.setAttribute('aria-selected', String(active));
    btn.title = a.asleep ? i18n.t('panel.appTabSleeping', { name: a.name })
      : i < 9 ? i18n.t('panel.appTab', { name: a.name, n: i + 1 }) : a.name;
    const n = s.counts[a.id] || 0;
    const badge = btn.querySelector('.badge');
    badge.textContent = n > 99 ? '99+' : String(n);
    badge.hidden = n === 0;
    if (n > (counted[a.id] || 0) && s.shown && !rebuilt) bump(badge); // a new message while it's open
    counted[a.id] = n;
  });
  placeGlider(rebuilt);
}

// The open app's highlight glides to the app picked (it jumps into place when there was nothing to
// glide from: new tabs, a new width, the volume bar closing).
function placeGlider(jump) {
  const btn = nav.querySelector('.app.active');
  if (!glider) return;
  glider.hidden = !btn || !btn.offsetWidth;
  if (glider.hidden) return;
  glider.classList.toggle('still', jump);
  glider.style.width = `${btn.offsetWidth}px`;
  glider.style.transform = `translateX(${btn.offsetLeft}px)`;
  if (jump) {
    void glider.offsetWidth;
    glider.classList.remove('still');
  }
}
window.addEventListener('resize', () => placeGlider(true));
document.fonts.ready.then(() => placeGlider(true));

// An unread number that went up pops once.
function bump(badge) {
  badge.classList.remove('bump');
  void badge.offsetWidth;
  badge.classList.add('bump');
}

function renderHotkey(label) {
  for (const el of document.querySelectorAll('.hotkey-keys')) {
    el.textContent = '';
    if (!label) {
      el.textContent = i18n.t('hotkey.none');
      continue;
    }
    label.split(' + ').forEach((key, i) => {
      if (i) el.append(' + ');
      const kbd = document.createElement('kbd');
      kbd.textContent = key;
      el.append(kbd);
    });
  }
}

// Language pickers (welcome screen and settings): "Automatic" + each language in its own name.
function renderLangSelects(pref) {
  for (const sel of document.querySelectorAll('[data-lang-select]')) {
    if (!sel.options.length) {
      sel.append(new Option('', 'auto'));
      for (const l of window.CHATDOCK_I18N.LANGS) sel.append(new Option(l.name, l.id));
    }
    sel.options[0].textContent = i18n.t('lang.auto');
    if (pref) sel.value = pref;
  }
}

// Everything that has to be written again in a new language.
function translatePage(s) {
  i18n.set(s.lang);
  i18n.vars = {
    hotkey: '<span class="hotkey-keys"></span>',
    pin: '<span class="inline-ico" data-icon="pin"></span>',
    gear: '<span class="inline-ico" data-icon="gear"></span>',
  };
  i18n.apply();
  window.fillIcons();
}

// Docked on the left: the page mirrors (grip on the right, hide arrow points left).
function renderSide(s) {
  if (s.side === side) return;
  side = s.side;
  const left = side === 'left';
  document.body.classList.toggle('left', left);
  $('#hide .hide-ico').innerHTML = window.iconHTML(left ? 'chevronLeft' : 'chevronRight');
}

function renderUpdate(u) {
  const chip = $('#update');
  const ready = u && u.status === 'ready';
  const available = u && u.status === 'available';
  chip.hidden = !(ready || available);
  if (chip.hidden) return;
  chip.querySelector('.label').textContent = !ready ? i18n.t('panel.updateAvailable')
    : u.build ? i18n.t('panel.updateChip', { n: u.build }) : u.name;
  chip.title = i18n.t(ready ? 'panel.updateReadyTitle' : 'panel.updateAvailableTitle', { version: u.name });
}

function render(s) {
  if (s.lang !== lang) {
    lang = s.lang;
    translatePage(s);
  }
  renderSide(s);
  renderTabs(s);
  renderVolume(s);
  renderUpdate(s.update);
  renderLangSelects(s.langPref);
  const t = i18n.t;
  $('#settings-btn').classList.toggle('on', !!s.settingsOpen);
  $('#settings-btn').title = t(s.settingsOpen ? 'panel.settingsClose' : 'panel.settings');

  const pin = $('#pin');
  pin.classList.toggle('on', s.pinned);
  pin.title = t(s.pinned ? 'panel.pinOn' : 'panel.pinOff');
  $('#hide').title = s.hotkey ? t('panel.hideHotkey', { hotkey: s.hotkey }) : t('panel.hide');

  const zoom = Math.round((s.zoom || 1) * 100);
  $('#zoom').hidden = zoom === 100;
  $('#zoom').textContent = `${zoom}%`;

  const activeApp = s.apps.find((a) => a.id === s.active);
  const appName = activeApp ? activeApp.name : '';
  $('#loading-text').textContent = t('loading.text', { app: appName });
  $('#error-title').textContent = t('error.title', { app: appName });

  const settingsOn = !!s.settingsOpen;
  const welcome = !settingsOn && (s.help || !s.onboarded);
  const cover = settingsOn || welcome; // a ChatDock screen instead of the chat
  const load = s.load[s.active];
  document.body.classList.toggle('panel-hidden', !s.shown);
  $('#progress').classList.toggle('on', s.shown && !cover && load === 'loading' && s.firstShown[s.active]);
  $('#settings').hidden = !settingsOn;
  $('#welcome').hidden = !welcome;
  $('#error').hidden = cover || load !== 'error';
  $('#loading').hidden = cover || load === 'error' || s.firstShown[s.active];
  if (settingsOn && window.renderSettings) window.renderSettings(s.settings);

  document.body.classList.toggle('with-banner', !!s.banner);
  $('#banner').hidden = !s.banner;

  if (window.tourRender) window.tourRender(s);
  renderHotkey(s.hotkey);
}

// This app's volume (on top of the site's own): the speaker, its wheel, and the slider that takes
// the tabs' place.
let volState = { id: '', level: 100, on: true, own: true, name: '' };
let wheelAcc = 0;
let volSendAt = 0;
function volIcon(level, on) {
  return !on || level === 0 ? 'volumeOff' : level < 50 ? 'volumeLow' : 'volume';
}
function renderVolume(s) {
  const a = s.apps.find((x) => x.id === s.active);
  const level = (s.volumes && s.volumes[s.active]) ?? 100;
  const own = !(s.soundOn && s.soundOn[s.active] === false); // this app's own switch
  const all = s.allMuted && s.active !== 'spotify'; // "all chat sounds off" (music plays on)
  const on = own && !all;
  volState = { id: s.active, level, on, own, name: a ? a.name : '' };
  if (!s.shown || s.settingsOpen || s.help) openVolume(false); // it closes with the chat
  const icon = volIcon(level, on);
  const btn = $('#vol');
  if (btn.dataset.icon !== icon) {
    btn.dataset.icon = icon;
    btn.innerHTML = window.iconHTML(icon);
  }
  btn.classList.toggle('off', icon === 'volumeOff');
  btn.classList.toggle('low', icon === 'volumeLow');
  btn.title = i18n.t('panel.volumeOf', { name: volState.name, n: on ? level : 0 });
  const mute = $('#vol-mute');
  mute.innerHTML = window.iconHTML(icon);
  mute.classList.toggle('off', !on);
  mute.title = own && all ? i18n.t('panel.allSoundsOff') : i18n.t(on ? 'panel.mute' : 'panel.unmute', { name: volState.name });
  const range = $('#vol-range');
  if (performance.now() - volSendAt > 600) range.value = String(level); // not while it's being dragged
  range.setAttribute('aria-label', btn.title);
  $('#vol-num').textContent = `${range.value}%`;
  $('#vol-done').title = i18n.t('panel.volumeDone');
}
function setVolume(level) {
  const v = Math.max(0, Math.min(100, Math.round(level)));
  volSendAt = performance.now();
  $('#vol-range').value = String(v);
  $('#vol-num').textContent = `${v}%`;
  chatdock.send('panel:volume', volState.id, v);
}
function openVolume(open) {
  if ($('#volbar').hidden === !open) return;
  $('#volbar').hidden = !open;
  document.body.classList.toggle('vol-open', open);
  if (open) $('#vol-range').focus();
  else placeGlider(true); // the tabs are back (measured while they were away)
}
// One wheel notch = 5 %; a touchpad's many small steps add up to the same, and sideways does nothing
const wheelStep = (e) => {
  e.preventDefault();
  if (!e.deltaY) return;
  wheelAcc += e.deltaMode === 1 ? e.deltaY * 33 : e.deltaY; // (lines, roughly in pixels)
  while (Math.abs(wheelAcc) >= 100) {
    const up = wheelAcc < 0;
    wheelAcc += up ? 100 : -100;
    setVolume(Number($('#vol-range').value) + (up ? 5 : -5));
  }
};
$('#vol').addEventListener('click', () => openVolume($('#volbar').hidden));
$('#vol').addEventListener('wheel', wheelStep, { passive: false });
$('#volbar').addEventListener('wheel', wheelStep, { passive: false });
$('#vol-range').addEventListener('input', (e) => setVolume(Number(e.target.value)));
$('#vol-mute').addEventListener('click', () => {
  if (volState.own && !volState.on) chatdock.send('panel:sounds-on'); // muted by "all chat sounds off"
  else chatdock.send('panel:sound', volState.id, !volState.on);
});
$('#vol-done').addEventListener('click', () => openVolume(false));
$('#volbar').addEventListener('keydown', (e) => {
  if (e.key === 'Escape') {
    e.stopPropagation();
    openVolume(false);
  }
});

$('#reload').addEventListener('click', () => chatdock.send('panel:reload'));
$('#pin').addEventListener('click', () => chatdock.send('panel:pin'));
$('#hide').addEventListener('click', () => chatdock.send('panel:hide'));
$('#zoom').addEventListener('click', () => chatdock.send('panel:zoom-reset'));
$('#retry').addEventListener('click', () => chatdock.send('panel:retry'));
$('#banner-close').addEventListener('click', () => chatdock.send('banner:dismiss'));
$('#settings-btn').addEventListener('click', () => chatdock.send('panel:settings'));
$('#update').addEventListener('click', () => chatdock.send('panel:update'));
$('#welcome [data-lang-select]').addEventListener('change', (e) => chatdock.send('settings:set', 'lang', e.target.value));

// Drag the inner edge to resize. ChatDock follows the real cursor itself (in physical pixels, so it
// works on any monitor layout); the page only says when the drag starts, moves (once per frame)
// and ends.
const grip = $('#grip');
let dragging = false;
let tick = false;
grip.addEventListener('pointerdown', (e) => {
  if (e.button !== 0) return;
  dragging = true;
  grip.setPointerCapture(e.pointerId);
  document.body.classList.add('resizing');
  chatdock.send('panel:resize-start');
});
grip.addEventListener('pointermove', (e) => {
  if (dragging && !(e.buttons & 1)) { // the button came up somewhere we didn't hear about
    endDrag();
    return;
  }
  if (!dragging || tick) return;
  tick = true;
  requestAnimationFrame(() => {
    tick = false;
    if (dragging) chatdock.send('panel:resize');
  });
});
const endDrag = () => {
  if (!dragging) return;
  dragging = false;
  document.body.classList.remove('resizing');
  chatdock.send('panel:resize-end');
};
grip.addEventListener('pointerup', endDrag);
grip.addEventListener('lostpointercapture', endDrag);
grip.addEventListener('pointercancel', endDrag);

chatdock.on('state', render);
chatdock.send('ui:ready');
