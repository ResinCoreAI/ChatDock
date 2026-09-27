'use strict';

const pill = document.getElementById('pill');
const appsEl = document.getElementById('apps');
const chev = pill.querySelector('.chev');
let shownAt = 0;
let appsKey = '';
let side = '';

function render(s) {
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
      btn.title = `เปิด ${a.name}`;
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
    const badge = appsEl.querySelector(`[data-app="${a.id}"] .badge`);
    if (!badge) continue;
    const n = s.counts[a.id] || 0;
    badge.textContent = n > 9 ? '9+' : String(n);
    badge.hidden = n === 0;
  }
}

chatdock.on('state', render);

chatdock.on('tab:show', (s) => {
  render(s);
  pill.classList.remove('instant', 'in');
  void pill.offsetWidth; // restart the slide-in transition
  pill.classList.add('in');
  shownAt = performance.now();
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
  const btn = e.target.closest('.app');
  chatdock.send('tab:open', btn ? btn.dataset.app : null);
});

chatdock.send('ui:ready');
