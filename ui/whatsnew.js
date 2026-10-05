'use strict';

// "What's new" after an update: "ChatDock is now on Beta Build 1.5.3 ✓", then what changed since the
// version the user had, one section per release. ChatDock sends the texts (already translated),
// sizes the window to the card and shows it. The same window explains a feature when it is switched
// on (kind "intro": game mode), with "don't show this again" where the GitHub link would be.
const card = document.querySelector('.card');
const notes = document.getElementById('notes');
const demoBox = document.getElementById('demo');
const scene = document.getElementById('scene');
const dots = document.getElementById('dots');
const againBox = document.getElementById('again-box');
let closing = false;
let intro = false;

// ---------------------------------------------------------------------------------------------
// Demos: a short animated scene for each line of a release that has them, in the lines' order
// (the same in every language). They take turns; the line being shown lights up, and clicking a
// line or a dot shows its scene. Every animation plays once per showing, and after two rounds the
// turns stop on the last frame: nothing keeps redrawing a window left open for hours.
// ---------------------------------------------------------------------------------------------
const DEMOS = {
  gameMode: ['gmHold', 'gmCard', 'gmOpen'], // game mode's explainer (intro)
  '1.7.4': ['guide', 'fbIcon', 'glide', 'gameMode'],
  '1.7.1': ['spotify', 'volume', 'dcList'],
  '1.7': ['dcPage', 'dcServers', 'dcVoice', 'dcShare', 'dockCalls', 'dcAwake', 'lighter'],
};
const SCENE_MS = 6000;
const ROUNDS = 2;
const still = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
let texts = {};
let demoList = [];
let demoAt = -1;
let demoTimer = 0;
let rounds = 0;
let hovering = false;

const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
const icon = (name) => window.iconHTML(name);
const CURSOR = '<svg class="cur" viewBox="0 0 16 16"><path d="M2 1.5v11.2l3-2.9 2 4.7 2.1-.9-2-4.6h4.2z" fill="#fff" stroke="#111" stroke-width="1" stroke-linejoin="round"/></svg>';
const GAME = '<div class="game"><i></i><i></i><i></i></div>';
const SWITCH = (on, cls = '') => `<i class="sw${on ? ' on' : ''}${cls ? ` ${cls}` : ''}"></i>`;
const MINI = (people) => `<div class="mini"><div class="mh"><i class="ic">${icon('discordMono')}</i><b></b></div>
  <div class="vc">${'<i class="av"></i>'.repeat(people)}</div></div>`;
// Facebook's icon before 1.7.4 (Messenger's), for the scene where it changes; n keeps each copy's
// gradient its own
const MESSENGER = (n) => `<svg viewBox="0 0 24 24"><defs><linearGradient id="ms-old-${n}" x1="0" y1="1" x2="1" y2="0">
  <stop offset="0" stop-color="#0866ff"/><stop offset=".6" stop-color="#a033ff"/><stop offset="1" stop-color="#ff5c87"/></linearGradient></defs>
  <path d="M12 2.2C6.5 2.2 2.2 6.3 2.2 11.7c0 2.8 1.2 5.3 3.1 7v3.4l3.2-1.8c1.1.3 2.3.5 3.5.5 5.5 0 9.8-4.1 9.8-9.5S17.5 2.2 12 2.2z" fill="url(#ms-old-${n})"/>
  <path d="M6.4 14.3l3.5-5.4 2.9 2.2 4.8-2.5-3.5 5.4-2.9-2.2z" fill="#fff"/></svg>`;

// Game mode's explainer: people (from the texts ChatDock sends) and the real pop-up card, small.
const PEOPLE = ['instagram', 'discord', 'facebook'];
const person = (i) => {
  const gm = texts.gm || {};
  return { app: PEOPLE[i], name: (gm.names || [])[i] || '', msg: (gm.msgs || [])[i] || '', grad: `pg${i + 1}` };
};
// a name's first letter for its picture (Thai: past a vowel written before its consonant, เจ → จ)
const initialOf = (name) => {
  const c = Array.from(name || '');
  return (/^[เแโใไ]$/.test(c[0] || '') && c[1] ? c[1] : c[0]) || '';
};
const PAV = (p) => `<span class="pav ${p.grad}">${esc(initialOf(p.name))}<i class="pbd">${icon(p.app)}</i></span>`;
const GAME2 = (cls = '') => `<div class="gm2${cls ? ` ${cls}` : ''}">${GAME}<i class="hp"><b></b></i><i class="mm"></i><i class="xh"></i></div>`;
const WAIT = (cls = '') => `<div class="hold${cls ? ` ${cls}` : ''}"><i class="pz"></i><i class="a a1 pg1"></i><i class="a a2 pg2"></i><i class="a a3 pg3"></i></div>`;
const SUMCARD = (cls, opened = false) => {
  const gm = texts.gm || {};
  // (once the first row is opened, the card counts what is left, as the real one does)
  const count = opened ? `<small class="c4">${esc(gm.count || '')}</small><small class="c2">${esc(gm.countLeft || '')}</small>` : `<small>${esc(gm.count || '')}</small>`;
  const rows = [0, 1, 2].map((i) => {
    const p = person(i);
    const where = i === 1 && gm.where ? `<small> · ${esc(gm.where)}</small>` : '';
    return `<div class="prow r${i + 1}">${PAV(p)}<span class="ptx"><b>${esc(p.name)}${where}</b><span>${esc(p.msg)}</span></span>${i ? '' : '<em>2</em>'}</div>`;
  }).join('');
  return `<div class="pcard ${cls}"><span class="pst"></span>
    <div class="phd"><i class="plg">${icon('logo')}</i><p><b>${esc(gm.title || '')}</b>${count}</p><i class="px">✕</i></div>${rows}</div>`;
};

