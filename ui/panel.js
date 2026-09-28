'use strict';

const $ = (sel) => document.querySelector(sel);
const nav = $('.apps');
let tabsKey = '';
let side = '';
let lang = '';

// One tab per switched-on app: the active one shows its name, the others just the icon + unread count.
function renderTabs(s) {
  const key = s.apps.map((a) => a.id).join(',');
  if (key !== tabsKey) {
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
  });
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
  side = ''; // step 1 of the welcome screen depends on the side
  delete $('#step2').dataset.key; // and step 2 on whether there is a hotkey
}

// Docked on the left: the page mirrors (grip on the right, hide arrow points left).
function renderSide(s) {
  if (s.side === side) return;
  side = s.side;
  const left = side === 'left';
  document.body.classList.toggle('left', left);
  $('#hide .hide-ico').innerHTML = window.iconHTML(left ? 'chevronLeft' : 'chevronRight');
  $('#step1').innerHTML = i18n.t(left ? 'welcome.step1.left' : 'welcome.step1.right');
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
  $('#progress').classList.toggle('on', !cover && load === 'loading' && s.firstShown[s.active]);
  $('#settings').hidden = !settingsOn;
  $('#welcome').hidden = !welcome;
  $('#error').hidden = cover || load !== 'error';
  $('#loading').hidden = cover || load === 'error' || s.firstShown[s.active];
  if (settingsOn && window.renderSettings) window.renderSettings(s.settings);

  document.body.classList.toggle('with-banner', !!s.banner);
  $('#banner').hidden = !s.banner;

  $('#start').textContent = t(s.onboarded ? 'welcome.back' : 'welcome.start');
  $('#autostart-row').hidden = s.onboarded || !s.canAutostart;
  const step2 = s.hotkey ? 'welcome.step2' : 'welcome.step2none';
  if ($('#step2').dataset.key !== step2 || !$('#step2').firstChild) {
    $('#step2').dataset.key = step2;
    $('#step2').innerHTML = t(step2, i18n.vars);
  }
  renderHotkey(s.hotkey);
}

$('#reload').addEventListener('click', () => chatdock.send('panel:reload'));
$('#pin').addEventListener('click', () => chatdock.send('panel:pin'));
$('#hide').addEventListener('click', () => chatdock.send('panel:hide'));
$('#zoom').addEventListener('click', () => chatdock.send('panel:zoom-reset'));
$('#retry').addEventListener('click', () => chatdock.send('panel:retry'));
$('#banner-close').addEventListener('click', () => chatdock.send('banner:dismiss'));
$('#settings-btn').addEventListener('click', () => chatdock.send('panel:settings'));
$('#update').addEventListener('click', () => chatdock.send('panel:update'));
$('#start').addEventListener('click', () => {
  chatdock.send('onboarding:done', { autostart: $('#autostart').checked });
});
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
