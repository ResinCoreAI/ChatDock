'use strict';

// Settings screen inside the panel. The controls are plain HTML with data-* hooks:
//   data-pref="key"          checkbox / select / range, or a button with data-value  -> 'settings:set'
//   data-action="name"       button -> 'settings:action' (data-arg = its argument)
//   data-confirm="i18n key"  risky action: the first click only arms the button for a few seconds
//   data-goto="section"      nav chip -> scroll to that section
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
  let displaysKey = '';
  let hotkeysKey = '';
  let shown = false;

  const CORNERS = { 'top-left': 'corner.tl', 'top-right': 'corner.tr', 'bottom-left': 'corner.bl', 'bottom-right': 'corner.br' };
  // per-app notification switches shown as chips (pop-ups themselves have the row's switch)
  const APP_CHIPS = [['preview', 'perApp.text'], ['chime', 'perApp.chime'], ['badge', 'perApp.badge'], ['sound', 'perApp.sound']];
  const clock = (ms) => new Date(ms).toLocaleTimeString(window.i18n.locale, { hour: '2-digit', minute: '2-digit' });
  const mb = (n) => Number(n || 0).toLocaleString(window.i18n.locale);
  const valueOf = (el, raw) => (el.dataset.type === 'number' ? Number(raw) : raw);
  const setText = (name, text, cls) => {
    for (const el of qa(`[data-text="${name}"]`)) {
      el.textContent = text;
      if (cls !== undefined) el.className = cls;
    }
  };

  // ---------------------------------------------------------------- input -> main process
  let rangeTimer = null;
  root.addEventListener('input', (e) => { // live preview while dragging a slider
    const el = e.target;
    if (el.type !== 'range' || !el.dataset.pref) return;
    const out = q(`[data-out="${el.dataset.pref}"]`);
    if (out) out.textContent = `${el.value}%`;
    clearTimeout(rangeTimer);
    rangeTimer = setTimeout(() => chatdock.send('settings:set', el.dataset.pref, Number(el.value) / Number(el.dataset.scale || 1)), 40);
  });

  root.addEventListener('change', (e) => {
    const el = e.target;
    const row = el.closest('[data-app]');
    if (el.dataset.appToggle) {
      chatdock.send('settings:app', el.dataset.appToggle, el.checked);
      return;
    }
    if (el.dataset.appPref && row) {
      chatdock.send('settings:app-pref', row.dataset.app, el.dataset.appPref, el.checked);
      return;
    }
    const key = el.dataset.pref;
    if (!key) return;
    if (el.type === 'checkbox') chatdock.send('settings:set', key, el.checked);
    else if (el.tagName === 'SELECT') chatdock.send('settings:set', key, valueOf(el, el.value));
    else if (el.type === 'range') chatdock.send('settings:set', key, Number(el.value) / Number(el.dataset.scale || 1));
  });

  root.addEventListener('click', (e) => {
    const btn = e.target.closest('button');
    if (!btn || !root.contains(btn)) return;
    const row = btn.closest('[data-app]');
    if (btn.dataset.goto) {
      goto(btn.dataset.goto);
    } else if (btn.dataset.pref && btn.dataset.value !== undefined) {
      chatdock.send('settings:set', btn.dataset.pref, valueOf(btn, btn.dataset.value));
    } else if (btn.dataset.appPref && row) {
      chatdock.send('settings:app-pref', row.dataset.app, btn.dataset.appPref, !btn.classList.contains('on'));
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

  // ---------------------------------------------------------------- sections
  function goto(name) {
    const sec = q(`[data-section="${name}"]`);
    if (sec) sec.scrollIntoView({ block: 'start', behavior: shown ? 'smooth' : 'instant' });
    markNav(name);
  }

  const nav = q('.set-nav');
  let navCurrent = '';
  function markNav(name) {
    if (name === navCurrent) return;
    navCurrent = name;
    for (const b of qa('.set-nav button')) {
      const on = b.dataset.goto === name;
      b.classList.toggle('on', on);
      if (!on) continue;
      // Keep the chip visible in a narrow panel. Scrolls the chip row only: scrollIntoView would
      // also stop the page's own smooth scroll to the section.
      const left = b.offsetLeft - 12;
      const right = b.offsetLeft + b.offsetWidth + 12;
      if (left < nav.scrollLeft) nav.scrollLeft = left;
      else if (right > nav.scrollLeft + nav.clientWidth) nav.scrollLeft = right - nav.clientWidth;
    }
  }

  stage.addEventListener('scroll', () => {
    if (root.hidden) return;
    const top = stage.getBoundingClientRect().top + 110;
    let current = 'general';
    for (const sec of qa('[data-section]')) if (sec.getBoundingClientRect().top <= top) current = sec.dataset.section;
    if (stage.scrollTop + stage.clientHeight >= stage.scrollHeight - 4) current = 'about';
    markNav(current);
  }, { passive: true });

  chatdock.on('settings:goto', (name) => goto(String(name)));

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

  function renderDisplays() {
    const key = st.displays.map((d) => `${d.id}:${d.label}`).join('|');
    if (key === displaysKey) return;
    displaysKey = key;
    const sel = q('[data-displays]');
    sel.textContent = '';
    for (const d of st.displays) sel.append(new Option(d.label, String(d.id)));
  }

  function el(tag, cls, text) {
    const node = document.createElement(tag);
    if (cls) node.className = cls;
    if (text !== undefined) node.textContent = text;
    return node;
  }

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

  function renderNotes() {
    const autostart = q('[data-pref="autostart"]');
    autostart.disabled = !st.autostartAvailable;
    setText('autostartNote', t(st.autostartAvailable ? 'set.autostartNote' : 'set.autostartUnavailable'));

    const p = st.prefs;
    if (p.hotkey && !st.hotkeyOk) setText('hotkeyNote', t('set.hotkeyTaken'), 'warn');
    else setText('hotkeyNote', t(p.hotkey ? 'set.hotkeyNote' : 'set.hotkeyOff'), '');

    for (const node of qa('[data-show="multiDisplay"]')) node.hidden = st.displays.length < 2;
    for (const node of qa('[data-show="packaged"]')) node.hidden = !st.packaged;
    // "· Language" after the translated word helps someone who picked a language they can't read
    for (const node of qa('.en-hint')) node.hidden = window.i18n.lang === 'en';

    const until = st.dndUntil;
    const dnd = until === -1 || until > Date.now();
    setText('dnd', !dnd ? t('dnd.statusOff') : until === -1 ? t('dnd.statusForever') : t('dnd.statusUntil', { time: clock(until) }), dnd ? 'warn' : '');
    for (const b of qa('[data-dnd] button')) {
      const arg = Number(b.dataset.arg);
      b.classList.toggle('on', arg === 0 ? !dnd : arg === -1 && until === -1);
    }

    setText('cornerLabel', CORNERS[p.popupPosition] ? t(CORNERS[p.popupPosition]) : '');
    for (const node of qa('[data-dim-when="popups-off"]')) node.classList.toggle('dim', !p.popups);
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
    setText('version', `v${st.version}`);
    const bar = q('.uc-bar');
    let text = '';
    let cls = '';
    switch (u.status) {
      case 'dev': text = t('upd.dev'); break;
      case 'checking': text = t('upd.checking'); break;
      case 'latest': text = t('upd.latest') + (u.checkedAt ? t('upd.checkedAt', { time: clock(u.checkedAt) }) : ''); cls = 'ok'; break;
      case 'available': text = t('upd.available', { version: u.version }); cls = 'new'; break;
      case 'downloading': text = t('upd.downloading', { version: u.version, percent: u.percent || 0 }); cls = 'new'; break;
      case 'ready': text = t('upd.ready', { version: u.version }); cls = 'new'; break;
      case 'error': {
        const offline = /net::ERR_|ENOTFOUND|ETIMEDOUT|ECONN|EAI_AGAIN|getaddrinfo|socket hang up/i.test(u.error || '');
        text = offline ? t('upd.offline') : t('upd.error', { error: u.error || t('upd.unknown') });
        cls = 'err';
        break;
      }
      default: text = t(st.prefs.updateAutoCheck ? 'upd.idleAuto' : 'upd.idle');
    }
    setText('updateStatus', text, cls);
    bar.hidden = u.status !== 'downloading';
    bar.querySelector('i').style.width = `${u.percent || 0}%`;
    q('[data-action="install-update"]').hidden = u.status !== 'ready';
    q('[data-action="download-update"]').hidden = u.status !== 'available';
    const check = q('[data-action="check-update"]');
    check.disabled = !u.enabled || u.status === 'checking' || u.status === 'downloading';
    check.hidden = u.status === 'ready';
    const notes = q('.uc-notes');
    notes.hidden = !u.notes || !['available', 'downloading', 'ready'].includes(u.status);
    setText('notes', u.notes || '');
    q('.set-nav .dot').hidden = !(u.status === 'ready' || u.status === 'available');
  }

  window.renderSettings = (next) => {
    if (!next) return;
    st = next;
    const opening = !shown;
    renderHotkeys();
    renderDisplays();
    renderApps();
    renderPrefs(st.prefs);
    renderNotes();
    renderSecurity();
    renderUpdates();
    if (opening) {
      shown = true;
      stage.scrollTop = 0;
      markNav('general');
    }
  };

  // Screen closed: start at the top next time.
  new MutationObserver(() => { if (root.hidden) shown = false; }).observe(root, { attributes: true, attributeFilter: ['hidden'] });
})();