const SCENES = {
  // game mode, 1: while the game fills the screen, what comes in waits (and the edge's glow lights)
  gmHold: () => `${GAME2()}<i class="glow"></i>${WAIT()}
    ${[0, 1, 2].map((i) => `<span class="drop d${i + 1}">${PAV(person(i))}</span>`).join('')}`,
  // 2: the game is gone, and one card says who wrote
  gmCard: () => `<div class="desk"><i class="tb"></i></div>${GAME2('off')}${WAIT('out')}${SUMCARD('in')}`,
  // 3: a name in the card opens that chat (the card moves aside for it)
  gmOpen: () => {
    const p = person(0);
    const gm = texts.gm || {};
    return `<div class="desk"><i class="tb"></i></div>${SUMCARD('stay', true)}
      <div class="pnl3"><div class="ph3"><span class="seg3">${['instagram', 'facebook', 'x', 'discord'].map((a, i) => `<i${i ? '' : ' class="on"'}>${icon(a)}</i>`).join('')}</span></div>
        <div class="ch3"><i class="bk3"></i>${PAV(p)}<b>${esc(p.name)}</b></div>
        <p class="mi">${esc(p.msg)}</p><p class="mo">${esc(gm.reply || '')}</p></div>${CURSOR}`;
  },
  // the guide on the first start: Next, step by step
  guide: () => `<div class="gcard"><div class="gt"><i class="lg">${icon('logo')}</i><b></b></div>
    <div class="thumb"><i class="m1"></i><i class="m2"></i><i class="gp"></i><i class="gpn"></i></div>
    <div class="gd"><i></i><i></i><i></i><i></i><i></i><i class="now"></i></div><b class="l1"></b><b class="l2"></b>
    <i class="nx">${icon('chevronRight')}</i></div>${CURSOR}`,
  // Facebook's own logo, big and on the tab
  fbIcon: () => `<i class="big swap"><span class="old">${MESSENGER(1)}</span><span class="new">${icon('facebook')}</span><i class="ring"></i></i>
    <div class="pill2"><i class="chev"></i><i class="ap">${icon('instagram')}</i>
    <i class="ap swap"><span class="old">${MESSENGER(2)}</span><span class="new">${icon('facebook')}</span><i class="ring"></i></i>
    <i class="ap">${icon('x')}</i><i class="ap">${icon('discord')}</i></div>`,
  // game mode: messages wait while the game fills the screen, then one card says who wrote
  gameMode: () => `<div class="desk"><i class="tb"></i></div><div class="gm">${GAME}</div>
    <div class="wait"><i class="pz"></i><i class="a a1"></i><i class="a a2"></i><i class="a a3"></i></div>
    <div class="sum"><div class="sh"><i class="lg">${icon('logo')}</i><p><b>${esc(texts.gameTitle)}</b><small>${esc(texts.gameCount)}</small></p></div>
      <p class="r r1"><i class="av"></i><b></b><span></span><em>2</em></p><p class="r r2"><i class="av"></i><b></b><span></span></p></div>${CURSOR}`,
  // the highlight glides to the app picked; a new message's number pops
  glide: () => `<div class="hdr"><i class="gl"></i>${['instagram', 'facebook', 'x', 'discord'].map((a, i) => `<i class="t t${i + 1}">${icon(a)}</i>`).join('')}
    <b class="bd b1">1</b><b class="bd b2">2</b></div>${CURSOR}`,
  // Settings → Discord
  dcPage: () => `<div class="win"><div class="wtop"><i></i><b></b></div>
    <div class="list"><p><i class="t t1"></i><b></b></p><p><i class="t t2"></i><b></b></p><p><i class="t t3"></i><b></b></p>
      <p class="dc"><i class="t tdc">${icon('discordMono')}</i><span>Discord</span></p></div>
    <div class="page"><div class="hero">${icon('discordMono')}</div>
      <div class="box"><small>${esc(texts.voice)}</small><em>Ctrl + Alt + M</em></div>
      <div class="box"><small>${esc(texts.popups)}</small><p><span>Gamers</span>${SWITCH(true)}</p><p><span>My Server</span>${SWITCH(false)}</p></div></div>
  </div>${CURSOR}`,
  // which server and channel; one server switched off
  dcServers: () => `${GAME}<div class="srv"><p><i class="g1"></i><span>Gamers</span>${SWITCH(true)}</p><p><i class="g2"></i><span>My Server</span>${SWITCH(true, 'flip')}</p></div>
    <div class="toast t1"><i class="ic">${icon('discordMono')}</i><div><small>Discord · <b>Gamers · #general</b></small><span>Alice: gg 🎉</span></div></div>
    <div class="toast t2"><i class="ic">${icon('discordMono')}</i><div><small>Discord · <b>My Server · #chat</b></small><span>Bob: …</span></div><i class="mute">${icon('bell')}</i></div>`,
  // voice keys from inside a game
  dcVoice: () => `${GAME}<div class="keys"><kbd class="k1">Ctrl</kbd><kbd class="k2">Alt</kbd><kbd class="k3">M</kbd></div>
    <div class="vt v1"><i class="ic">${icon('discordMono')}</i><span>${esc(texts.muted)}</span></div>
    <div class="vt v2"><i class="ic">${icon('discordMono')}</i><span>${esc(texts.unmuted)}</span></div>`,
  // sharing the screen: it goes on with ChatDock hidden
  dcShare: () => `${GAME}<i class="live"></i><i class="badge">${icon('monitor')}</i>${MINI(2)}
    <div class="bar"><i class="mon">${icon('monitor')}</i><b></b><i class="stop"></i></div>`,
  // the phone and screen icons on the tab
  dockCalls: () => `${GAME}<div class="pill"><i class="chev"></i><i class="ap">${icon('instagram')}</i><i class="ap">${icon('facebook')}</i><i class="ap">${icon('discord')}</i>
    <i class="line"></i><i class="chip call">${icon('phone')}</i><i class="chip share">${icon('monitor')}</i></div>${MINI(3)}${CURSOR}`,
  // a call keeps Discord awake
  dcAwake: () => `<i class="wave w1"></i><i class="wave w2"></i><div class="orb">${icon('discordMono')}<i class="ph">${icon('phone')}</i></div>
    <div class="moon"><i></i><span>z</span><span>Z</span></div><i class="no"></i>`,
  // Spotify in the chat; it plays on with ChatDock hidden
  spotify: () => `${GAME}<div class="spot"><div class="sh"><i class="ic sp">${icon('spotify')}</i><b></b></div><div class="art"></div><b class="l1"></b><b class="l2"></b>
    <div class="prog"><i></i></div><div class="ctl"><i class="prev"></i><i class="play"></i><i class="next"></i></div>
    <div class="eq in"><i></i><i></i><i></i><i></i></div></div>
    <i class="tuck"></i><div class="eq out"><i></i><i></i><i></i><i></i></div>`,
  // each app's own volume
  volume: () => ['discord', 'spotify'].map((app, n) => `<div class="vcard c${n + 1}"><i class="ic ${app}">${icon(app === 'discord' ? 'discordMono' : 'spotify')}</i>
    <div class="sl"><i class="fill"></i><i class="thumb"></i></div><span class="pct">${n ? '<b>100%</b>' : '<b class="num"></b>'}</span>
    <i class="spk hi">${icon('volume')}</i>${n ? '' : `<i class="spk lo">${icon('volumeLow')}</i>`}</div>`).join('') + CURSOR,
  // Discord's servers listed right away; keep it awake
  dcList: () => `<div class="side"><i class="g g1">G</i><i class="g g2">M</i><i class="g g3">V</i></div>
    <div class="list"><p class="warn"><span class="moon"><i></i></span><b></b><em>${esc(texts.keepAwake)}</em></p>
      <p class="r r1"><i class="d d1"></i><span>Gamers</span>${SWITCH(true)}</p><p class="r r2"><i class="d d2"></i><span>My Server</span>${SWITCH(true)}</p>
      <p class="r r3"><i class="d d3"></i><span>Valorant TH</span>${SWITCH(true)}</p></div>${CURSOR}`,
  // lighter while hidden
  lighter: () => `<div class="meters"><p><span>CPU</span><i class="bar"><b class="b1"></b></i></p><p><span>GPU</span><i class="bar"><b class="b2"></b></i></p></div>
    <i class="tuck"></i><i class="done"></i>`,
};

