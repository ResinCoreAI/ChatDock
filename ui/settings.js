'use strict';

// Settings screen inside the panel: a home page (search + categories, each with a one-line summary
// of how it is set) and one page per category. The controls are plain HTML with data-* hooks:
//   data-pref="key"          checkbox / select / range, or a button with data-value  -> 'settings:set'
//   data-action="name"       button -> 'settings:action' (data-arg = its argument)
//   data-action-local="name" button handled here (back, ...)
//   data-confirm="i18n key"  risky action: the first click only arms the button for a few seconds
//   data-goto="page"         category -> that page
//   data-app-pref="key"      per-app switch (inside a [data-app] row) -> 'settings:app-pref'
// Values are checked again in the main process before anything uses them. Text comes from i18n.js.

(() => {
  const root = document.getElementById('settings');
  const stage = document.getElementById('stage');
  const q = (sel) => root.querySelector(sel);
  const qa = (sel) => root.querySelectorAll(sel);
  const t = (key, vars) => window.i18n.t(key, vars);
  let st = null;
  let appsKey = '';
  let hotkeysKey = '';
  let monitorsKey = '';
  let mapKey = '';
  let shown = false;
  let page = 'home';
  let homeScroll = 0;
  let hovered = '';
  let searchedLang = '';

  const CORNERS = { 'top-left': 'corner.tl', 'top-right': 'corner.tr', 'bottom-left': 'corner.bl', 'bottom-right': 'corner.br' };
  // per-app notification switches shown as chips (pop-ups themselves have the row's switch)
  const APP_CHIPS = [['preview', 'perApp.text'], ['chime', 'perApp.chime'], ['badge', 'perApp.badge'], ['sound', 'perApp.sound']];
  const clock = (ms) => new Date(ms).toLocaleTimeString(window.i18n.locale, { hour: '2-digit', minute: '2-digit' });
  const mb = (n) => Number(n || 0).toLocaleString(window.i18n.locale);
  const num = (n) => Number(n).toLocaleString(window.i18n.locale);
  const valueOf = (el, raw) => (el.dataset.type === 'number' ? Number(raw) : raw);
  const setText = (name, text, cls) => {
    for (const node of qa(`[data-text="${name}"]`)) {
      node.textContent = text;
      if (cls !== undefined) node.className = cls;
    }
  };

  function el(tag, cls, text) {
    const node = document.createElement(tag);
    if (cls) node.className = cls;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  // ---------------------------------------------------------------- input -> main process
  let rangeTimer = null;
  root.addEventListener('input', (e) => { // live preview while dragging a slider
    const node = e.target;
    if (node.type !== 'range' || !node.dataset.pref) return;
    const out = q(`[data-out="${node.dataset.pref}"]`);
    if (out) out.textContent = `${node.value}%`;
    clearTimeout(rangeTimer);
    rangeTimer = setTimeout(() => chatdock.send('settings:set', node.dataset.pref, Number(node.value) / Number(node.dataset.scale || 1)), 40);
  });

  root.addEventListener('change', (e) => {
    const node = e.target;
    const row = node.closest('[data-app]');
    if (node.dataset.appToggle) {
      chatdock.send('settings:app', node.dataset.appToggle, node.checked);
      return;
    }
    if (node.dataset.dcServer !== undefined) {
      chatdock.send('settings:action', 'discord-server', { name: node.dataset.dcServer, on: node.checked });
      return;
    }
    if (node.dataset.appPref && row) {
      chatdock.send('settings:app-pref', row.dataset.app, node.dataset.appPref, node.checked);
      return;
    }
    const key = node.dataset.pref;
    if (!key) return;
    if (node.type === 'checkbox') chatdock.send('settings:set', key, node.checked);
    else if (node.tagName === 'SELECT') chatdock.send('settings:set', key, valueOf(node, node.value));
    else if (node.type === 'range') chatdock.send('settings:set', key, Number(node.value) / Number(node.dataset.scale || 1));
  });

  root.addEventListener('click', (e) => {
    const btn = e.target.closest('button');
    if (!btn || !root.contains(btn)) return;
    const row = btn.closest('[data-app]');
    if (btn.dataset.goto) {
      goto(btn.dataset.goto);
    } else if (btn.dataset.monitor !== undefined) {
      chatdock.send('settings:set', 'displayId', btn.dataset.monitor);
    } else if (btn.dataset.pref && btn.dataset.value !== undefined) {
      chatdock.send('settings:set', btn.dataset.pref, valueOf(btn, btn.dataset.value));
    } else if (btn.dataset.appPref && row) {
      chatdock.send('settings:app-pref', row.dataset.app, btn.dataset.appPref, !btn.classList.contains('on'));
    } else if (btn.dataset.actionLocal === 'back') {
      goto('home');
    } else if (btn.dataset.actionLocal === 'other-side') {
      chatdock.send('settings:set', 'side', st.prefs.side === 'left' ? 'right' : 'left');
    } else if (btn.dataset.action) {
      if (btn.dataset.confirm && !arm(btn)) return;
      const arg = btn.dataset.arg;
      chatdock.send('settings:action', btn.dataset.action, arg === undefined ? undefined : /^-?\d+$/.test(arg) ? Number(arg) : arg);
    }
  });

  // Risky buttons: the first click turns the button red and asks again; the second one runs it.
  function arm(btn) {
    if (btn.classList.contains('armed')) {
      disarm(btn);
      return true;
    }
    btn.dataset.label = btn.textContent;
    btn.textContent = t(btn.dataset.confirm);
    btn.classList.add('armed');
    btn.armTimer = setTimeout(() => disarm(btn), 4000);
    return false;
  }

  function disarm(btn) {
    clearTimeout(btn.armTimer);
    if (btn.dataset.label) btn.textContent = btn.dataset.label;
    btn.classList.remove('armed');
  }

  // ---------------------------------------------------------------- pages
  function pageTitle(name) {
    const p = q(`.set-page[data-page="${name}"]`);
    return p && p.dataset.title ? t(p.dataset.title) : t('set.title');
  }

  function goto(name) {
    const target = q(`.set-page[data-page="${name}"]`) ? name : 'home';
    const back = target === 'home' && page && page !== 'home';
    if (page === 'home' && target !== 'home') homeScroll = stage.scrollTop;
    for (const p of qa('.set-page')) p.hidden = p.dataset.page !== target;
    const shownPage = q(`.set-page[data-page="${target}"]`);
    shownPage.classList.toggle('from-left', back);
    shownPage.style.animation = 'none';
    void shownPage.offsetWidth; // play the slide-in again
    shownPage.style.animation = '';
    if (target !== page && page === 'monitors') identifyHover('');
    page = target;
    root.dataset.at = target;
    q('.set-back').hidden = target === 'home';
    setText('pageTitle', pageTitle(target));
    stage.scrollTop = target === 'home' ? homeScroll : 0;
    if (target === 'monitors') mapKey = ''; // drawn now that it has a size
    if (st) render();
  }

  stage.addEventListener('scroll', () => {
    if (root.hidden) return;
    q('.set-top').classList.toggle('scrolled', stage.scrollTop > 4);
  }, { passive: true });

  chatdock.on('settings:goto', (name) => goto(String(name)));

  // ---------------------------------------------------------------- search: every setting on every page
  const search = q('[data-search]');

  // The English text of an element that is translated (so English words find settings in any language).
  const english = (node) => {
    const key = node && (node.dataset.i18n || (node.querySelector('[data-i18n]') || {}).dataset?.i18n);
    const en = window.CHATDOCK_I18N.STRINGS.en;
    return key && window.i18n.lang !== 'en' && en[key] ? en[key] : '';
  };

  function searchIndex() {
    const out = [];
    for (const p of qa('.set-page')) {
      if (p.dataset.page === 'home') continue;
      if (p.dataset.page === 'discord' && !(st && st.discord.on)) continue; // its page is only there while it is on
      const where = pageTitle(p.dataset.page);
      const pageEn = window.i18n.lang !== 'en' ? window.CHATDOCK_I18N.STRINGS.en[p.dataset.title] || '' : '';
      out.push({ page: p.dataset.page, where: t('set.title'), title: where, desc: '', alt: pageEn, node: null });
      for (const node of p.querySelectorAll('.row, .sec-title, .link-row, .radio')) {
        const hidden = node.closest('[hidden]');
        if (hidden && hidden !== p) continue; // e.g. monitor choices with a single monitor
        const head = node.matches('.sec-title') ? node : node.querySelector('.label b, .txt b, :scope > span:first-child');
        const small = node.querySelector('.label small, .txt small');
        const title = ((head || {}).textContent || '').trim();
        const desc = ((small || {}).textContent || '').trim();
        if (title) out.push({ page: p.dataset.page, where, title, desc, alt: `${english(head)} ${english(small)}`, node });
      }
    }
    return out;
  }

  function marked(text, needle) {
    const b = el('b');
    const i = text.toLowerCase().indexOf(needle);
    if (i < 0) {
      b.textContent = text;
      return b;
    }
    b.append(text.slice(0, i), el('mark', '', text.slice(i, i + needle.length)), text.slice(i + needle.length));
    return b;
  }

  function runSearch() {
    searchedLang = window.i18n.lang;
    const text = search.value.trim();
    const needle = text.toLowerCase();
    const results = q('[data-results]');
    q('[data-cats]').hidden = !!needle;
    results.hidden = !needle;
    results.textContent = '';
    if (!needle) return;
    const hits = searchIndex().filter((h) => `${h.title} ${h.desc} ${h.alt}`.toLowerCase().includes(needle)).slice(0, 30);
    if (!hits.length) {
      results.append(el('p', 'no-results', t('s.searchEmpty', { q: text })));
      return;
    }
    const card = el('div', 'card');
    for (const h of hits) {
      const b = el('button', 'result');
      b.append(marked(h.title, needle), el('small', '', h.desc ? `${h.where} · ${h.desc}` : h.where));
      b.addEventListener('click', () => jump(h));
      card.append(b);
    }
    results.append(card);
  }

  function jump(h) {
    goto(h.page);
    if (!h.node) return;
    requestAnimationFrame(() => {
      h.node.scrollIntoView({ block: 'center' });
      h.node.classList.remove('flash');
      void h.node.offsetWidth;
      h.node.classList.add('flash');
    });
  }

  search.addEventListener('input', runSearch);
  search.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') q('.result')?.click();
    if (e.key === 'Escape') e.preventDefault(); // not cleared here: 'settings:esc' decides
  });

  // Esc (ChatDock hears it first; two quick ones hide the chat): one step back each time
  chatdock.on('settings:esc', () => {
    if (root.hidden) return;
    if (page !== 'home') {
      goto('home'); // back to the results too, if a search brought you here
    } else if (search.value) {
      search.value = '';
      runSearch();
    } else {
      chatdock.send('settings:action', 'close');
    }
  });

  // ---------------------------------------------------------------- state -> controls
  function renderHotkeys() {
    const key = `${window.i18n.lang}|${st.hotkeys.map((h) => h.label).join('|')}`;
    if (key === hotkeysKey) return;
    hotkeysKey = key;
    const sel = q('[data-hotkeys]');
    sel.textContent = '';
    for (const h of st.hotkeys) sel.append(new Option(h.label, h.acc));
    sel.append(new Option(t('set.hotkeyNone'), ''));
  }

  // ---------------------------------------------------------------- Discord
  let voiceKey = '';
  let serversKey = '';

  function renderDiscord() {
    const dc = st.discord;
    for (const node of qa('[data-show="discord"]')) node.hidden = !dc.on;
    const vk = `${window.i18n.lang}|${st.voiceKeys.map((k) => k.label).join('|')}`;
    if (vk !== voiceKey) {
      voiceKey = vk;
      for (const sel of qa('[data-voice-keys]')) {
        sel.textContent = '';
        sel.append(new Option(t('set.hotkeyNone'), ''));
        for (const k of st.voiceKeys) sel.append(new Option(k.label, k.acc));
      }
    }
    for (const sel of qa('[data-voice-keys]')) {
      const other = st.prefs[sel.dataset.pref === 'discordMuteKey' ? 'discordDeafenKey' : 'discordMuteKey'];
      for (const o of sel.options) o.disabled = !!o.value && (o.value === other || o.value === st.prefs.hotkey);
    }
    const note = (key, ok) => (!st.prefs[key] ? [t('dc.keyOff'), ''] : ok ? [t('dc.keyNote'), ''] : [t('dc.keyTaken'), 'warn']);
    const [mute, muteCls] = note('discordMuteKey', dc.muteOk);
    const [deafen, deafenCls] = note('discordDeafenKey', dc.deafenOk);
    setText('dcMuteNote', mute, muteCls);
    setText('dcDeafenNote', deafen, deafenCls);

    const box = q('[data-dc-servers]');
    const sk = JSON.stringify([dc.servers.map((s) => s.name), window.i18n.lang]);
    if (sk !== serversKey) {
      serversKey = sk;
      box.textContent = '';
      if (!dc.servers.length) box.append(el('p', 'dc-empty', t('dc.noServers')));
      for (const s of dc.servers) {
        const row = el('div', 'row dc-server');
        const label = el('div', 'label');
        label.append(el('b', '', s.name));
        const input = document.createElement('input');
        input.type = 'checkbox';
        input.dataset.dcServer = s.name;
        input.setAttribute('aria-label', s.name);
        row.append(label, switchFor(input));
        box.append(row);
      }
    }
    q('.dc-sleep').hidden = !dc.sleeps; // asleep, Discord sends no pop-ups
    for (const input of box.querySelectorAll('input[data-dc-server]')) {
      const s = dc.servers.find((x) => x.name === input.dataset.dcServer);
      if (s) input.checked = s.on;
    }
  }

  // ---------------------------------------------------------------- monitors
  const sideWord = (side) => t(side === 'left' ? 's.sideLeft' : 's.sideRight');
  const chosen = () => st.monitors.find((m) => m.key === st.prefs.displayId);

  function identifyHover(key) {
    if (key === hovered) return;
    hovered = key;
    chatdock.send('settings:action', 'identify-hover', key);
  }

  // Every monitor to scale, where it really is; the chat drawn where it opens, and the edges the
  // mouse opens it from glowing.
  function renderMap() {
    const box = q('[data-map]');
    const width = box.clientWidth;
    const key = JSON.stringify([st.monitors, st.monitorMode, st.prefs.displayId, st.prefs.side, width, window.i18n.lang]);
    if (key === mapKey || !width) return;
    mapKey = key;
    box.textContent = '';
    const mons = st.monitors;
    if (!mons.length) return;
    const pad = 18;
    const minX = Math.min(...mons.map((m) => m.x));
    const minY = Math.min(...mons.map((m) => m.y));
    const spanX = Math.max(...mons.map((m) => m.x + m.w)) - minX;
    const spanY = Math.max(...mons.map((m) => m.y + m.h)) - minY;
    const s = Math.min((width - pad * 2) / spanX, (box.clientHeight - pad * 2) / spanY);
    const ox = (width - spanX * s) / 2;
    const oy = (box.clientHeight - spanY * s) / 2;
    const side = st.prefs.side;
    const one = st.monitorMode === 'one';
    for (const m of mons) {
      const x = ox + (m.x - minX) * s;
      const y = oy + (m.y - minY) * s;
      const w = m.w * s;
      const h = m.h * s;
      const b = el('button', 'mon');
      b.style.cssText = `left:${x + 2}px;top:${y + 2}px;width:${w - 4}px;height:${h - 4}px`;
      b.dataset.monitor = m.key;
      b.title = `${t('mon.name', { n: m.n })} · ${m.label}`;
      b.setAttribute('aria-label', b.title);
      b.classList.toggle('chosen', one && m.key === st.prefs.displayId);
      b.classList.toggle('small', w < 84 || h < 56);
      if (m.chat) b.append(el('i', `art ${side}`));
      b.append(el('span', 'num', String(m.n)), el('span', 'nm', m.label));
      if (m.primary) b.append(el('span', 'tag', t('mon.main')));
      b.addEventListener('pointerenter', () => identifyHover(m.key));
      b.addEventListener('pointerleave', () => identifyHover(''));
      box.append(b);
      if (one && m.key !== st.prefs.displayId) continue;
      for (const [a, z] of m.edges[side] || []) {
        const mark = el('i', 'edge-mark');
        mark.style.cssText = `left:${side === 'left' ? x - 1 : x + w - 3}px;top:${y + a * h + 3}px;height:${Math.max(4, (z - a) * h - 6)}px`;
        box.append(mark);
      }
    }
  }

  // the panel was made wider or narrower: draw the map for its new size
  new ResizeObserver(() => {
    if (st && page === 'monitors') renderMap();
  }).observe(q('[data-map]'));

  function monitorChoices() {
    const list = [{ key: 'auto', badge: 'auto', title: t('mon.auto'), sub: t('mon.autoNote') }];
    for (const m of st.monitors) {
      list.push({
        key: m.key,
        badge: String(m.n),
        title: `${t('mon.name', { n: m.n })} · ${m.label}`,
        sub: t('mon.res', { w: m.w, h: m.h, hz: m.hz }),
        pill: m.primary ? t('mon.main') : '',
      });
    }
    if (st.monitorMode === 'missing') {
      list.push({ key: st.prefs.displayId, badge: '?', title: st.chosenLabel || t('mon.picked'), sub: t('mon.missing'), missing: true });
    }
    return list;
  }

  function renderMonitors() {
    const multi = st.monitors.length > 1;
    for (const node of qa('[data-show="multiDisplay"]')) node.hidden = !multi;
    for (const node of qa('[data-show="singleDisplay"]')) node.hidden = multi;

    const choices = monitorChoices();
    const key = JSON.stringify([choices, window.i18n.lang]);
    const box = q('[data-monitors]');
    if (key !== monitorsKey) {
      monitorsKey = key;
      box.textContent = '';
      for (const c of choices) {
        const b = el('button', 'radio');
        b.dataset.monitor = c.key;
        b.setAttribute('role', 'radio');
        b.classList.toggle('missing', !!c.missing);
        const badge = el('span', c.badge === 'auto' ? 'badge auto' : 'badge');
        if (c.badge === 'auto') badge.innerHTML = window.iconHTML('pointerEdge');
        else badge.textContent = c.badge;
        const txt = el('span', 'txt');
        txt.append(el('b', '', c.title), el('small', '', c.sub));
        b.append(el('span', 'ring'), badge, txt);
        if (c.pill) b.append(el('span', 'pill', c.pill));
        if (c.key !== 'auto' && !c.missing) {
          b.addEventListener('pointerenter', () => identifyHover(c.key));
          b.addEventListener('pointerleave', () => identifyHover(''));
        }
        box.append(b);
      }
    }
    for (const b of box.querySelectorAll('.radio')) {
      const on = b.dataset.monitor === st.prefs.displayId;
      b.classList.toggle('on', on);
      b.setAttribute('aria-checked', String(on));
    }

    const side = st.prefs.side;
    const c = chosen();
    const seam = st.seam;
    let cap = t('mon.capAuto');
    if (!multi) cap = t('mon.capSingle', { side: sideWord(side) });
    else if (st.monitorMode === 'one' && c) cap = seam ? t('mon.capOneOnly', { n: c.n }) : t('mon.capOne', { n: c.n, side: sideWord(side) });
    setText('mapCaption', cap);

    q('[data-notice="seam"]').hidden = !seam;
    if (seam) {
      const other = side === 'left' ? 'right' : 'left';
      const vars = { n: seam.n, m: seam.m, side: sideWord(side), other: sideWord(other) };
      setText('seamText', `${t(seam.m ? 'mon.seam' : 'mon.seamAny', vars)} ${t(seam.canSwitch ? 'mon.seamHowOther' : 'mon.seamHow', vars)}`);
      setText('seamFix', t('mon.useOther', vars));
      q('[data-action-local="other-side"]').hidden = !seam.canSwitch;
    }
    if (page === 'monitors') renderMap();
  }

  // ---------------------------------------------------------------- apps
  function appIcon(a) {
    const ico = el('span', 'ico');
    ico.innerHTML = window.iconHTML(a.icon); // our own static SVG
    return ico;
  }

  function switchFor(input) {
    const sw = el('label', 'switch');
    sw.append(input, document.createElement('span'));
    return sw;
  }

  // Apps card (on/off, RAM, sleep, clear data) and the per-app notification card.
  function buildAppRows() {
    const card = q('[data-apps]');
    const per = q('[data-perapp]');
    card.textContent = '';
    per.textContent = '';
    for (const a of st.catalog) {
      const row = el('div', 'row app-row');
      row.dataset.app = a.id;
      const label = el('div', 'label');
      const sleep = el('button', 'mini-chip sleep-chip', `💤 ${t('app.sleep')}`);
      sleep.dataset.appPref = 'sleep';
      sleep.title = t('app.sleepTitle');
      label.append(el('b', '', a.name), el('small', 'status'), sleep);
      const actions = el('div', 'actions');
      const clear = el('button', 'link-btn', t('app.clear'));
      clear.title = t('app.clearTitle', { name: a.name });
      clear.dataset.action = 'clear-app';
      clear.dataset.arg = a.id;
      clear.dataset.confirm = 'confirm.again';
      const input = document.createElement('input');
      input.type = 'checkbox';
      input.dataset.appToggle = a.id;
      input.setAttribute('aria-label', a.name);
      actions.append(clear, switchFor(input));
      row.append(appIcon(a), label, actions);
      card.append(row);

      if (!a.on) continue; // per-app notification settings only for apps that are on
      const prow = el('div', 'row perapp-row');
      prow.dataset.app = a.id;
      const plabel = el('div', 'label');
      const chips = el('div', 'toggles');
      for (const [key, text] of APP_CHIPS) {
        const chip = el('button', 'mini-chip toggle', t(text));
        chip.dataset.appPref = key;
        chips.append(chip);
      }
      plabel.append(el('b', '', a.name), el('small', 'status'), chips);
      const pop = document.createElement('input');
      pop.type = 'checkbox';
      pop.dataset.appPref = 'popups';
      pop.setAttribute('aria-label', `${a.name}: ${t('perApp.popups')}`);
      prow.append(appIcon(a), plabel, switchFor(pop));
      per.append(prow);
    }
  }

  function renderApps() {
    const key = `${window.i18n.lang}|${st.catalog.map((a) => `${a.id}:${a.on}`).join(',')}`;
    if (key !== appsKey) {
      appsKey = key;
      buildAppRows();
    }
    setText('memory', st.memory ? t('apps.memory', { mb: mb(st.memory) }) : t('apps.memoryWait'));
    const onCount = st.catalog.filter((a) => a.on).length;
    for (const a of st.catalog) {
      const row = q(`[data-apps] [data-app="${a.id}"]`);
      const input = row.querySelector('input');
      input.checked = a.on;
      input.disabled = a.on && onCount === 1; // keep at least one app
      let status = input.disabled ? t('app.keepOne') : t(a.on ? 'app.on' : 'app.off');
      if (a.on && a.asleep) status = t('app.asleep');
      else if (a.on && a.mb) status = t('app.ram', { state: status, mb: mb(a.mb) });
      row.querySelector('.status').textContent = status;
      const sleep = row.querySelector('.sleep-chip');
      sleep.hidden = !a.on;
      sleep.classList.toggle('on', a.prefs.sleep);

      const prow = q(`[data-perapp] [data-app="${a.id}"]`);
      if (!prow) continue;
      prow.querySelector('input').checked = a.prefs.popups;
      prow.classList.toggle('muted', !a.prefs.popups);
      prow.querySelector('.status').textContent = a.prefs.popups ? '' : t('perApp.offNote');
      for (const chip of prow.querySelectorAll('.toggle')) {
        const on = !!a.prefs[chip.dataset.appPref];
        chip.classList.toggle('on', on);
        chip.setAttribute('aria-pressed', String(on));
      }
    }
  }

  // ---------------------------------------------------------------- the rest
  function renderPrefs(p) {
    for (const node of qa('[data-pref]')) {
      const key = node.dataset.pref;
      if (!(key in p)) continue;
      const v = p[key];
      if (node.dataset.value !== undefined) {
        const on = String(v) === node.dataset.value;
        node.classList.toggle('on', on);
        node.setAttribute('aria-checked', String(on));
      } else if (node.type === 'checkbox') {
        node.checked = !!v;
      } else if (node.type === 'range') {
        if (document.activeElement === node) continue; // don't fight the user's drag
        node.value = String(Math.round(Number(v) * Number(node.dataset.scale || 1)));
        const out = q(`[data-out="${key}"]`);
        if (out) out.textContent = `${node.value}%`;
      } else if (node.tagName === 'SELECT') {
        node.value = String(v);
      }
    }
  }

  const dndOn = () => st.dndUntil === -1 || st.dndUntil > Date.now();

  function renderNotes() {
    const autostart = q('[data-pref="autostart"]');
    autostart.disabled = !st.autostartAvailable;
    setText('autostartNote', t(st.autostartAvailable ? 'set.autostartNote' : 'set.autostartUnavailable'));

    const p = st.prefs;
    if (p.hotkey && !st.hotkeyOk) setText('hotkeyNote', t('set.hotkeyTaken'), 'warn');
    else setText('hotkeyNote', t(p.hotkey ? 'set.hotkeyNote' : 'set.hotkeyOff'), '');

    for (const node of qa('[data-show="packaged"]')) node.hidden = !st.packaged;
    // "· Language" after the translated word helps someone who picked a language they can't read
    for (const node of qa('.en-hint')) node.hidden = window.i18n.lang === 'en';

    const until = st.dndUntil;
    const dnd = dndOn();
    setText('dnd', !dnd ? t('dnd.statusOff') : until === -1 ? t('dnd.statusForever') : t('dnd.statusUntil', { time: clock(until) }), dnd ? 'warn' : '');
    for (const b of qa('[data-dnd] button')) {
      const arg = Number(b.dataset.arg);
      b.classList.toggle('on', arg === 0 ? !dnd : arg === -1 && until === -1);
    }

    setText('cornerLabel', CORNERS[p.popupPosition] ? t(CORNERS[p.popupPosition]) : '');
    for (const node of qa('[data-dim-when="popups-off"]')) node.classList.toggle('dim', !p.popups);
    for (const node of qa('[data-dim-when="edge-off"]')) node.classList.toggle('dim', p.edgeMode === 'off');
  }

  function renderSecurity() {
    const box = q('[data-sec]');
    box.classList.toggle('warn', !st.cookieEncryption);
    if (st.cookieEncryption) {
      setText('secTitle', t('sec.onTitle'));
      setText('secDesc', t('sec.onDesc') + (st.cookiesMigrated ? '' : t('sec.migrating')));
    } else {
      setText('secTitle', t('sec.devTitle'));
      setText('secDesc', t('sec.devDesc'));
    }
  }

  function renderUpdates() {
    const u = st.update;
    const next = st.updateName; // e.g. "Beta Build 1.7"
    const news = st.whatsNew; // what's new in the build running now
    setText('version', st.build);
    setText('semver', t('about.version', { version: st.version }));
    const bar = q('.uc-bar');
    let text = '';
    let cls = '';
    switch (u.status) {
      case 'dev': text = t('upd.dev'); break;
      case 'checking': text = t('upd.checking'); break;
      case 'latest':
        text = (news && news.justUpdated ? `${t('upd.justUpdated', { version: st.build })} · ` : '')
          + t('upd.latest') + (u.checkedAt ? t('upd.checkedAt', { time: clock(u.checkedAt) }) : '');
        cls = 'ok';
        break;
      case 'available': text = t('upd.available', { version: next }); cls = 'new'; break;
      case 'downloading': text = t('upd.downloading', { version: next, percent: u.percent || 0 }); cls = 'new'; break;
      case 'ready': text = t('upd.ready', { version: next }); cls = 'new'; break;
      case 'installing': text = t('updwin.installing'); cls = 'new'; break;
      case 'error': {
        const offline = /net::ERR_|ENOTFOUND|ETIMEDOUT|ECONN|EAI_AGAIN|getaddrinfo|socket hang up/i.test(u.error || '');
        text = offline ? t('upd.offline') : t('upd.error', { error: u.error || t('upd.unknown') });
        cls = 'err';
        break;
      }
      default:
        text = news && news.justUpdated ? t('upd.justUpdated', { version: st.build }) : t(st.prefs.updateAutoCheck ? 'upd.idleAuto' : 'upd.idle');
        if (news && news.justUpdated) cls = 'ok';
    }
    setText('updateStatus', text, cls);
    bar.hidden = u.status !== 'downloading';
    bar.querySelector('i').style.width = `${u.percent || 0}%`;
    q('[data-action="install-update"]').hidden = u.status !== 'ready';
    q('[data-action="download-update"]').hidden = u.status !== 'available';
    const check = q('[data-action="check-update"]');
    check.disabled = !u.enabled || u.status === 'checking' || u.status === 'downloading';
    check.hidden = u.status === 'ready';
    // Notes of the update on its way, else what's new in the build we're on.
    const notes = q('.uc-notes');
    const coming = u.notes && ['available', 'downloading', 'ready', 'installing'].includes(u.status);
    notes.hidden = !coming && !news;
    setText('notesTitle', coming ? t('upd.notes') : news ? news.title : '');
    setText('notes', coming ? u.notes : news ? news.text : '');
    q('[data-action="whats-new"]').hidden = coming || !news; // the animated demos of this version
    q('.cat .dot').hidden = !(u.status === 'ready' || u.status === 'available');
  }

  // One line under each category: how it is set right now.
  function renderSummaries() {
    const p = st.prefs;
    const sub = (name, text) => setText(`sub-${name}`, text);
    const lang = p.lang === 'auto' ? t('lang.auto') : (st.langs.find((l) => l.id === p.lang) || {}).name || p.lang;
    sub('general', [lang, p.autostart ? t('set.autostart') : ''].filter(Boolean).join(' · '));

    const c = chosen();
    const where = st.monitors.length < 2 ? '' : st.monitorMode === 'one' && c ? `${t('mon.name', { n: c.n })} · ${c.label}` : t('mon.auto');
    sub('monitors', [where, t(p.side === 'left' ? 's.edgeLeft' : 's.edgeRight')].filter(Boolean).join(' · '));

    const edge = p.edgeMode === 'off' ? t('s.openNoEdge') : p.edgeHold > 0 ? t('s.openEdgeHold', { n: num(p.edgeHold) }) : t('s.openEdge');
    const hotkey = (st.hotkeys.find((h) => h.acc === p.hotkey) || {}).label;
    sub('open', `${edge} · ${hotkey || t('s.noHotkey')}`);

    const theme = p.theme === 'dark' ? t('theme.dark') : p.theme === 'light' ? t('theme.light') : t('s.themeSystem');
    sub('look', `${theme} · ${t('s.opacityShort', { n: Math.round(Number(p.opacity) * 100) })}`);

    const onCount = st.catalog.filter((a) => a.on).length;
    sub('apps', st.memory ? t('s.appsSub', { n: onCount, mb: mb(st.memory) }) : t('s.appsOn', { n: onCount }));

    sub('popups', !p.popups ? t('s.popupsOff') : dndOn() ? t('set.dnd') : t('s.popupsOn', { corner: t(CORNERS[p.popupPosition] || 'corner.tr') }));
    sub('security', t(st.cookieEncryption ? 's.secOk' : 's.secDev'));
    const vkLabel = (acc) => (st.voiceKeys.find((k) => k.acc === acc) || {}).label;
    const keys = [vkLabel(p.discordMuteKey), vkLabel(p.discordDeafenKey)].filter(Boolean).join(', ');
    sub('discord', [keys || t('s.dcNoKeys'), st.discord.servers.length ? t('s.dcServers', { n: st.discord.servers.length }) : ''].filter(Boolean).join(' · '));

    const u = st.update;
    let upd = st.build;
    if (['available', 'downloading', 'ready'].includes(u.status)) upd = t('s.updNew', { version: st.updateName });
    else if (u.status === 'installing') upd = t('s.updInstalling');
    else if (u.status === 'checking') upd = t('upd.checking');
    else if (u.status === 'error') upd = t('s.updError');
    else if (u.status === 'latest') upd = t('s.updLatest');
    sub('updates', upd);
    sub('about', st.build);
  }

  function render() {
    renderHotkeys();
    renderDiscord(); // fills its key lists before renderPrefs picks the chosen ones
    renderApps();
    renderPrefs(st.prefs);
    renderNotes();
    renderMonitors();
    renderSecurity();
    renderUpdates();
    renderSummaries();
    setText('pageTitle', pageTitle(page));
  }

  window.renderSettings = (next) => {
    if (!next) return;
    st = next;
    if (!shown) { // just opened: start at the top of the home page
      shown = true;
      search.value = '';
      runSearch();
      homeScroll = 0;
      page = '';
      goto('home');
      return;
    }
    render();
    if (search.value && searchedLang !== window.i18n.lang) runSearch(); // only new words: keeps the focus
  };

  // Screen closed: start at the home page next time, and no monitor stays lit.
  new MutationObserver(() => {
    if (!root.hidden) return;
    shown = false;
    identifyHover('');
  }).observe(root, { attributes: true, attributeFilter: ['hidden'] });
})();
