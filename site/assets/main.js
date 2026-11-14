// The home page: the demo screen, the latest release, and the clay toys around them.
import { initCommon, onLang, t, getLang, allReleases, team, RELEASES, REPO } from './common.js';
import { installer, formatSize, versionOf, people, latestStable, summaryLine, parseRelease } from './lib.js';
import { mountScreen } from './screen.js';
import { ICONS } from './icons.js';

const still = matchMedia('(prefers-reduced-motion: reduce)').matches;
const touch = matchMedia('(hover: none)').matches;
const $ = (q) => document.querySelector(q);
const $$ = (q) => [...document.querySelectorAll(q)];

let screen = null;
let releases = null; // newest first, from GitHub
let stickerStage = 0;
let crew = null; // the people from GitHub, once they arrive

// ---- the people who made it, from their GitHub profiles (shown as text, never as HTML) ----
const GH_MARK = document.querySelector('.gh-link svg')?.outerHTML || '<svg viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M12 2a10 10 0 0 0-3.16 19.49c.5.09.68-.22.68-.48v-1.7c-2.78.6-3.37-1.34-3.37-1.34-.45-1.16-1.11-1.47-1.11-1.47-.9-.62.07-.6.07-.6 1 .07 1.53 1.03 1.53 1.03.89 1.52 2.34 1.08 2.91.83.09-.65.35-1.08.63-1.33-2.22-.25-4.56-1.11-4.56-4.95 0-1.09.39-1.98 1.03-2.68-.1-.25-.45-1.27.1-2.65 0 0 .84-.27 2.75 1.02a9.5 9.5 0 0 1 5 0c1.91-1.29 2.75-1.02 2.75-1.02.55 1.38.2 2.4.1 2.65.64.7 1.03 1.59 1.03 2.68 0 3.85-2.34 4.7-4.57 4.94.36.31.68.92.68 1.85v2.74c0 .27.18.58.69.48A10 10 0 0 0 12 2z"/></svg>';
function renderTeam() {
  const box = $('#team-list');
  box.textContent = '';
  // no people (GitHub didn't answer): the section and its menu link stay out of sight
  const none = !crew || !crew.length;
  $('#team').hidden = none;
  $$('a[href$="#team"]').forEach((a) => { a.hidden = none; });
  if (none) return;
  crew.forEach((m) => {
    const card = document.createElement('article');
    card.className = 'person puff';
    const face = document.createElement('div');
    face.className = 'face';
    const initial = document.createElement('span');
    initial.textContent = [...m.name][0].toUpperCase();
    if (m.avatar) {
      const img = new Image(96, 96);
      img.alt = '';
      img.loading = 'lazy';
      img.decoding = 'async';
      img.src = m.avatar;
      img.onerror = () => img.replaceWith(initial); // a blocked picture falls back to the first letter
      face.append(img);
    } else face.append(initial);
    const name = document.createElement('h3');
    name.textContent = m.name;
    const login = document.createElement('div');
    login.className = 'login';
    login.textContent = '@' + m.login;
    card.append(face, name, login);
    if (m.role) { const r = document.createElement('div'); r.className = 'role'; r.textContent = m.role; card.append(r); }
    if (m.about) { const ab = document.createElement('div'); ab.className = 'about'; ab.textContent = m.about; card.append(ab); }
    const c = document.createElement('div');
    c.className = 'commits';
    c.textContent = t('team_commits').replace('{n}', m.commits.toLocaleString(getLang()));
    const gh = document.createElement('a');
    gh.className = 'gh';
    gh.href = m.url;
    gh.rel = 'noopener';
    gh.innerHTML = GH_MARK;
    gh.append(document.createTextNode(t('team_profile')));
    const grow = document.createElement('div');
    grow.className = 'grow'; // keeps the commit count and the button on the same line in every card
    card.append(grow, c, gh);
    box.append(card);
  });
}

// ---- text that depends on the language ----
function splitHeadline() {
  const h = $('h1[data-t]');
  h.innerHTML = h.innerHTML.split(/<br\s*\/?>/i).map((l, i) => `<span class="ln" style="animation-delay:${(0.12 + i * 0.1).toFixed(2)}s">${l}</span>`).join('<br>');
}

