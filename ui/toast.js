'use strict';

// Renders the pop-up stack. All site-provided text goes in with textContent (never as HTML).
// Cards slide in, slide away when they close, and the others glide into their new places
// (transform and opacity only, so it all runs at the screen's refresh rate).

const stack = document.getElementById('stack');
const wrap = document.getElementById('wrap');
const LEAVE_MS = 240;
const MOVE_MS = 300;
const EASE_LEAVE = 'cubic-bezier(.4, 0, 1, 1)';
const EASE_MOVE = 'cubic-bezier(.2, .8, .2, 1)';
let labels = { more: '', close: '' }; // translated by the main process
let bottom = false; // bottom corners: the stack grows upwards from the window's bottom edge
let shown = false; // the window is on screen (animations only make sense then)
let sizedTo = 0; // height the window was last given
let shrinkTimer = null;
const moreEl = document.createElement('div');
moreEl.className = 'more';
moreEl.hidden = true;
moreEl.addEventListener('click', () => chatdock.send('toast:more'));

function appIcon(name) {
  const span = document.createElement('span');
  span.innerHTML = window.iconHTML(window.ICONS[name] ? name : 'logo'); // our own static SVGs
  return span;
}

// The sender's picture with the app's icon on it, or just the app's icon.
function avatarOf(iconName, picture) {
  const avatar = document.createElement('div');
  avatar.className = 'avatar';
  const big = document.createElement('div');
  big.className = 'big';
  big.append(appIcon(iconName));
  if (picture) {
    const img = document.createElement('img');
    const badge = document.createElement('span');
    img.alt = '';
    img.referrerPolicy = 'no-referrer';
    img.src = picture;
    img.addEventListener('error', () => { // picture didn't load: fall back to the app icon
      img.replaceWith(big);
      badge.remove();
    });
    badge.className = 'app-badge';
    badge.append(appIcon(iconName));
    avatar.append(img, badge);
  } else {
    avatar.append(big);
  }
  return avatar;
}

function closeButton(it) {
  const close = document.createElement('button');
  close.className = 'close';
  close.title = labels.close;
  close.textContent = '✕';
  close.addEventListener('click', (e) => {
    e.stopPropagation();
    chatdock.send('toast:dismiss', it.key);
  });
  return close;
}

// The card after a game (game mode): who wrote meanwhile, a row each; a row opens that chat.
function createSummary(it) {
  const card = document.createElement('div');
  card.className = 'card enter summary';
  card.dataset.key = it.key;
  card.style.setProperty('--accent', it.accent || '#8b5cf6');
  const head = document.createElement('div');
  head.className = 'sum-head';
  const logo = appIcon('logo');
  logo.className = 'sum-logo';
  const text = document.createElement('div');
  text.className = 'text';
  const title = document.createElement('div');
  title.className = 'title';
  title.textContent = it.title;
  const meta = document.createElement('div');
  meta.className = 'meta';
  meta.textContent = it.meta;
  text.append(title, meta);
  head.append(logo, text);
  const rows = document.createElement('div');
  rows.className = 'rows';
  card.append(head, rows, closeButton(it));
  fillRows(card, it);
  card.addEventListener('click', () => chatdock.send('toast:click', it.key));
  card.addEventListener('animationend', () => card.classList.remove('enter'), { once: true });
  return card;
}

// (again when a row has gone: opened, or read meanwhile)
function fillRows(card, it) {
  const box = card.querySelector('.rows');
  const sig = JSON.stringify([it.rows.map((r) => [r.appId, r.who, r.text, r.n]), it.moreRows, it.meta]);
  if (box.dataset.sig === sig) return;
  box.dataset.sig = sig;
  card.querySelector('.sum-head .meta').textContent = it.meta;
  box.replaceChildren(...it.rows.map((r) => {
    const row = document.createElement('div');
    row.className = 'row';
    const words = document.createElement('div');
    words.className = 'rt';
    const who = document.createElement('b');
    who.textContent = r.who || r.appName;
    if (r.meta) {
      const where = document.createElement('small');
      where.textContent = ` · ${r.meta}`;
      who.append(where);
    }
    const said = document.createElement('span');
    said.textContent = r.text;
    words.append(who, said);
    const n = document.createElement('i');
    n.className = 'n';
    n.textContent = String(r.n);
    n.hidden = !(r.n > 1);
    row.append(avatarOf(r.iconName, r.icon), words, n);
    row.addEventListener('click', (e) => {
      e.stopPropagation();
      chatdock.send('toast:row', it.key, r.tag); // the conversation, not its place in the card
    });
    return row;
  }));
  if (it.moreRows > 0) {
    const more = document.createElement('div');
    more.className = 'rows-more';
    more.textContent = it.moreLabel;
    box.append(more);
  }
}