function setupDemos(list) {
  demoList = list;
  clearTimeout(demoTimer);
  demoBox.hidden = list.length === 0;
  dots.replaceChildren(
    ...list.map((d, i) => {
      const b = document.createElement('button');
      b.type = 'button';
      b.className = 'dot';
      b.tabIndex = -1;
      b.addEventListener('click', () => showDemo(i, true));
      return b;
    }),
  );
  rounds = 0;
  if (list.length) showDemo(0, false);
}

function showDemo(i, byHand) {
  clearTimeout(demoTimer);
  demoAt = i;
  const d = demoList[i];
  scene.className = `scene s-${d.id}`;
  scene.innerHTML = SCENES[d.id](); // our own markup; the texts in it are escaped
  demoList.forEach((x, j) => {
    x.li.classList.toggle('now', j === i);
    dots.children[j].classList.toggle('now', j === i);
  });
  if (byHand) rounds = 0; // a click starts the turns over
  // only for a click: by itself it would open the list scrolled down (before the window has its
  // size) and pull it away from what the user is reading every few seconds
  if (byHand && notes.scrollHeight > notes.clientHeight) d.li.scrollIntoView({ block: 'nearest' });
  next();
}

function next() {
  clearTimeout(demoTimer);
  if (still || demoList.length === 0) return;
  demoTimer = setTimeout(() => {
    if (hovering) return next(); // watching this one: keep it
    const n = (demoAt + 1) % demoList.length;
    if (n === 0 && ++rounds >= ROUNDS) return; // enough: stay on the last frame
    showDemo(n, false);
  }, SCENE_MS);
}