function renderLog() {
  const ol = $('#log');
  const small = $('#notes-lang');
  ol.textContent = '';
  const list = (releases || []).filter((r) => !r.draft).slice(0, 6);
  small.textContent = list.length ? t('notes_en') : '';
  small.hidden = !small.textContent;
  // no releases (GitHub didn't answer): the whole log stays out of sight
  $('#news').hidden = !list.length;
  if (!list.length) return;
  const date = new Intl.DateTimeFormat(getLang(), { year: 'numeric', month: 'short', day: 'numeric' });
  const make = (tag, cls, text) => { const n = document.createElement(tag); if (cls) n.className = cls; if (text != null) n.textContent = text; return n; };
  list.forEach((r, i) => {
    const li = make('li');
    li.append(make('i', 'dot'));
    const d = make('details', 'puff');
    d.open = i === 0;
    const sum = make('summary');
    const ver = make('span', 'ver', versionOf(r.tag_name));
    if (i === 0 && !r.prerelease) ver.append(make('span', 'badge', t('wn_latest')));
    if (r.published_at) { const tm = make('time', null, date.format(new Date(r.published_at))); tm.dateTime = r.published_at; ver.append(tm); }
    sum.append(ver, make('span', 'sum', summaryLine(r.body, 3) || r.name || ''));
    d.append(sum);
    const body = parseRelease(r.body);
    body.sections.forEach((sec) => {
      if (sec.title) d.append(make('h4', null, sec.title));
      const ul = make('ul');
      sec.items.forEach((it) => {
        const item = make('li');
        if (it.h) item.append(make('b', null, it.h));
        item.append(document.createTextNode(it.p));
        ul.append(item);
      });
      d.append(ul);
    });
    body.paras.forEach((p) => d.append(make('p', 'rp', p)));
    const ins = installer(r.assets);
    const a = make('a', 'dl-ver', ins ? `${t('wn_download')} · ${formatSize(ins.size)} ↓` : t('wn_github'));
    a.href = ins ? ins.url : (r.html_url || RELEASES);
    d.append(a);
    li.append(d);
    ol.append(li);
  });
}

onLang((l) => {
  splitHeadline();
  $('#tip').textContent = t(touch ? 'tip_touch' : 'tip_mouse');
  setSticker(stickerStage, true);
  renderLog();
  renderTeam();
  screen?.setLang(l);
  $$('a[href^="whats-new.html"]').forEach((a) => { a.href = `whats-new.html?lang=${l}`; });
});

// ---- the opening ----
initCommon();
setTimeout(() => document.documentElement.classList.remove('intro'), 2400);
$$('[data-icon]').forEach((el) => { el.innerHTML = ICONS[el.dataset.icon] || ''; });

// ---- releases: the newest full one sets the version, the download and the size; all feed the log ----
allReleases().then((list) => { releases = list; }).catch(() => { releases = []; }).then(() => {
  const j = latestStable(releases);
  if (j) {
    const v = versionOf(j.tag_name);
    if (v) $$('[data-ver]').forEach((el) => { el.textContent = v; });
    const ins = installer(j.assets);
    if (ins) {
      $$('[data-dl]').forEach((a) => { a.href = ins.url; });
      $$('[data-size]').forEach((el) => { el.textContent = ` · ${formatSize(ins.size)}`; });
    }
  }
  renderLog();
});

team().then(({ list, profiles }) => { crew = people(list, profiles); renderTeam(); }).catch(() => { crew = []; renderTeam(); });

// ---- the sticker answers what the visitor does on the screen ----
function setSticker(n, force) {
  const el = $('#sticker');
  if (n === stickerStage && !force) return;
  const moved = n !== stickerStage;
  stickerStage = n;
  el.textContent = t(['sticker1', 'sticker2', 'sticker3'][n]);
  el.classList.toggle('done', n === 2);
  if (moved && !still) {
    el.animate([{ scale: '.6' }, { scale: '1.15 .9', offset: 0.55 }, { scale: '.97 1.03', offset: 0.8 }, { scale: '1' }],
      { duration: 650, easing: 'cubic-bezier(.3,1.4,.5,1)' });
  }
}

// ---- the demo screen ----
document.fonts.ready.then(() => {
  screen = mountScreen($('#screen'), { font: '"IBM Plex Sans Thai Looped", "Noto Sans SC", "Noto Sans JP", sans-serif', still });
  screen.setLang(getLang());
  if (new URLSearchParams(location.search).has('debug')) window.__screen = screen; // for checking by hand
  const monitor = $('.monitor');
  let wasOpen = false;
  screen.onChange((s) => {
    if (s.open !== wasOpen) {
      wasOpen = s.open;
      // the monitor wobbles like jelly when the panel comes out or goes back
      if (!still) {
        monitor.animate([{ scale: '1 1' }, { scale: '1.022 .975', offset: 0.28 }, { scale: '.988 1.016', offset: 0.55 }, { scale: '1.004 .997', offset: 0.8 }, { scale: '1 1' }],
          { duration: 700, easing: 'ease-out' });
      }
    }
    if (s.demo) return; // only what the visitor does moves the sticker on
    if (s.sent.some((m) => m.me)) setSticker(2);
    else if (s.open && stickerStage === 0) setSticker(1);
  });
  // an app button opens that app on the screen
  $$('[data-app]').forEach((b) => b.addEventListener('click', () => {
    const r = $('#stage').getBoundingClientRect();
    const inView = r.top >= 0 && r.bottom <= innerHeight;
    if (!inView) $('#stage').scrollIntoView({ behavior: still ? 'auto' : 'smooth', block: 'center' });
    setTimeout(() => screen.openApp(b.dataset.app), inView || still ? 0 : 520);
  }));
});