function createCard(it) {
  if (it.rows && it.rows.length) return createSummary(it);
  const card = document.createElement('div');
  card.className = 'card enter';
  card.dataset.key = it.key;
  card.style.setProperty('--accent', it.accent || '#0866ff');

  const avatar = avatarOf(it.iconName, it.icon);

  const text = document.createElement('div');
  text.className = 'text';
  const meta = document.createElement('div');
  meta.className = 'meta';
  meta.textContent = it.meta ? `${it.appName} · ${it.meta}` : it.appName;
  const title = document.createElement('div');
  title.className = 'title';
  title.textContent = it.title;
  if (it.vip) {
    const tag = document.createElement('span');
    tag.className = 'vip';
    tag.textContent = `★ ${it.vipLabel}`;
    title.append(tag);
  }
  text.append(meta, title);
  if (it.body) {
    const body = document.createElement('div');
    body.className = 'body';
    body.textContent = it.body;
    text.append(body);
  }
  const hint = document.createElement('div');
  hint.className = 'hint';
  hint.textContent = it.hint || '';
  if (it.hint) text.append(hint);
  if (it.quick && it.quick.length) text.append(quickChips(it));

  card.append(avatar, text, closeButton(it));
  if (it.star) {
    // the sender as one of the important people (their pop-ups come through game mode)
    const star = document.createElement('button');
    star.className = `star${it.vip ? ' on' : ''}`;
    star.title = it.star;
    star.textContent = it.vip ? '★' : '☆';
    star.addEventListener('click', (e) => {
      e.stopPropagation();
      chatdock.send('toast:star', it.key);
    });
    card.append(star);
  }
  card.addEventListener('click', () => chatdock.send('toast:click', it.key));
  card.addEventListener('animationend', () => card.classList.remove('enter'), { once: true });
  return card;
}

// One-click replies under the message. A click sends that text into the conversation without
// opening anything (the game keeps the keyboard); the chips then say how it went.
function quickChips(it) {
  const row = document.createElement('div');
  row.className = 'chips';
  it.quick.forEach((q, i) => {
    const b = document.createElement('button');
    b.className = 'qchip';
    b.textContent = q;
    b.addEventListener('click', (e) => {
      e.stopPropagation(); // not the card's own click (that opens the chat)
      if (row.classList.contains('busy')) return;
      row.classList.add('busy');
      chatdock.send('toast:quick', it.key, i);
    });
    row.append(b);
  });
  if (it.replyLabel) {
    const r = document.createElement('button');
    r.className = 'qchip reply';
    r.textContent = it.replyLabel;
    r.addEventListener('click', (e) => {
      e.stopPropagation();
      if (!row.classList.contains('busy')) chatdock.send('toast:reply-open', it.key);
    });
    row.append(r);
  }
  row.card = it; // (to put the chips back after the reply box)
  return row;
}

// The reply box: the pop-ups have the keyboard while it is open. Enter sends, Shift+Enter is a new
// line, Esc closes it (the main process gives the keyboard back to the game either way).
chatdock.on('toast:reply-box', ({ key, hint }) => {
  const card = stack.querySelector(`.card[data-key="${CSS.escape(key)}"]:not(.leaving)`);
  const row = card && card.querySelector('.chips');
  if (!row) return;
  row.textContent = '';
  const box = document.createElement('textarea');
  box.className = 'reply-box';
  box.rows = 2;
  box.maxLength = 2000;
  const tip = document.createElement('span');
  tip.className = 'reply-tip';
  tip.textContent = hint;
  let alive = 0;
  for (const ev of ['click', 'mousedown']) box.addEventListener(ev, (e) => e.stopPropagation());
  box.addEventListener('input', () => {
    if (Date.now() - alive > 5000) {
      alive = Date.now();
      chatdock.send('toast:reply-alive');
    }
  });
  box.addEventListener('keydown', (e) => {
    if (e.isComposing || e.keyCode === 229) return; // Thai and other input methods choosing a character
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      if (box.value.trim()) chatdock.send('toast:reply-send', key, box.value);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      chatdock.send('toast:reply-cancel', key);
    }
  });
  row.append(box, tip);
  reportSize();
  box.focus();
});

// The box closed: the quick replies come back (a reply that was sent then says how it went).
chatdock.on('toast:reply-closed', ({ key }) => {
  const card = stack.querySelector(`.card[data-key="${CSS.escape(key)}"]:not(.leaving)`);
  const row = card && card.querySelector('.chips');
  if (!row || !row.querySelector('textarea')) return;
  row.replaceWith(quickChips(row.card));
  reportSize();
});

// How a reply went: sending (the chips wait), sent (one green chip), or not sent (why, and a way
// into the chat).
chatdock.on('toast:reply-state', ({ key, state, text, line, open }) => {
  const card = stack.querySelector(`.card[data-key="${CSS.escape(key)}"]:not(.leaving)`);
  const row = card && card.querySelector('.chips');
  if (!row) return;
  if (state === 'sending') {
    row.classList.add('busy');
    return;
  }
  row.classList.remove('busy');
  row.textContent = '';
  if (state === 'sent') {
    const done = document.createElement('span');
    done.className = 'qchip sent';
    done.textContent = `✓ ${text}`;
    row.append(done);
  } else {
    const why = document.createElement('span');
    why.className = 'reply-why';
    why.textContent = line;
    const b = document.createElement('button');
    b.className = 'qchip';
    b.textContent = open;
    b.addEventListener('click', (e) => {
      e.stopPropagation();
      chatdock.send('toast:click', key);
    });
    row.append(why, b);
  }
  reportSize();
});

