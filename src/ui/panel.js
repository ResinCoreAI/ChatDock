'use strict';

const $ = (sel) => document.querySelector(sel);
const nav = $('.apps');
let tabsKey = '';
let side = '';

// One tab per switched-on app: the active one shows its name, the others just the icon + unread count.
function renderTabs(s) {
  const key = s.apps.map((a) => a.id).join(',');
  if (key !== tabsKey) {
    tabsKey = key;
    nav.textContent = '';
    s.apps.forEach((a, i) => {
      const btn = document.createElement('button');
      btn.className = 'app';
      btn.dataset.app = a.id;
      btn.setAttribute('role', 'tab');
      btn.title = i < 9 ? `${a.name} (Ctrl+${i + 1})` : a.name;
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
    });
  }
  for (const btn of nav.children) {
    const id = btn.dataset.app;
    const active = id === s.active;
    btn.classList.toggle('active', active);
    btn.setAttribute('aria-selected', String(active));
    const n = s.counts[id] || 0;
    const badge = btn.querySelector('.badge');
    badge.textContent = n > 99 ? '99+' : String(n);
    badge.hidden = n === 0;
  }
}

function renderHotkey(label) {
  for (const el of document.querySelectorAll('.hotkey-keys')) {
    el.textContent = '';
    if (!label) {
      el.textContent = '(ยังไม่ได้ตั้ง — เลือกได้ในตั้งค่า)';
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

// Docked on the left: the page mirrors (grip on the right, hide arrow points left).
function renderSide(s) {
  if (s.side === side) return;
  side = s.side;
  const left = side === 'left';
  document.body.classList.toggle('left', left);
  $('#hide .hide-ico').innerHTML = window.iconHTML(left ? 'chevronLeft' : 'chevronRight');
  for (const el of document.querySelectorAll('.side-word')) el.textContent = left ? 'ซ้าย' : 'ขวา';
}

function renderUpdate(u) {
  const chip = $('#update');
  const ready = u && u.status === 'ready';
  const available = u && u.status === 'available';
  chip.hidden = !(ready || available);
  if (chip.hidden) return;
  chip.querySelector('.label').textContent = ready ? `อัปเดต ${u.version}` : 'มีอัปเดต';
  chip.title = ready
    ? `ChatDock ${u.version} ดาวน์โหลดแล้ว — คลิกเพื่อติดตั้ง (ใช้เวลาไม่กี่วินาที แล้วเปิดขึ้นมาเอง)`
    : `มี ChatDock ${u.version} — คลิกเพื่อดูรายละเอียด`;
}

function render(s) {
  renderSide(s);
  renderTabs(s);
  renderUpdate(s.update);
  $('#settings-btn').classList.toggle('on', !!s.settingsOpen);
  $('#settings-btn').title = s.settingsOpen ? 'ปิดตั้งค่า กลับไปที่แชท' : 'ตั้งค่า';

  const pin = $('#pin');
  pin.classList.toggle('on', s.pinned);
  pin.title = s.pinned
    ? 'ปักหมุดอยู่ — แชทจะค้างไว้แม้คลิกที่อื่น (คลิกเพื่อเลิกปักหมุด)'
    : 'ปักหมุด — ให้แชทค้างไว้แม้คลิกที่อื่น';
  $('#hide').title = s.hotkey ? `ซ่อนแชท (${s.hotkey} หรือ Esc 2 ครั้ง)` : 'ซ่อนแชท (Esc 2 ครั้ง)';

  const zoom = Math.round((s.zoom || 1) * 100);
  $('#zoom').hidden = zoom === 100;
  $('#zoom').textContent = `${zoom}%`;

  const activeApp = s.apps.find((a) => a.id === s.active);
  for (const el of document.querySelectorAll('.app-name')) el.textContent = activeApp ? activeApp.name : '';

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

  $('#start').textContent = s.onboarded ? 'กลับไปที่แชท' : 'เริ่มใช้งาน — ล็อกอินแอปแชท';
  $('#autostart-row').hidden = s.onboarded || !s.canAutostart;
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

// Drag the inner edge to resize. We send where that edge should be (screen DIP) once per frame:
// the panel's left edge when docked right, its right edge when docked left.
const grip = $('#grip');
let dragging = false;
let grabOffset = 0;
let pendingEdge = null;
grip.addEventListener('pointerdown', (e) => {
  if (e.button !== 0) return;
  dragging = true;
  grabOffset = side === 'left' ? window.innerWidth - e.clientX : e.clientX;
  grip.setPointerCapture(e.pointerId);
  document.body.classList.add('resizing');
});
grip.addEventListener('pointermove', (e) => {
  if (!dragging) return;
  if (pendingEdge === null) {
    requestAnimationFrame(() => {
      chatdock.send('panel:resize', pendingEdge);
      pendingEdge = null;
    });
  }
  pendingEdge = side === 'left' ? e.screenX + grabOffset : e.screenX - grabOffset;
});
const endDrag = () => {
  dragging = false;
  document.body.classList.remove('resizing');
};
grip.addEventListener('pointerup', endDrag);
grip.addEventListener('pointercancel', endDrag);

chatdock.on('state', render);
chatdock.send('ui:ready');
