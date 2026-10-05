'use strict';

// The guide: ChatDock opens on it the first time, and "How to use" (the tray menu, Settings → About)
// shows it again. Five steps, each with a short animated scene of the real thing (with the user's
// own apps, hotkey and dock side); Next moves on. A scene plays up to three times, then keeps its
// last frame: a panel left open on the guide never keeps redrawing. With "reduce motion" in
// Windows every scene is one still picture. panel.js hands over each new state (tourRender).
(() => {
  const $ = (sel) => document.querySelector(sel);
  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
  const icon = (name) => window.iconHTML(name);
  const t = (key, vars) => i18n.t(key, vars);

  const STEPS = ['edge', 'hotkey', 'popup', 'game', 'ready'];
  const STILL = { edge: 7000, hotkey: 3200, popup: 7200, game: 7400, ready: 7000 }; // "reduce motion": the frame shown (ms)
  const PLAYS = 3;
  const PAUSE_MS = 1300; // between two plays
  const reduce = window.matchMedia('(prefers-reduced-motion: reduce)');

  const screen = $('#tour-screen');
  const stage = $('#tour-stage');
  const dots = $('#tour-dots');
  const texts = $('#tour-texts');
  const back = $('#tour-back');
  const next = $('#tour-next');
  const skip = $('#tour-skip');
  const autoRow = $('#autostart-row');

  let s = null; // ChatDock's last state
  let at = 0; // the step on screen
  let on = false; // the guide is the panel's screen
  let live = false; // ... and the panel is on screen: the scene plays
  let plays = 0;
  let timer = 0;
  let textKey = '';
  let sceneKey = '';

  const CURSOR = '<svg viewBox="0 0 16 16"><path d="M2 1.5v11.2l3-2.9 2 4.7 2.1-.9-2-4.6h4.2z" fill="#fff" stroke="#111" stroke-width="1" stroke-linejoin="round"/></svg>';
  const CUR = `<i class="cur"><span class="uf">${CURSOR}</span></i>`;
  const GAME = '<div class="game"><i></i><i></i><i></i><b></b></div>';
  const CLOCK = '<i class="clock"></i>';
  const RIPS = (n) => Array.from({ length: n }, (_, i) => `<i class="rip r${i + 1}"></i>`).join('');
  // the chat panel: its header with the user's apps (the first one open), and a few messages
  const head = (c) => `<div class="ph"><span class="seg">${c.apps.map((a, i) => `<i${i ? '' : ' class="on"'}>${icon(a.icon)}</i>`).join('')}</span></div>`;
  const chat = (c) => `<div class="pnl"><div class="uf">${head(c)}<i class="bb in b1"></i><i class="bb out b2"></i><i class="bb in b3"></i></div></div>`;

  const SCENES = {
    // hold the pointer on the edge: the line grows, the tab comes out, click an app, the chat slides in
    edge: (c) => `${GAME}<div class="flip"><i class="track"></i><i class="ln up"></i><i class="ln dn"></i>
      <div class="pill"><div class="uf"><i class="chev"></i>${c.apps.map((a) => `<i class="ap">${icon(a.icon)}</i>`).join('')}</div></div>
      ${RIPS(1)}${chat(c)}${CUR}</div>${CLOCK}`,
    // the hotkey brings the chat over the game, and takes it away again
    hotkey: (c) => `${GAME}<div class="flip">${chat(c)}</div>
      <div class="fix"><div class="keys">${c.keys.length ? c.keys.map((k, i) => `${i ? '<b>+</b>' : ''}<kbd class="k${i + 1}">${esc(k)}</kbd>`).join('') : `<kbd class="k1 gear">${icon('gear')}</kbd>`}</div>
      <i class="back">🎮</i></div>${CLOCK}`,
    // a pop-up says who wrote; a click on it opens that conversation
    popup: (c) => `${GAME}<div class="flip"><i class="glow g-${esc(c.from.id)}"></i>
      <div class="toast"><div class="uf"><span class="av">${esc(c.initial)}<i class="ab">${icon(c.from.icon)}</i></span>
        <span class="tx"><small>${esc(c.from.name)}</small><b>${esc(c.name)}</b><span>${esc(c.msg)}</span></span></div></div>
      ${RIPS(1)}
      <div class="pnl"><div class="uf"><div class="ch"><i class="bk"></i><span class="av">${esc(c.initial)}</span><b>${esc(c.name)}</b></div>
        <p class="msg in">${esc(c.msg)}</p><p class="typing"><i></i><i></i><i></i></p><p class="msg out">${esc(c.reply)}</p></div></div>
      ${CUR}</div>${CLOCK}`,
    // a game's display mode set to borderless, and the chat floats over it
    game: (c) => `${GAME}<div class="flip">${chat(c)}</div>
      <div class="fix"><div class="menu"><p class="mt">${icon('gear')}<b>${esc(t('tour.game.menu'))}</b></p>
        <p class="mr"><span>${esc(t('tour.game.mode'))}</span></p>
        <div class="dd"><span class="v v1">${esc(t('tour.game.full'))}</span><span class="v v2">${esc(t('tour.game.borderless'))}</span><i class="caret"></i></div><i class="ok"></i>
        <div class="list"><p class="o1">${esc(t('tour.game.full'))}</p><p class="o2">${esc(t('tour.game.borderless'))}</p><p class="o3">${esc(t('tour.game.windowed'))}</p></div></div>
      ${RIPS(2)}${CUR}</div>${CLOCK}`,
    // logging in on the app's own page, once, and it stays on this PC
    ready: (c) => `${GAME}<div class="flip"><div class="pnl"><div class="uf">${head(c)}
        <div class="login"><i class="lg">${icon(c.from.icon)}</i><i class="f f1"><i class="ty"></i></i><i class="f f2"><i class="ty"></i></i><b class="btn">${esc(t('tour.ready.login'))}</b></div>
        ${[1, 2, 3].map((n) => `<p class="row w${n}"><i class="av"></i><i class="l1"></i><i class="l2"></i></p>`).join('')}</div></div>
      <div class="safe"><p class="uf">${icon('shield')}<span>${esc(t('tour.ready.local'))}</span></p></div>
      ${RIPS(3)}${CUR}</div>${CLOCK}`,
  };

  // What the scenes show of the user's own setup.
  function context() {
    const apps = (s.apps || []).slice(0, 4);
    const from = apps.find((a) => a.id === 'instagram') || apps.find((a) => a.id === 'facebook') || apps[0]
      || { id: 'instagram', name: 'Instagram', icon: 'instagram' };
    const name = t('tour.sampleName');
    return {
      apps: apps.length ? apps : [from],
      from,
      name,
      initial: Array.from(name)[0] || '?',
      msg: t('tour.sampleMsg'),
      reply: t('tour.sampleReply'),
      keys: s.hotkey ? s.hotkey.split(' + ') : [],
    };
  }

  // Every step's text, stacked in one place (the tallest sets the height: the buttons never jump).
  function renderTexts() {
    const left = s.side === 'left';
    const key = [i18n.lang, s.side, s.hotkey || '', s.onboarded].join('|');
    if (key === textKey) return;
    textKey = key;
    const parts = {
      edge: ['tour.edge.title', left ? 'tour.edge.body.left' : 'tour.edge.body.right', left ? 'tour.edge.more.left' : 'tour.edge.more.right'],
      hotkey: s.hotkey ? ['tour.key.title', 'tour.key.body', 'tour.key.more'] : ['tour.key.titleNone', 'tour.key.bodyNone', 'tour.key.more'],
      popup: ['tour.pop.title', 'tour.pop.body', 'tour.pop.more'],
      game: ['tour.game.title', 'tour.game.body', 'tour.game.more'],
      ready: ['tour.ready.title', 'tour.ready.body', 'tour.ready.more'],
    };
    texts.replaceChildren(...STEPS.map((id, i) => {
      const [title, body, more] = parts[id];
      const el = document.createElement('div');
      el.className = 'tour-text';
      // our own texts (with <b>, <kbd> and the icons' places in them)
      el.innerHTML = `<small class="count">${esc(t('tour.count', { n: i + 1, total: STEPS.length }))}</small>
        <h2>${t(title, i18n.vars)}</h2><p>${t(body, i18n.vars)}</p><p class="more">${t(more, i18n.vars)}</p>`;
      return el;
    }));
    window.fillIcons(texts);
    window.renderHotkey(s.hotkey);
    markText('');
  }

  function markText(anim) {
    [...texts.children].forEach((el, i) => {
      el.classList.toggle('now', i === at);
      el.classList.remove('go-next', 'go-back');
      el.setAttribute('aria-hidden', String(i !== at));
    });
    const now = texts.children[at];
    if (anim && now) {
      void now.offsetWidth; // restart the slide-in
      now.classList.add(anim);
    }
  }

  function renderDots() {
    if (dots.children.length !== STEPS.length) {
      dots.replaceChildren(...STEPS.map((_, i) => {
        const b = document.createElement('button');
        b.type = 'button';
        b.className = 'tour-dot';
        b.addEventListener('click', () => go(i));
        return b;
      }));
    }
    [...dots.children].forEach((b, i) => {
      b.classList.toggle('now', i === at);
      b.classList.toggle('done', i < at);
      b.setAttribute('aria-label', t('tour.count', { n: i + 1, total: STEPS.length }));
      if (i === at) b.setAttribute('aria-current', 'step');
      else b.removeAttribute('aria-current');
    });
  }

  function renderButtons() {
    const last = at === STEPS.length - 1;
    back.hidden = at === 0;
    next.querySelector('.label').textContent = last ? t(s.onboarded ? 'welcome.back' : 'tour.start') : t('tour.next');
    next.classList.toggle('start', last);
    skip.textContent = t(s.onboarded ? 'tour.close' : 'tour.skip');
    skip.hidden = last;
    autoRow.hidden = !last || s.onboarded || !s.canAutostart;
    screen.title = t('tour.replay');
  }

  // The scene of this step, from its start.
  function play() {
    clearTimeout(timer);
    if (!live) return;
    const id = STEPS[at];
    sceneKey = sceneOf();
    stage.className = `tour-stage s-${id}${s.side === 'left' ? ' left' : ''}`;
    stage.innerHTML = SCENES[id](context()); // our own markup; the texts in it are escaped
    screen.setAttribute('aria-label', texts.children[at]?.querySelector('h2')?.textContent || '');
    if (reduce.matches) {
      for (const a of stage.getAnimations({ subtree: true })) {
        a.pause();
        a.currentTime = STILL[id];
      }
      return;
    }
    stage.querySelector('.clock').addEventListener('animationend', () => {
      if (++plays < PLAYS) timer = setTimeout(play, PAUSE_MS);
    }, { once: true });
  }

  const sceneOf = () => [at, i18n.lang, s.side, s.hotkey || '', (s.apps || []).map((a) => a.id).join(',')].join('|');

  function go(i) {
    i = Math.max(0, Math.min(STEPS.length - 1, i));
    const anim = i > at ? 'go-next' : i < at ? 'go-back' : '';
    at = i;
    markText(anim);
    renderDots();
    renderButtons();
    plays = 0;
    play();
  }

  function finish(started) {
    // the "start with Windows" box is on the last step, the first time only
    chatdock.send('onboarding:done', started && !autoRow.hidden ? { autostart: $('#autostart').checked } : {});
  }

  next.addEventListener('click', () => (at < STEPS.length - 1 ? go(at + 1) : finish(true)));
  back.addEventListener('click', () => go(at - 1));
  skip.addEventListener('click', () => finish(false));
  screen.addEventListener('click', () => {
    plays = 0;
    play();
  });
  document.addEventListener('keydown', (e) => {
    const typing = e.target instanceof Element && e.target.closest('select, input');
    if (!on || e.altKey || e.ctrlKey || e.metaKey || typing) return;
    if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
      e.preventDefault();
      go(at + (e.key === 'ArrowRight' ? 1 : -1));
    }
  });
  // drawn for 400 px across, scaled to the panel's width
  new ResizeObserver(() => stage.style.setProperty('--k', String(screen.clientWidth / 400 || 1))).observe(screen);

  window.tourRender = (state) => {
    s = state;
    const was = on;
    on = !s.settingsOpen && (s.help || !s.onboarded);
    if (on && !was) at = 0; // shown again ("How to use"): from the start
    renderTexts();
    renderDots();
    renderButtons();
    if (on && !was) markText('');
    const nowLive = on && !!s.shown;
    if (nowLive !== live) {
      live = nowLive;
      plays = 0;
      if (live) play();
      else clearTimeout(timer);
    } else if (live && sceneOf() !== sceneKey) {
      plays = 0;
      play(); // new language, side, hotkey or apps
    }
  };

  // for the self-test: which step, and go to one
  window.tourAt = () => at;
  window.tourGo = (i) => go(i);
})();
