'use strict';

// Renders the pop-up stack. All site-provided text goes in with textContent (never as HTML).

const stack = document.getElementById('stack');
const wrap = document.getElementById('wrap');

function appIcon(name) {
  const span = document.createElement('span');
  span.innerHTML = window.iconHTML(window.ICONS[name] ? name : 'logo'); // our own static SVGs
  return span;
}

function createCard(it) {
  const card = document.createElement('div');
  card.className = 'card enter';
  card.dataset.key = it.key;
  card.style.setProperty('--accent', it.accent || '#0866ff');

  const avatar = document.createElement('div');
  avatar.className = 'avatar';
  const big = document.createElement('div');
  big.className = 'big';
  big.append(appIcon(it.iconName));
  if (it.icon) {
    const img = document.createElement('img');
    const badge = document.createElement('span');
    img.alt = '';
    img.referrerPolicy = 'no-referrer';
    img.src = it.icon;
    img.addEventListener('error', () => { // picture didn't load: fall back to the app icon
      img.replaceWith(big);
      badge.remove();
    });
    badge.className = 'app-badge';
    badge.append(appIcon(it.iconName));
    avatar.append(img, badge);
  } else {
    avatar.append(big);
  }

  const text = document.createElement('div');
  text.className = 'text';
  const meta = document.createElement('div');
  meta.className = 'meta';
  meta.textContent = `${it.appName} · ${it.meta || 'เพิ่งทักมา'}`;
  const title = document.createElement('div');
  title.className = 'title';
  title.textContent = it.title;
  text.append(meta, title);
  if (it.body) {
    const body = document.createElement('div');
    body.className = 'body';
    body.textContent = it.body;
    text.append(body);
  }
  const hint = document.createElement('div');
  hint.className = 'hint';
  hint.textContent = it.hint || 'คลิกเพื่อเปิดแชทนี้';
  text.append(hint);

  const close = document.createElement('button');
  close.className = 'close';
  close.title = 'ปิด';
  close.textContent = '✕';
  close.addEventListener('click', (e) => {
    e.stopPropagation();
    chatdock.send('toast:dismiss', it.key);
  });

  card.append(avatar, text, close);
  card.addEventListener('click', () => chatdock.send('toast:click', it.key));
  card.addEventListener('animationend', () => card.classList.remove('enter'), { once: true });
  return card;
}

function render(state) {
  const pos = String(state.position || 'top-right');
  document.body.classList.toggle('bottom', pos.startsWith('bottom'));
  document.body.classList.toggle('left', pos.endsWith('left')); // cards slide in from the left edge
  const keep = new Set(state.items.map((it) => it.key));
  for (const el of [...stack.children]) {
    if (!el.dataset.key || !keep.has(el.dataset.key)) el.remove();
  }
  state.items.forEach((it, i) => {
    const el = stack.querySelector(`.card[data-key="${it.key}"]`) || createCard(it);
    if (stack.children[i] !== el) stack.insertBefore(el, stack.children[i] || null);
  });
  if (state.more > 0) {
    const more = document.createElement('div');
    more.className = 'more';
    more.textContent = `+ อีก ${state.more} ข้อความ · คลิกเพื่อเปิดแชท`;
    more.addEventListener('click', () => chatdock.send('toast:more'));
    stack.append(more);
  }
}

// Measured right away (layout is synchronous); no requestAnimationFrame, which never fires while
// the window is still hidden. The main process shows the window once it knows the height.
function reportSize() {
  chatdock.send('toast:size', wrap.offsetHeight);
}

function chime() {
  try {
    const ctx = new AudioContext();
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    const t = ctx.currentTime;
    osc.type = 'sine';
    osc.frequency.setValueAtTime(880, t);
    osc.frequency.exponentialRampToValueAtTime(1320, t + 0.12);
    gain.gain.setValueAtTime(0.0001, t);
    gain.gain.exponentialRampToValueAtTime(0.12, t + 0.02);
    gain.gain.exponentialRampToValueAtTime(0.0001, t + 0.38);
    osc.connect(gain).connect(ctx.destination);
    osc.start(t);
    osc.stop(t + 0.4);
    setTimeout(() => ctx.close(), 700);
  } catch {
    // no audio device
  }
}

wrap.addEventListener('mouseenter', () => chatdock.send('toast:hover', true));
wrap.addEventListener('mouseleave', () => chatdock.send('toast:hover', false));

chatdock.on('toasts', (state) => {
  render(state);
  reportSize();
  if (state.chime) chime();
});

chatdock.send('ui:ready');