scene.addEventListener('mouseenter', () => { hovering = true; });
scene.addEventListener('mouseleave', () => { hovering = false; });
notes.addEventListener('click', (e) => {
  const li = e.target.closest('li[data-demo]');
  if (li && !window.getSelection().toString()) showDemo(Number(li.dataset.demo), true);
});

// The card's full height with every line showing, plus the room around it for the shadow (CSS px).
// Its layout height: the pop-in animation scales what getBoundingClientRect() would report.
function naturalHeight() {
  card.classList.add('measure');
  const h = card.offsetHeight;
  card.classList.remove('measure');
  return h + 33;
}

chatdock.on('whatsnew:show', ({ kind, locale, title, route, sections, ok, github, close, demo, again }) => {
  texts = demo || {};
  intro = kind === 'intro';
  closing = false; // (shown again in place: the explainer over What's new)
  card.classList.remove('out');
  document.body.classList.toggle('intro', intro);
  document.documentElement.lang = locale;
  document.getElementById('title').textContent = title;
  document.getElementById('route').textContent = route;
  document.getElementById('ok').textContent = ok;
  document.getElementById('github').textContent = github;
  document.getElementById('github').hidden = intro;
  document.getElementById('again').hidden = !intro;
  document.querySelector('#again span').textContent = again || '';
  againBox.checked = false;
  const x = document.getElementById('x');
  x.title = close;
  x.setAttribute('aria-label', close);
  notes.replaceChildren();
  const demos = [];
  for (const s of sections) {
    if (s.title) {
      const h = document.createElement('h3');
      h.textContent = s.title;
      notes.append(h);
    }
    const ul = document.createElement('ul');
    const ids = DEMOS[s.build] || [];
    s.lines.forEach((line, i) => {
      const li = document.createElement('li');
      li.textContent = line;
      if (SCENES[ids[i]]) {
        li.dataset.demo = String(demos.length);
        demos.push({ id: ids[i], li });
      }
      ul.append(li);
    });
    notes.append(ul);
  }
  notes.scrollTop = 0;
  setupDemos(demos);
  chatdock.send('whatsnew:size', naturalHeight());
  card.classList.remove('in');
  void card.offsetWidth; // restart the pop-in animation
  card.classList.add('in');
});

function close(action) {
  if (closing) return;
  closing = true;
  card.classList.remove('in');
  card.classList.add('out');
  // the explainer: show it again next time, unless "don't show this again" is ticked
  const said = intro ? { introAgain: !againBox.checked } : undefined;
  setTimeout(() => (said ? chatdock.send(action, said) : chatdock.send(action)), 150);
}

document.getElementById('ok').addEventListener('click', () => close('whatsnew:close'));
document.getElementById('x').addEventListener('click', () => close('whatsnew:close'));
document.getElementById('github').addEventListener('click', () => close('whatsnew:releases'));
document.addEventListener('keydown', (e) => {
  // Enter on a focused button is that button's click
  if (e.key === 'Escape' || (e.key === 'Enter' && !(document.activeElement instanceof HTMLButtonElement))) {
    close('whatsnew:close');
  }
});

chatdock.send('ui:ready');