// ---- clay bits: springs follow the pointer a little; drag one and fling it, it bounces home ----
const bits = $$('.bit').map((el) => ({ el, depth: Number(el.dataset.depth), x: 0, y: 0, vx: 0, vy: 0, held: false, gx: 0, gy: 0, last: null }));
let lean = [0, 0];
let running = false;
let tPrev = 0;
function loop(now) {
  const dt = Math.min(0.032, (now - tPrev) / 1000);
  tPrev = now;
  let moving = false;
  for (const b of bits) {
    if (b.held) {
      moving = true;
    } else {
      const tx = lean[0] * b.depth;
      const ty = lean[1] * b.depth;
      b.vx += (170 * (tx - b.x) - 12 * b.vx) * dt;
      b.vy += (170 * (ty - b.y) - 12 * b.vy) * dt;
      b.x += b.vx * dt;
      b.y += b.vy * dt;
      if (Math.abs(tx - b.x) + Math.abs(ty - b.y) + Math.abs(b.vx) + Math.abs(b.vy) > 0.05) moving = true;
    }
    b.el.style.transform = `translate3d(${b.x.toFixed(2)}px, ${b.y.toFixed(2)}px, 0)`;
  }
  if (moving) requestAnimationFrame(loop); else running = false;
}
function wake() {
  if (running || still) return;
  running = true;
  tPrev = performance.now();
  requestAnimationFrame(loop);
}
bits.forEach((b) => {
  b.el.addEventListener('pointerdown', (e) => {
    b.held = true;
    b.gx = e.clientX - b.x;
    b.gy = e.clientY - b.y;
    b.last = { x: e.clientX, y: e.clientY, t: performance.now(), vx: 0, vy: 0 };
    b.el.setPointerCapture(e.pointerId);
    b.el.classList.add('held');
    $('.bits-hint')?.classList.add('gone');
    wake();
  });
  b.el.addEventListener('pointermove', (e) => {
    if (!b.held) return;
    const now = performance.now();
    const dt = Math.max(1, now - b.last.t) / 1000;
    b.last = { x: e.clientX, y: e.clientY, t: now, vx: (e.clientX - b.last.x) / dt, vy: (e.clientY - b.last.y) / dt };
    b.x = e.clientX - b.gx;
    b.y = e.clientY - b.gy;
  });
  const letGo = () => {
    if (!b.held) return;
    b.held = false;
    b.el.classList.remove('held');
    // keep the speed of the throw, then the spring brings it home
    b.vx = Math.max(-1800, Math.min(1800, b.last.vx));
    b.vy = Math.max(-1800, Math.min(1800, b.last.vy));
    wake();
  };
  b.el.addEventListener('pointerup', letGo);
  b.el.addEventListener('pointercancel', letGo);
});
if (!touch) {
  $('.hero').addEventListener('pointermove', (e) => {
    const r = e.currentTarget.getBoundingClientRect();
    lean = [(e.clientX - r.left) / r.width - 0.5, (e.clientY - r.top) / r.height - 0.5];
    wake();
  });
  $('.hero').addEventListener('pointerleave', () => { lean = [0, 0]; wake(); });
}

// ---- clay crumbs out of a download button ----
const colours = ['#ff6b4a', '#ffd166', '#7ed3b2', '#8ecae6', '#f4a6c6', '#c9b6f2'];
$$('[data-dl]').forEach((btn) => btn.addEventListener('pointerdown', () => {
  if (still || !btn.classList.contains('btn')) return;
  const r = btn.getBoundingClientRect();
  for (let i = 0; i < 14; i++) {
    const c = document.createElement('i');
    c.className = 'crumb';
    c.style.background = colours[i % colours.length];
    c.style.left = `${r.left + r.width / 2 - 7}px`;
    c.style.top = `${r.top + r.height / 2 - 7}px`;
    document.body.append(c);
    const a = Math.PI * (1.15 + Math.random() * 0.7);
    const v = 90 + Math.random() * 120;
    const x = Math.cos(a) * v * (i % 2 ? -1 : 1);
    const y = Math.sin(a) * v;
    c.animate([
      { transform: 'translate3d(0,0,0) scale(.4)', opacity: 1 },
      { transform: `translate3d(${x * 0.7}px, ${y}px, 0) scale(1.1, .9)`, opacity: 1, offset: 0.45 },
      { transform: `translate3d(${x}px, ${y + 170}px, 0) scale(.7)`, opacity: 0 },
    ], { duration: 950 + Math.random() * 300, easing: 'cubic-bezier(.2,.7,.4,1)' }).onfinish = () => c.remove();
  }
}));

// ---- sections settle into place as they arrive (they can be read the whole time) ----
if (!still && 'IntersectionObserver' in window) {
  const io = new IntersectionObserver((en) => en.forEach((e) => {
    if (e.isIntersecting) { e.target.classList.remove('below'); io.unobserve(e.target); }
  }), { threshold: 0.12 });
  $$('.settle').forEach((el) => {
    if (el.getBoundingClientRect().top > innerHeight) { el.classList.add('below'); io.observe(el); }
  });
}
