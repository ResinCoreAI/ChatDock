'use strict';

const pill = document.getElementById('pill');
const appsEl = document.getElementById('apps');
const callsEl = document.getElementById('calls');
const chev = pill.querySelector('.chev');
let shownAt = 0;
let appsKey = '';
let callsKey = '';
let side = '';
let lang = '';

function render(s) {
  if (s.lang !== lang) {
    lang = s.lang;
    i18n.set(lang);
    i18n.apply();
  }
  if (s.side !== side) { // mirror for the left edge; the arrow points into the screen
    side = s.side;
    pill.classList.add('instant'); // jump to the other side's tucked-away spot, no sliding across
    document.body.classList.toggle('left', side === 'left');
    void pill.offsetWidth;
    pill.classList.remove('instant');
    chev.innerHTML = window.iconHTML(side === 'left' ? 'chevronRight' : 'chevronLeft');
  }
  const key = s.apps.map((a) => a.id).join(',');
  if (key !== appsKey) { // rebuild only when the set of apps changes
    appsKey = key;
    appsEl.textContent = '';
    for (const a of s.apps) {
      const btn = document.createElement('button');
      btn.className = 'app';
      btn.dataset.app = a.id;
      const ico = document.createElement('span');
      ico.innerHTML = window.iconHTML(a.icon);
      const badge = document.createElement('span');
      badge.className = 'badge';
      badge.hidden = true;
      btn.append(ico, badge);
      appsEl.append(btn);
    }
  }
  for (const a of s.apps) {
    const btn = appsEl.querySelector(`[data-app="${a.id}"]`);
    if (!btn) continue;
    btn.classList.toggle('asleep', !!a.asleep); // RAM saver: dimmed until opened
    btn.title = i18n.t(a.asleep ? 'tab.sleeping' : 'tab.openApp', { name: a.name });
    const badge = btn.querySelector('.badge');
    const n = s.counts[a.id] || 0;
    if (n > Number(badge.dataset.n || 0) && pill.classList.contains('in')) bump(badge); // a new one while it's out
    badge.dataset.n = String(n);
    badge.textContent = n > 9 ? '9+' : String(n);
    badge.hidden = n === 0;
  }
  // In a call: a phone; sharing the screen: a screen. Each with its app's icon, and opens that app.
  const calls = s.calls || [];
  const callKey = calls.map((c) => `${c.app}:${c.kind}`).join(',') + '|' + lang;
  if (callKey !== callsKey) {
    callsKey = callKey;
    callsEl.textContent = '';
    for (const c of calls) {
      const app = s.apps.find((a) => a.id === c.app);
      const btn = document.createElement('button');
      btn.className = `chip ${c.kind}`;
      btn.dataset.app = c.app;
      btn.title = i18n.t(c.kind === 'share' ? 'tab.sharing' : 'tab.inCall', { name: app ? app.name : c.app });
      btn.setAttribute('aria-label', btn.title);
      const glyph = document.createElement('span');
      glyph.className = 'glyph';
      glyph.innerHTML = window.iconHTML(c.kind === 'share' ? 'monitor' : 'phone');
      const from = document.createElement('span');
      from.className = 'from';
      from.innerHTML = app ? window.iconHTML(app.icon) : '';
      btn.append(glyph, from);
      callsEl.append(btn);
    }
    callsEl.hidden = calls.length === 0;
  }
}

// An unread number pops once: as the tab slides out, and when it goes up while the tab is out.
function bump(badge) {
  badge.classList.remove('bump');
  void badge.offsetWidth;
  badge.classList.add('bump');
  badge.addEventListener('animationend', () => badge.classList.remove('bump'), { once: true });
}

chatdock.on('state', render);

chatdock.on('tab:show', (s) => {
  render(s);
  pill.classList.remove('instant', 'in');
  void pill.offsetWidth; // restart the slide-in transition
  pill.classList.add('in');
  shownAt = performance.now();
  for (const badge of appsEl.querySelectorAll('.badge:not([hidden])')) bump(badge);
});

chatdock.on('tab:hide', (instant) => {
  pill.classList.toggle('instant', !!instant);
  pill.classList.remove('in');
});

document.addEventListener('click', (e) => {
  // Ignore a click that was already on its way when the tab popped up under the cursor.
  if (performance.now() - shownAt < 180) {
    chatdock.send('tab:log', `click ignored (${Math.round(performance.now() - shownAt)}ms after show)`);
    return;
  }
  const btn = e.target.closest('.app, .chip');
  chatdock.send('tab:open', btn ? btn.dataset.app : null, btn && btn.classList.contains('chip') ? 'call' : '');
});

chatdock.send('ui:ready');