// A card already on screen whose sender was starred (or no longer is): its tag and its star follow.
function syncVip(card, it) {
  const title = card.querySelector('.title');
  const tag = title && title.querySelector('.vip');
  if (it.vip && !tag && title) {
    const t = document.createElement('span');
    t.className = 'vip';
    t.textContent = `★ ${it.vipLabel}`;
    title.append(t);
  } else if (!it.vip && tag) {
    tag.remove();
  }
  const star = card.querySelector('.star');
  if (star) {
    star.classList.toggle('on', !!it.vip);
    star.textContent = it.vip ? '★' : '☆';
    star.title = it.star || '';
  }
}

const liveCards = () => [...stack.querySelectorAll('.card:not(.leaving)')];

// Distance from the edge the stack grows away from. It stays put on screen while the window
// changes size, so old and new places can be compared.
function anchorPos(el) {
  return bottom ? stack.offsetHeight - el.offsetTop - el.offsetHeight : el.offsetTop;
}

// No cards left at all (the last one has finished sliding away): the window can go.
function settle() {
  if (stack.querySelector('.card')) return;
  sizedTo = 0;
  chatdock.send('toast:empty');
}

// A card closes: it slides away where it was (pos, measured before anything moved), while the
// others close the gap.
function leave(el, pos) {
  if (!shown || pos === undefined) {
    el.remove();
    return;
  }
  for (const a of el.getAnimations()) a.cancel();
  el.classList.add('leaving');
  el.style.position = 'absolute';
  el.style.left = '0';
  el.style[bottom ? 'bottom' : 'top'] = `${pos}px`;
  const dir = document.body.classList.contains('left') ? -1 : 1;
  el.animate([
    { transform: 'none', opacity: 1 },
    { transform: `translateX(${dir * 45}%) scale(.97)`, opacity: 0 },
  ], { duration: LEAVE_MS, easing: EASE_LEAVE, fill: 'forwards' }).onfinish = () => {
    el.remove();
    settle();
  };
}

function render(state) {
  labels = state.labels || labels;
  shown = !!state.shown;
  if (state.lang) document.documentElement.lang = state.lang;
  const pos = String(state.position || 'top-right');
  bottom = pos.startsWith('bottom');
  document.body.classList.toggle('bottom', bottom);
  document.body.classList.toggle('left', pos.endsWith('left')); // cards slide in from the left edge
  if (!shown) for (const el of stack.querySelectorAll('.card.leaving')) el.remove();

  const before = new Map(); // where each card is now
  if (shown) for (const el of liveCards()) before.set(el.dataset.key, anchorPos(el));

  const keep = new Set(state.items.map((it) => it.key));
  for (const el of liveCards()) if (!keep.has(el.dataset.key)) leave(el, before.get(el.dataset.key));
  for (const b of stack.querySelectorAll('.close')) b.title = labels.close;
  state.items.forEach((it, i) => {
    const el = stack.querySelector(`.card[data-key="${it.key}"]:not(.leaving)`) || createCard(it);
    if (el.classList.contains('summary') && it.rows) fillRows(el, it);
    else syncVip(el, it);
    const cards = liveCards();
    if (cards[i] !== el) stack.insertBefore(el, cards[i] || moreEl);
  });
  moreEl.textContent = labels.more;
  moreEl.hidden = !(state.more > 0);
  stack.append(moreEl); // always last

  // Cards that changed place glide there from where they were (added on top of a slide-in).
  for (const el of liveCards()) {
    const was = before.get(el.dataset.key);
    if (was === undefined) continue;
    const moved = was - anchorPos(el);
    if (Math.abs(moved) < 1) continue;
    el.animate([{ transform: `translateY(${bottom ? -moved : moved}px)` }, { transform: 'translateY(0)' }],
      { duration: MOVE_MS, easing: EASE_MOVE, composite: 'add' });
  }
  if (!state.items.length && !stack.querySelector('.card')) settle();
}

// Measured right away (layout is synchronous); no requestAnimationFrame, which never fires while
// the window is still hidden. The main process shows the window once it knows the height. Growing
// happens at once; shrinking waits until the cards have finished sliding.
function reportSize() {
  const h = wrap.offsetHeight;
  clearTimeout(shrinkTimer);
  if (h >= sizedTo || !shown) {
    sizedTo = h;
    chatdock.send('toast:size', h);
    return;
  }
  shrinkTimer = setTimeout(() => {
    sizedTo = wrap.offsetHeight;
    chatdock.send('toast:size', sizedTo);
  }, Math.max(LEAVE_MS, MOVE_MS) + 40);
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
  if (state.items.length) reportSize();
  if (state.chime) chime();
});

stack.append(moreEl);

chatdock.send('ui:ready');
