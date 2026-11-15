'use strict';

// The Browser tab's own part of the panel page: its tab strip, the navigation row, the start page
// (search box, engines, shortcuts, recent searches) and the notice bar. The web pages themselves
// are WebView2 views Rust puts under this bar; everything here talks to Rust with browser:*.
(() => {
  const $ = (s) => document.querySelector(s);
  const t = (k, v) => window.i18n.t(k, v);
  const ico = (n) => window.iconHTML(n);
  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

  // letter badges for the engines (the mock's look; never the companies' own logos)
  const badge = (bg, fg, ch) => `<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="11" fill="${bg}"/><text x="12" y="16.4" text-anchor="middle" font-family="Segoe UI, sans-serif" font-weight="800" font-size="12.5" fill="${fg}">${ch}</text></svg>`;
  const ENGINES = [
    { id: 'google', name: 'Google', svg: badge('#4285f4', '#fff', 'G') },
    { id: 'bing', name: 'Bing', svg: badge('#008373', '#fff', 'b') },
    { id: 'ddg', name: 'DuckDuckGo', svg: badge('#de5833', '#fff', 'D') },
    { id: 'youtube', name: 'YouTube', svg: '<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="1" y="4.5" width="22" height="15" rx="4.5" fill="#ff0000"/><path d="M10 8.6v6.8l5.8-3.4z" fill="#fff"/></svg>' },
    { id: 'brave', name: 'Brave', svg: badge('#fb542b', '#fff', 'B') },
    { id: 'wiki', name: 'Wikipedia', svg: badge('#e8e8ea', '#202122', 'W') },
  ];
  const engineOf = (id) => ENGINES.find((e) => e.id === id) || ENGINES[0];
  const SHORTCUTS_MAX = 12;

  let st = null; // the last browser:state
  let lastTabs = '';

  // ---------------------------------------------------------------- the tab strip
  const COLORS = ['#0866ff', '#16a34a', '#f59e0b', '#db2777', '#7c3aed', '#0891b2', '#ea580c', '#e11d48'];
  const colorOf = (s) => COLORS[[...s].reduce((a, c) => a + c.charCodeAt(0), 0) % COLORS.length];
  const host = (u) => { try { return new URL(u).hostname.replace(/^www\./, ''); } catch { return ''; } };
  const titleOf = (tab) => (tab.url ? (tab.title || host(tab.url)) : t('browser.startTitle'));

  // the site's icon, or a letter tile while it has none (or it fails to load)
  function favicon(tab) {
    const box = document.createElement('span');
    box.className = 'f';
    if (!tab.url) {
      box.innerHTML = ico('search');
      return box;
    }
    const h = host(tab.url) || '?';
    const letter = () => {
      const lt = document.createElement('span');
      lt.className = 'lt';
      lt.style.background = colorOf(h);
      lt.textContent = h[0].toUpperCase();
      return lt;
    };
    if (tab.icon && tab.icon.startsWith('data:image/png;base64,')) { // read by Rust (see browser.rs)
      const img = document.createElement('img');
      img.alt = '';
      img.src = tab.icon;
      img.addEventListener('error', () => img.replaceWith(letter()), { once: true });
      box.append(img);
    } else {
      box.append(letter());
    }
    return box;
  }

  function renderTabs() {
    const strip = $('#btabs');
    const key = JSON.stringify([st.tabs.map((x) => [x.id, x.title, x.url, x.icon, x.loading]), st.open, st.full, window.i18n.lang]);
    if (key === lastTabs) return;
    lastTabs = key;
    strip.textContent = '';
    for (const tab of st.tabs) {
      const on = tab.id === st.open;
      const b = document.createElement('div');
      b.className = `bt${on ? ' on' : ''}${tab.loading ? ' spin' : ''}`;
      b.setAttribute('role', 'tab');
      b.setAttribute('aria-selected', String(on));
      b.title = titleOf(tab);
      const name = document.createElement('span');
      name.className = 't';
      name.textContent = titleOf(tab);
      const x = document.createElement('span');
      x.className = 'x';
      x.setAttribute('role', 'button');
      x.title = t('browser.closeTab');
      x.innerHTML = ico('close');
      b.append(favicon(tab), name, x);
      b.addEventListener('click', (e) => {
        if (e.target.closest('.x')) chatdock.send('browser:close', tab.id);
        else if (!on) chatdock.send('browser:tab', tab.id);
      });
      b.addEventListener('auxclick', (e) => { if (e.button === 1) chatdock.send('browser:close', tab.id); });
      b.addEventListener('mousedown', (e) => { if (e.button === 1) e.preventDefault(); }); // (no autoscroll)
      strip.append(b);
    }
    const add = document.createElement('button');
    add.type = 'button';
    add.className = 'bt-new';
    add.innerHTML = ico('plus');
    add.disabled = st.full;
    add.title = t(st.full ? 'browser.full' : 'browser.newTab');
    add.addEventListener('click', () => chatdock.send('browser:new'));
    strip.append(add);
    const open = strip.querySelector('.bt.on');
    if (open) open.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }

  // ---------------------------------------------------------------- the navigation row
  function renderNav() {
    $('#nav-back').disabled = !st.canBack;
    $('#nav-fwd').disabled = !st.canForward;
    $('#nav-reload').disabled = st.start;
    $('#nav-out').disabled = st.start;
    const addr = $('#addr-in');
    if (document.activeElement !== addr) addr.value = st.url;
    $('#addr-ico').innerHTML = st.url.startsWith('https://') ? ico('lock') : ico('search');
    const cur = st.tabs.find((x) => x.id === st.open);
    $('#bprog').classList.toggle('go', !!(cur && cur.loading));
  }
  $('#nav-back').addEventListener('click', () => chatdock.send('browser:back'));
  $('#nav-fwd').addEventListener('click', () => chatdock.send('browser:forward'));
  $('#nav-reload').addEventListener('click', () => chatdock.send('browser:reload'));
  $('#nav-home').addEventListener('click', () => chatdock.send('browser:home'));
  $('#nav-out').addEventListener('click', () => chatdock.send('browser:outside'));
  $('#addr').addEventListener('submit', (e) => {
    e.preventDefault();
    const v = $('#addr-in').value.trim();
    if (v) chatdock.send('browser:go', v);
    $('#addr-in').blur();
  });
  $('#addr-in').addEventListener('focus', () => $('#addr-in').select());
  $('#addr-in').addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      e.stopPropagation();
      $('#addr-in').value = st ? st.url : '';
      $('#addr-in').blur();
    }
  });

  // ---------------------------------------------------------------- the notice bar
  function renderNotice() {
    const n = st.notice;
    $('#bnotice').hidden = !n;
    if (!n) return;
    const act = $('#bnotice-act');
    if (n.kind === 'download') {
      $('#bnotice-text').textContent = t('browser.downloadRefused', { name: n.name || '' });
      act.textContent = t('browser.outside');
      act.hidden = !n.url;
      act.onclick = () => {
        chatdock.send('browser:outside', n.url);
        chatdock.send('browser:notice-close');
      };
    } else if (n.kind === 'program') {
      $('#bnotice-text').textContent = t('browser.programAsk', { scheme: n.scheme || '' });
      act.textContent = t('browser.programOpen');
      act.hidden = false;
      act.onclick = () => chatdock.send('browser:program-open');
    } else {
      $('#bnotice-text').textContent = t('browser.localRefused');
      act.hidden = true;
      act.onclick = null;
    }
  }
  $('#bnotice-x').addEventListener('click', () => chatdock.send('browser:notice-close'));

  // ---------------------------------------------------------------- the start page
  function renderEngines() {
    const e = engineOf(st.engine);
    $('#engine-btn').innerHTML = e.svg;
    $('#engine-btn').title = t('browser.engine', { name: e.name });
    const box = $('#bengines');
    box.textContent = '';
    for (const x of ENGINES) {
      const b = document.createElement('button');
      b.type = 'button';
      b.className = 'eng';
      b.setAttribute('role', 'radio');
      b.setAttribute('aria-checked', String(x.id === st.engine));
      b.innerHTML = `${x.svg}<span>${esc(x.name)}</span>`;
      b.addEventListener('click', () => chatdock.send('browser:engine', x.id));
      box.append(b);
    }
  }
  $('#engine-btn').addEventListener('click', () => {
    const i = ENGINES.findIndex((x) => x.id === st.engine);
    chatdock.send('browser:engine', ENGINES[(i + 1) % ENGINES.length].id);
  });

  let editing = -1;
  function renderTiles() {
    const box = $('#btiles');
    box.textContent = '';
    st.shortcuts.forEach((s, i) => {
      const a = document.createElement('div');
      a.className = 'b-tile';
      a.setAttribute('role', 'button');
      a.tabIndex = 0;
      a.title = s.u;
      const h = host(s.u) || s.n;
      a.innerHTML = `<span class="fav">${esc(s.n[0].toUpperCase())}</span>`
        + `<span class="t-name">${esc(s.n)}</span><span class="t-more" role="button" title="${esc(t('browser.edit'))}">${ico('dots')}</span>`;
      a.querySelector('.fav').style.background = colorOf(h); // (the page's content policy refuses style="" in markup)
      a.addEventListener('click', (e) => {
        if (e.target.closest('.t-more')) editTile(i);
        else chatdock.send('browser:open', s.u, false);
      });
      a.addEventListener('contextmenu', (e) => {
        e.preventDefault();
        e.stopPropagation();
        editTile(i);
      });
      a.addEventListener('keydown', (e) => { if (e.key === 'Enter') chatdock.send('browser:open', s.u, false); });
      box.append(a);
    });
    if (st.shortcuts.length < SHORTCUTS_MAX) {
      const add = document.createElement('button');
      add.type = 'button';
      add.className = 'b-tile add';
      add.innerHTML = `<span class="fav">${ico('plus')}</span><span class="t-name">${esc(t('browser.addShortcut'))}</span>`;
      add.addEventListener('click', () => editTile(-1));
      box.append(add);
    }
  }
  function editTile(i) {
    editing = i;
    const s = i >= 0 ? st.shortcuts[i] : { n: '', u: '' };
    $('#btile-title').textContent = t(i >= 0 ? 'browser.editShortcut' : 'browser.newShortcut');
    $('#btile-name').value = s.n;
    $('#btile-url').value = s.u;
    $('#btile-bad').hidden = true;
    $('#btile-delete').hidden = i < 0;
    $('#btile-sheet').hidden = false;
    $('#btile-name').focus();
  }
  const closeSheet = () => { $('#btile-sheet').hidden = true; };
  $('#btile-cancel').addEventListener('click', closeSheet);
  $('#btile-delete').addEventListener('click', () => {
    chatdock.send('browser:shortcut-delete', editing);
    closeSheet();
  });
  $('#btile-form').addEventListener('submit', (e) => {
    e.preventDefault();
    chatdock.send('browser:shortcut', editing, $('#btile-name').value.trim(), $('#btile-url').value.trim());
    closeSheet(); // a bad address opens it again (browser:shortcut-bad)
  });
  chatdock.on('browser:shortcut-bad', () => {
    $('#btile-sheet').hidden = false;
    $('#btile-bad').hidden = false;
    $('#btile-url').focus();
  });
  $('#btile-sheet').addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      e.stopPropagation();
      closeSheet();
    }
  });

  function renderRecent() {
    const ul = $('#brecent');
    ul.textContent = '';
    st.recent.forEach((r, i) => {
      const li = document.createElement('li');
      li.innerHTML = `<span class="r-ico">${ico('reload')}</span><span class="r-txt">${esc(r.q)}</span>`
        + `<span class="r-eng" title="${esc(engineOf(r.e).name)}">${engineOf(r.e).svg}</span>`
        + `<button type="button" class="r-x" title="${esc(t('browser.remove'))}">${ico('close')}</button>`;
      li.addEventListener('click', (e) => {
        if (e.target.closest('.r-x')) {
          chatdock.send('browser:recent-remove', i);
        } else {
          chatdock.send('browser:engine', r.e);
          chatdock.send('browser:go', r.q);
        }
      });
      ul.append(li);
    });
    const none = !st.recent.length;
    const row = $('#brecent-clear').parentElement; // the heading and "Clear all"
    row.hidden = !st.remember;
    ul.hidden = !st.remember;
    $('#brecent-empty').hidden = !none || !st.remember;
    $('#brecent-clear').hidden = none;
  }
  $('#brecent-clear').addEventListener('click', () => chatdock.send('browser:recent-clear'));

  // ---------------------------------------------------------------- typing
  const q = $('#bq');
  let rows = [];
  let at = -1;
  let timer = 0;
  let suggestions = { text: '', items: [] };

  const FILE_ENDINGS = ['js', 'ts', 'py', 'rs', 'md', 'txt', 'exe', 'dll', 'json', 'zip', 'png', 'jpg'];
  // Rust's classify, only to choose the first row and whether to ask for suggestions (Rust decides)
  function looksLikeAddress(s) {
    if (/^https?:\/\/\S+$/i.test(s)) return true;
    if (/\s/.test(s)) return false;
    const h = s.split(/[/?#]/)[0].replace(/:\d+$/, '');
    if (/^localhost$/i.test(h)) return true;
    if (/^(25[0-5]|2[0-4]\d|1?\d?\d)(\.(25[0-5]|2[0-4]\d|1?\d?\d)){3}$/.test(h)) return true; // an address as numbers
    const m = /^(?:[\p{L}\p{N}-]+\.)+(\p{L}{2,})$/u.exec(h);
    return !!m && !FILE_ENDINGS.includes(m[1].toLowerCase());
  }

  function hl(s, typed) {
    const i = s.toLowerCase().indexOf(typed.toLowerCase());
    if (!typed || i < 0) return esc(s);
    return `${esc(s.slice(0, i))}<b>${esc(s.slice(i, i + typed.length))}</b>${esc(s.slice(i + typed.length))}`;
  }

  function refresh() {
    if (!st) return;
    const text = q.value.trim();
    $('#bclear').hidden = !q.value;
    rows = [];
    if (text) {
      if (looksLikeAddress(text)) rows.push({ kind: 'url', text });
      const seen = new Set([text.toLowerCase()]);
      if (st.remember) {
        for (const r of st.recent) {
          if (rows.length >= 4) break;
          const low = r.q.toLowerCase();
          if (low.includes(text.toLowerCase()) && !seen.has(low)) {
            seen.add(low);
            rows.push({ kind: 'recent', text: r.q });
          }
        }
      }
      rows.push({ kind: 'search', text });
      if (suggestions.text === text) {
        for (const s of suggestions.items) {
          const low = s.toLowerCase();
          if (!seen.has(low)) {
            seen.add(low);
            rows.push({ kind: 'suggest', text: s });
          }
        }
      }
    }
    const ul = $('#bsuggest');
    ul.textContent = '';
    at = -1;
    rows.forEach((r, i) => {
      const li = document.createElement('li');
      li.className = `s-${r.kind}`;
      li.setAttribute('role', 'option');
      const icon = r.kind === 'url' ? 'link' : r.kind === 'recent' ? 'reload' : 'search';
      const label = r.kind === 'url' ? esc(t('browser.openAddr', { addr: r.text }))
        : r.kind === 'search' ? esc(t('browser.searchFor', { engine: engineOf(st.engine).name, q: r.text }))
          : hl(r.text, text);
      li.innerHTML = `<span class="s-ico">${ico(icon)}</span><span class="s-txt">${label}</span>`;
      li.addEventListener('mousedown', (e) => {
        e.preventDefault();
        pick(r);
      });
      li.addEventListener('mousemove', () => at !== i && mark(i));
      ul.append(li);
    });
    const open = rows.length > 0 && document.activeElement === q;
    ul.hidden = !open;
    $('#bbox').classList.toggle('open', open);
  }
  function mark(i) {
    at = i;
    [...$('#bsuggest').children].forEach((li, n) => li.classList.toggle('at', n === i));
  }
  function pick(r) {
    chatdock.send('browser:go', r.text);
    q.value = '';
    suggestions = { text: '', items: [] };
    refresh();
    q.blur();
  }
  q.addEventListener('input', () => {
    refresh();
    clearTimeout(timer);
    const text = q.value.trim();
    if (text && st && st.suggest && !looksLikeAddress(text)) timer = setTimeout(() => chatdock.send('browser:suggest', text), 150);
  });
  q.addEventListener('focus', refresh);
  q.addEventListener('blur', () => setTimeout(refresh, 0));
  q.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      if (!rows.length) return;
      mark((at + (e.key === 'ArrowDown' ? 1 : -1) + rows.length) % rows.length);
    } else if (e.key === 'Escape') {
      // one Esc closes the list, the next clears the text (two quick ones still hide the panel)
      if (!$('#bsuggest').hidden) {
        $('#bsuggest').hidden = true;
        $('#bbox').classList.remove('open');
      } else {
        q.value = '';
        refresh();
      }
    }
  });
  $('#bbox').addEventListener('submit', (e) => {
    e.preventDefault();
    if (at >= 0 && rows[at]) pick(rows[at]);
    else if (q.value.trim()) pick({ kind: 'search', text: q.value.trim() });
  });
  $('#bclear').addEventListener('click', () => {
    q.value = '';
    refresh();
    q.focus();
  });
  chatdock.on('browser:suggest', (a) => {
    if (!a || a.text !== q.value.trim()) return; // an answer to older text
    suggestions = { text: a.text, items: Array.isArray(a.items) ? a.items : [] };
    refresh();
  });

  chatdock.on('browser:focus-search', () => { if (st && st.start) q.focus(); });
  chatdock.on('browser:focus-address', () => $('#addr-in').focus());

  // ---------------------------------------------------------------- state
  function renderAll(entering) {
    renderTabs();
    renderNav();
    renderNotice();
    $('#bhome').hidden = !st.start;
    if (st.start) {
      renderEngines();
      renderTiles();
      renderRecent();
      refresh();
    } else {
      $('#btile-sheet').hidden = true; // (a web page covers it anyway)
    }
    if (entering) {
      const sec = $('#browser');
      sec.classList.remove('in');
      void sec.offsetWidth;
      sec.classList.add('in');
    }
  }
  chatdock.on('browser:state', (s) => {
    const entering = s.start && !(st && st.start);
    st = s;
    renderAll(entering);
  });
  // a new language (panel.js translatePage): the strip and the start page are drawn again
  window.browserRelang = () => {
    lastTabs = '';
    if (st) renderAll(false);
  };
})();
