'use strict';

// ChatDock's menu (menu.rs): rows come in, the size goes back, a pick or Esc goes back. Arrow keys
// move between the rows, Enter picks; the window closing on a click elsewhere is ChatDock's part.
const menu = document.getElementById('menu');
let rows = [];
let at = -1;

function mark(i) {
  rows.forEach((r, n) => r.classList.toggle('at', n === i));
  at = i;
  if (rows[i]) rows[i].focus({ preventScroll: true });
}

function pick(row) {
  if (!row || row.getAttribute('aria-disabled') === 'true') return;
  chatdock.send('menu:pick', row.dataset.id);
}

chatdock.on('menu:show', ({ items }) => {
  menu.textContent = '';
  menu.classList.remove('in');
  rows = [];
  at = -1;
  (items || []).forEach((it) => {
    if (it.sep) {
      menu.append(Object.assign(document.createElement('div'), { className: 'sep' }));
      return;
    }
    const row = document.createElement('button');
    row.type = 'button';
    row.className = `item${it.danger ? ' danger' : ''}${/^(instagram|facebook|x|discord|telegram|whatsapp|spotify)/i.test(it.icon || '') ? ' app' : ''}`;
    row.dataset.id = it.id;
    row.setAttribute('role', 'menuitem');
    row.style.animationDelay = `${rows.length * 18}ms`;
    if (it.disabled) row.setAttribute('aria-disabled', 'true');
    const ico = document.createElement('span');
    ico.className = 'ico';
    ico.innerHTML = it.icon ? window.iconHTML(it.icon) : '';
    const label = document.createElement('span');
    label.className = 'label';
    label.textContent = it.label || '';
    row.append(ico, label);
    if (it.hint) {
      const hint = document.createElement('span');
      hint.className = 'hint';
      hint.textContent = it.hint;
      row.append(hint);
      row.title = it.hint;
    }
    row.addEventListener('click', () => pick(row));
    row.addEventListener('mousemove', () => at !== rows.indexOf(row) && mark(rows.indexOf(row)));
    rows.push(row);
    menu.append(row);
  });
  void menu.offsetWidth;
  menu.classList.add('in');
  const r = menu.getBoundingClientRect();
  // (the window keeps room around the card for its shadow)
  chatdock.send('menu:size', Math.ceil(r.width + 20), Math.ceil(r.height + 20));
});

chatdock.on('menu:hide', () => {
  menu.classList.remove('in');
  rows.forEach((r) => r.classList.remove('at'));
});

document.addEventListener('keydown', (e) => {
  const live = rows.filter((r) => r.getAttribute('aria-disabled') !== 'true');
  if (e.key === 'Escape') {
    chatdock.send('menu:close');
  } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    e.preventDefault();
    if (!live.length) return;
    const now = live.indexOf(rows[at]);
    const next = e.key === 'ArrowDown' ? (now + 1) % live.length : (now - 1 + live.length) % live.length;
    mark(rows.indexOf(live[next < 0 ? 0 : next]));
  } else if (e.key === 'Enter' || e.key === ' ') {
    e.preventDefault();
    pick(rows[at]);
  }
});

document.addEventListener('contextmenu', (e) => e.preventDefault());

chatdock.send('ui:ready');
