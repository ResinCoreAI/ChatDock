'use strict';
// Browser tab mock: everything here is pretend except the layout and the behaviour to judge.
const $ = (s) => document.querySelector(s);
const ico = (n) => window.iconHTML(n);

// ---- theme: follows the system, the button on the desktop flips it ----
const root = document.documentElement;
if (/still/.test(location.search)) root.classList.add('still'); // screenshots: no motion
root.dataset.theme = matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
$('#theme-toggle').onclick = () => { root.dataset.theme = root.dataset.theme === 'dark' ? 'light' : 'dark'; };

// ---- search engines (badges, not their logos) ----
const badge = (bg, fg, ch) => `<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="11" fill="${bg}"/><text x="12" y="16.4" text-anchor="middle" font-family="Segoe UI, sans-serif" font-weight="800" font-size="12.5" fill="${fg}">${ch}</text></svg>`;
const ENGINES = [
  { id: 'google', name: 'Google', svg: badge('#4285f4', '#fff', 'G'), url: 'https://www.google.com/search?q=' },
  { id: 'bing', name: 'Bing', svg: badge('#008373', '#fff', 'b'), url: 'https://www.bing.com/search?q=' },
  { id: 'ddg', name: 'DuckDuckGo', svg: badge('#de5833', '#fff', 'D'), url: 'https://duckduckgo.com/?q=' },
  { id: 'youtube', name: 'YouTube', svg: `<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="1" y="4.5" width="22" height="15" rx="4.5" fill="#ff0000"/><path d="M10 8.6v6.8l5.8-3.4z" fill="#fff"/></svg>`, url: 'https://www.youtube.com/results?search_query=' },
  { id: 'brave', name: 'Brave', svg: badge('#fb542b', '#fff', 'B'), url: 'https://search.brave.com/search?q=' },
  { id: 'wiki', name: 'Wikipedia', svg: badge('#e8e8ea', '#202122', 'W'), url: 'https://th.wikipedia.org/w/index.php?search=' },
];
const engineOf = (id) => ENGINES.find((e) => e.id === id) || ENGINES[0];

// ---- state (the real app keeps it in settings.json; here: this browser only) ----
const load = (k, d) => { try { return JSON.parse(localStorage.getItem('mock-b:' + k)) ?? d; } catch { return d; } };
const save = (k, v) => { try { localStorage.setItem('mock-b:' + k, JSON.stringify(v)); } catch { /* fine */ } };
let engine = load('engine', 'google');
let recent = load('recent', [
  { q: 'สภาพอากาศวันนี้', e: 'google' }, { q: 'valorant patch notes', e: 'google' },
  { q: 'lofi hip hop', e: 'youtube' }, { q: 'ราคาการ์ดจอ rtx 5070', e: 'google' },
]);
let tiles = load('tiles', [
  { n: 'YouTube', u: 'youtube.com', c: '#ff0033' }, { n: 'Gmail', u: 'mail.google.com', c: '#ea4335' },
  { n: 'GitHub', u: 'github.com', c: '#24292f' }, { n: 'Twitch', u: 'twitch.tv', c: '#9146ff' },
  { n: 'Shopee', u: 'shopee.co.th', c: '#ee4d2d' }, { n: 'Netflix', u: 'netflix.com', c: '#e50914' },
]);
let tabs = load('tabs', ['facebook', 'instagram', 'discord']);
let active = load('active', 'facebook');
const opts = load('opts', { suggest: true, recent: true });

const APPS = {
  instagram: 'Instagram', facebook: 'Facebook', x: 'X', discord: 'Discord', telegram: 'Telegram',
  whatsapp: 'WhatsApp', spotify: 'Spotify', browser: 'Browser',
};

// ---- tab strip ----
function renderTabs(added) {
  const nav = $('#apps');
  nav.querySelectorAll('.app').forEach((b) => b.remove());
  for (const id of tabs) {
    const b = document.createElement('button');
    b.className = `app${id === active ? ' active' : ''}${id === 'browser' ? ' browser' : ''}${id === added ? ' new' : ''}`;
    b.dataset.id = id;
    b.title = APPS[id];
    b.innerHTML = `<span class="ico">${ico(id === 'browser' ? 'search' : id)}</span><span class="name">${APPS[id]}</span>`;
    b.onclick = () => openTab(id);
    b.oncontextmenu = (e) => { e.preventDefault(); tabMenu(id, b); };
    nav.append(b);
  }
  requestAnimationFrame(placeGlider);
}
function placeGlider() {
  const a = $('#apps .app.active'); const g = $('#glider');
  if (!a) { g.style.width = '0'; return; }
  g.style.width = a.offsetWidth + 'px';
  g.style.transform = `translateX(${a.offsetLeft}px)`;
}
function openTab(id) {
  active = id; save('active', id);
  renderTabs();
  const isB = id === 'browser';
  $('#browser').hidden = !isB; $('#chat').hidden = isB;
  if (isB) showPage(true);
  else fakeChat(id);
}
function fakeChat(id) {
  const colors = ['#f4a6c6', '#8ecae6', '#ffd166', '#7ed3b2', '#c9b6f2', '#ff9f7f'];
  $('.fake-list').innerHTML = colors.map((c) => `<div class="fake-row"><i style="background:${c}"></i><span><b></b><em></em></span></div>`).join('')
    + `<p class="fake-cap">(หน้าแชทของ ${APPS[id]} จะอยู่ตรงนี้)</p>`;
}

// ---- "+" menu and right-click menu ----
const menu = $('#menu');
function showMenu(items, x, onPick) {
  menu.innerHTML = '';
  items.forEach((it, i) => {
    if (it.sep) { menu.append(Object.assign(document.createElement('div'), { className: 'sep' })); return; }
    if (it.cap) { menu.append(Object.assign(document.createElement('div'), { className: 'menu-cap', textContent: it.cap })); return; }
    const r = document.createElement('button');
    r.className = `item${it.cls ? ' ' + it.cls : ''}`;
    r.style.animationDelay = `${i * 18}ms`;
    r.innerHTML = `<span class="ico">${ico(it.icon)}</span><span class="label">${it.label}</span>${it.hint ? `<span class="hint">${it.hint}</span>` : ''}`;
    r.onclick = () => { closeMenu(); onPick(it.id); };
    menu.append(r);
  });
  menu.hidden = false;
  const pw = $('#panel').clientWidth;
  menu.style.left = Math.max(6, Math.min(x, pw - menu.offsetWidth - 6)) + 'px';
}
function closeMenu() { menu.hidden = true; $('#add-app').classList.remove('open'); }
$('#add-app').onclick = (e) => {
  e.stopPropagation();
  if (!menu.hidden) return closeMenu();
  $('#add-app').classList.add('open');
  const off = Object.keys(APPS).filter((id) => id !== 'browser' && !tabs.includes(id));
  const items = [];
  if (!tabs.includes('browser')) items.push({ id: 'browser', icon: 'search', label: 'Browser', hint: 'ค้นหาเว็บ', cls: 'browser' }, { sep: true });
  off.forEach((id) => items.push({ id, icon: id, label: APPS[id] }));
  if (!items.length) items.push({ cap: 'เปิดครบทุกแอปแล้ว' });
  const b = $('#add-app');
  showMenu(items, b.offsetLeft - 6, (id) => {
    tabs = id === 'browser' ? [...tabs, 'browser'] : [...tabs, id];
    save('tabs', tabs);
    active = id; renderTabs(id); openTab(id);
  });
};
function tabMenu(id, btn) {
  showMenu([{ id: 'close', icon: 'close', label: `ปิดแท็บ ${APPS[id]}`, cls: 'danger' }], btn.offsetLeft, () => {
    tabs = tabs.filter((t) => t !== id); save('tabs', tabs);
    if (active === id) active = tabs[0] || 'facebook';
    openTab(active);
  });
}
document.addEventListener('click', (e) => { if (!menu.contains(e.target)) closeMenu(); });
addEventListener('resize', placeGlider);

// ---- engines ----
function renderEngines() {
  $('#engine-btn').innerHTML = engineOf(engine).svg;
  $('#engine-btn').title = `เครื่องมือค้นหา: ${engineOf(engine).name}`;
  $('#engines').innerHTML = '';
  for (const e of ENGINES) {
    const b = document.createElement('button');
    b.type = 'button'; b.className = 'eng'; b.setAttribute('role', 'radio');
    b.setAttribute('aria-checked', String(e.id === engine));
    b.innerHTML = `${e.svg}<span>${e.name}</span>`;
    b.onclick = () => { engine = e.id; save('engine', engine); renderEngines(); $('#q').focus(); refresh(); };
    $('#engines').append(b);
  }
}
// the engine badge in the box steps to the next engine
$('#engine-btn').onclick = () => {
  const i = ENGINES.findIndex((e) => e.id === engine);
  engine = ENGINES[(i + 1) % ENGINES.length].id; save('engine', engine); renderEngines(); $('#q').focus(); refresh();
};

// ---- is it an address? ----
function asUrl(s) {
  const t = s.trim();
  if (!t || /\s/.test(t)) return null;
  if (/^https?:\/\//i.test(t)) return t;
  if (/^localhost(:\d+)?(\/|$)/i.test(t)) return 'http://' + t;
  if (/^[\w-]+(\.[\w-]+)*\.[a-z]{2,}(:\d+)?(\/\S*)?$/i.test(t)) return 'https://' + t;
  return null;
}

// ---- suggestions (pretend: the real ones come from the chosen engine) ----
const TAILS = ['', ' คือ', ' ราคา', ' รีวิว', ' วิธีใช้', ' 2026', ' ล่าสุด', ' pantip'];
function fakeSuggest(q) {
  const t = q.trim(); if (!t) return [];
  return TAILS.map((x) => t + x).filter((s, i, a) => a.indexOf(s) === i).slice(1, 6);
}
const esc = (s) => s.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
function hl(s, q) {
  const i = s.toLowerCase().indexOf(q.toLowerCase());
  return i < 0 ? esc(s) : esc(s.slice(0, i + q.length)) + '<b>' + esc(s.slice(i + q.length)) + '</b>';
}

let rows = []; let at = -1;
function refresh() {
  const q = $('#q').value;
  $('#clear').hidden = !q;
  const list = $('#suggest');
  rows = [];
  if (!q.trim()) { list.hidden = true; $('#box').classList.remove('open'); return; }
  const url = asUrl(q);
  if (url) rows.push({ kind: 'url', text: q.trim(), go: url });
  if (opts.recent) recent.filter((r) => r.q.toLowerCase().includes(q.trim().toLowerCase()) && r.q !== q.trim()).slice(0, 3)
    .forEach((r) => rows.push({ kind: 'recent', text: r.q }));
  rows.push({ kind: 'search', text: q.trim() });
  if (opts.suggest && !url) fakeSuggest(q).forEach((s) => rows.push({ kind: 'sug', text: s }));
  list.innerHTML = '';
  rows.forEach((r, i) => {
    const li = document.createElement('li');
    li.className = r.kind === 'url' ? 'url' : '';
    li.setAttribute('role', 'option');
    const icon = r.kind === 'url' ? 'external' : r.kind === 'recent' ? 'reload' : 'search';
    const tag = r.kind === 'url' ? 'เปิดเว็บ' : r.kind === 'recent' ? 'เคยค้นหา' : r.kind === 'search' ? engineOf(engine).name : '';
    const txt = r.kind === 'url' ? esc(r.text) : r.kind === 'search' ? `<b>${esc(r.text)}</b>` : hl(r.text, q.trim());
    li.innerHTML = `<span class="s-ico">${ico(icon)}</span><span class="s-txt">${txt}</span><span class="s-tag">${tag}</span>`;
    li.onmousedown = (e) => { e.preventDefault(); pick(r); };
    li.onmouseenter = () => mark(i);
    list.append(li);
  });
  list.hidden = false; $('#box').classList.add('open');
  mark(-1);
}
function mark(i) { at = i; [...$('#suggest').children].forEach((li, n) => li.classList.toggle('at', n === i)); }
function pick(r) { r.kind === 'url' ? openUrl(r.go) : search(r.text); }

function search(q, eng = engine) {
  const t = q.trim(); if (!t) return;
  const e = engineOf(eng);
  if (opts.recent) {
    recent = [{ q: t, e: e.id }, ...recent.filter((r) => r.q !== t)].slice(0, 8);
    save('recent', recent); renderRecent();
  }
  $('#q').value = ''; refresh();
  go({ type: 'results', q: t, e: e.id });
}
function openUrl(url) {
  $('#q').value = ''; refresh();
  go({ type: 'site', url });
}
function toast(html) {
  const tst = $('#toast');
  tst.innerHTML = html;
  tst.hidden = false; tst.style.animation = 'none'; void tst.offsetWidth; tst.style.animation = '';
  clearTimeout(toast.t); toast.t = setTimeout(() => { tst.hidden = true; }, 2600);
}

const q = $('#q');
q.addEventListener('input', refresh);
q.addEventListener('focus', refresh);
q.addEventListener('blur', () => setTimeout(() => { $('#suggest').hidden = true; $('#box').classList.remove('open'); }, 120));
q.addEventListener('keydown', (e) => {
  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    if (!rows.length) return;
    e.preventDefault();
    const n = e.key === 'ArrowDown' ? (at + 1) % rows.length : (at <= 0 ? rows.length - 1 : at - 1);
    mark(n);
  } else if (e.key === 'Escape') {
    if (!$('#suggest').hidden) { $('#suggest').hidden = true; $('#box').classList.remove('open'); } else { q.value = ''; refresh(); }
  }
});
$('#box').addEventListener('submit', (e) => {
  e.preventDefault();
  if (at >= 0 && rows[at]) return pick(rows[at]);
  const url = asUrl(q.value);
  url ? openUrl(url) : search(q.value);
});
$('#clear').onclick = () => { q.value = ''; refresh(); q.focus(); };

// ---- recent ----
function renderRecent() {
  const ul = $('#recent'); ul.innerHTML = '';
  const show = opts.recent ? recent : [];
  $('#recent-empty').hidden = show.length > 0;
  $('#recent-empty').textContent = opts.recent ? 'ยังไม่มีคำที่ค้นหา' : 'ปิดการจำคำที่ค้นหาอยู่ (เปิดได้ที่ปุ่มฟันเฟือง)';
  $('#clear-recent').hidden = !show.length;
  show.forEach((r) => {
    const li = document.createElement('li');
    li.innerHTML = `<span class="r-ico">${ico('reload')}</span><span class="r-txt">${esc(r.q)}</span><span class="r-eng" title="${engineOf(r.e).name}">${engineOf(r.e).svg}</span><button class="r-x" title="ลบ">${ico('close')}</button>`;
    li.onclick = () => search(r.q, r.e);
    li.querySelector('.r-x').onclick = (e) => {
      e.stopPropagation(); li.classList.add('gone');
      setTimeout(() => { recent = recent.filter((x) => x !== r); save('recent', recent); renderRecent(); }, 230);
    };
    ul.append(li);
  });
}
$('#clear-recent').onclick = () => { recent = []; save('recent', recent); renderRecent(); };

// ---- shortcuts ----
let editing = -1;
function renderTiles() {
  const box = $('#tiles'); box.innerHTML = '';
  tiles.forEach((t, i) => {
    const b = document.createElement('button');
    b.className = 'tile'; b.title = t.u;
    b.innerHTML = `<span class="fav" style="background:${t.c}">${esc(t.n[0].toUpperCase())}</span><span class="t-name">${esc(t.n)}</span><span class="t-more" title="แก้ไข">${ico('dots')}</span>`;
    b.onclick = (e) => { if (e.target.closest('.t-more')) return editTile(i); openUrl(asUrl(t.u) || 'https://' + t.u); };
    b.oncontextmenu = (e) => { e.preventDefault(); editTile(i); };
    box.append(b);
  });
  if (tiles.length < 12) {
    const add = document.createElement('button');
    add.className = 'tile add';
    add.innerHTML = `<span class="fav">${ico('plus')}</span><span class="t-name">เพิ่ม</span>`;
    add.onclick = () => editTile(-1);
    box.append(add);
  }
}
function editTile(i) {
  editing = i;
  $('#tile-title').textContent = i < 0 ? 'เพิ่มทางลัด' : 'แก้ไขทางลัด';
  $('#tile-name').value = i < 0 ? '' : tiles[i].n;
  $('#tile-url').value = i < 0 ? '' : tiles[i].u;
  let del = $('#tile-del');
  if (i >= 0 && !del) {
    del = Object.assign(document.createElement('button'), { type: 'button', id: 'tile-del', className: 'ghost', textContent: 'ลบ' });
    del.style.marginRight = 'auto'; del.style.color = 'var(--danger)';
    $('#tile-form .sheet-acts').prepend(del);
  }
  if (del) { del.hidden = i < 0; del.onclick = () => { tiles.splice(editing, 1); save('tiles', tiles); renderTiles(); $('#tile-sheet').hidden = true; }; }
  $('#tile-sheet').hidden = false;
  setTimeout(() => $('#tile-name').focus(), 30);
}
$('#tile-cancel').onclick = () => { $('#tile-sheet').hidden = true; };
$('#tile-form').onsubmit = (e) => {
  e.preventDefault();
  const n = $('#tile-name').value.trim(); const u = $('#tile-url').value.trim().replace(/^https?:\/\//i, '');
  if (!n || !u) return;
  const palette = ['#0866ff', '#16a34a', '#f59e0b', '#db2777', '#7c3aed', '#0891b2', '#ea580c'];
  if (editing < 0) tiles.push({ n, u, c: palette[tiles.length % palette.length] });
  else tiles[editing] = { ...tiles[editing], n, u };
  save('tiles', tiles); renderTiles(); $('#tile-sheet').hidden = true;
};

// ---- settings for the tab (gear while on Browser) ----
$('#gear').onclick = () => {
  if (active !== 'browser') return;
  $('#opt-suggest').checked = opts.suggest; $('#opt-recent').checked = opts.recent;
  $('#set-sheet').hidden = false;
};
$('#set-done').onclick = () => {
  opts.suggest = $('#opt-suggest').checked; opts.recent = $('#opt-recent').checked;
  save('opts', opts); $('#set-sheet').hidden = true; renderRecent();
};
document.querySelectorAll('.sheet').forEach((s) => s.addEventListener('click', (e) => { if (e.target === s) s.hidden = true; }));
addEventListener('keydown', (e) => { if (e.key === 'Escape') { document.querySelectorAll('.sheet').forEach((s) => { s.hidden = true; }); closeMenu(); } });

// ---- the Browser's own tabs: each has its history; a page is home, search results, or a site ----
const host = (u) => { try { return new URL(u).hostname.replace(/^www\./, ''); } catch { return u; } };
const COLORS = ['#0866ff', '#16a34a', '#f59e0b', '#db2777', '#7c3aed', '#0891b2', '#ea580c', '#e11d48'];
const colorOf = (s) => COLORS[[...s].reduce((a, c) => a + c.charCodeAt(0), 0) % COLORS.length];
const urlOf = (p) => p.type === 'home' ? '' : p.type === 'results' ? engineOf(p.e).url + encodeURIComponent(p.q) : p.url;
const titleOf = (p) => p.type === 'home' ? 'หน้าแรก' : p.type === 'results' ? `${p.q} - ${engineOf(p.e).name}` : (p.title || host(p.url));
let btabs = load('btabs', [{ hist: [{ type: 'home' }], i: 0 }]);
let bon = Math.min(load('bon', 0), btabs.length - 1);
const cur = () => btabs[bon];
const page = () => cur().hist[cur().i];
const keep = () => { save('btabs', btabs); save('bon', bon); };

function go(p, newTab) {
  if (newTab) { btabs.push({ hist: [p], i: 0 }); bon = btabs.length - 1; }
  else { const t = cur(); t.hist = t.hist.slice(0, t.i + 1); t.hist.push(p); t.i = t.hist.length - 1; }
  keep(); showPage(false, true);
}
function renderBtabs() {
  const bar = $('#btabs'); bar.innerHTML = '';
  btabs.forEach((t, n) => {
    const p = t.hist[t.i];
    const b = document.createElement('button');
    b.className = `bt${n === bon ? ' on' : ''}`; b.setAttribute('role', 'tab'); b.title = titleOf(p);
    const f = p.type === 'home' ? ico('search') : p.type === 'results' ? engineOf(p.e).svg : esc(host(p.url)[0].toUpperCase());
    b.innerHTML = `<span class="f" style="${p.type === 'site' ? `background:${colorOf(host(p.url))}` : ''}">${f}</span><span class="t">${esc(titleOf(p))}</span><span class="x" title="ปิดแท็บ">${ico('close')}</span>`;
    b.onclick = (e) => {
      if (e.target.closest('.x')) return closeBtab(n);
      bon = n; keep(); showPage();
    };
    b.onauxclick = (e) => { if (e.button === 1) closeBtab(n); };
    bar.append(b);
  });
  const add = document.createElement('button');
  add.className = 'bt-new'; add.title = 'แท็บใหม่'; add.innerHTML = ico('plus');
  add.onclick = () => go({ type: 'home' }, true);
  bar.append(add);
  bar.querySelector('.bt.on')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
}
function closeBtab(n) {
  btabs.splice(n, 1);
  if (!btabs.length) btabs = [{ hist: [{ type: 'home' }], i: 0 }];
  if (n < bon) bon--;
  bon = Math.max(0, Math.min(bon, btabs.length - 1));
  keep(); showPage();
}
function showPage(entering, loading) {
  const p = page();
  renderBtabs();
  $('#nav-back').disabled = cur().i === 0;
  $('#nav-fwd').disabled = cur().i >= cur().hist.length - 1;
  $('#nav-out').disabled = p.type === 'home';
  $('#addr-in').value = urlOf(p);
  $('#addr-ico').innerHTML = p.type === 'home' ? ico('search') : ico('lock');
  const home = p.type === 'home';
  $('#home').hidden = !home; $('#web').hidden = home;
  if (home) {
    if (entering || loading) { const b = $('#browser'); b.classList.remove('in'); void b.offsetWidth; b.classList.add('in'); }
    setTimeout(() => $('#q').focus(), 60);
  } else {
    drawWeb(p);
    if (loading) {
      const pr = $('#bprog'); pr.classList.remove('go'); void pr.offsetWidth; pr.classList.add('go');
      const tab = $('#btabs .bt.on'); tab?.classList.add('spin'); setTimeout(() => tab?.classList.remove('spin'), 600);
    }
  }
}
const SITES = ['pantip.com', 'reddit.com', 'gamerant.com', 'fandom.com', 'youtube.com', 'wikipedia.org', 'steamcommunity.com', 'ign.com'];
function drawWeb(p) {
  const w = $('#web'); w.scrollTop = 0;
  if (p.type === 'results') {
    const e = engineOf(p.e);
    const hits = SITES.slice(0, 6).map((s, k) => ({
      site: s, url: `https://${s}/${encodeURIComponent(p.q.toLowerCase().replace(/\s+/g, '-'))}${k ? '-' + k : ''}`,
      title: [`${p.q} คืออะไร สรุปครบในที่เดียว`, `${p.q} - กระทู้รวมคำถาม`, `วิธีผ่าน ${p.q} แบบละเอียด`, `${p.q} Wiki`, `${p.q} (คลิปสอน)`, `${p.q} อัปเดตล่าสุด`][k],
    }));
    w.innerHTML = `<div class="w-res"><div class="w-engine">${e.svg}<span>ผลการค้นหาจาก <b>${e.name}</b> (หน้าจริงของเว็บนั้นจะแสดงตรงนี้)</span></div>${hits.map((h, k) => `
      <div class="w-hit" data-k="${k}"><div class="u"><i style="background:${colorOf(h.site)}">${h.site[0].toUpperCase()}</i>${h.site}<span class="nt" title="เปิดในแท็บใหม่">${ico('plus')}แท็บใหม่</span></div>
      <h4>${esc(h.title)}</h4><p>ข้อความตัวอย่างจากหน้าเว็บที่เกี่ยวกับ “${esc(p.q)}” แสดงสองบรรทัดเพื่อให้เห็นว่าเป็นผลอะไร ก่อนจะกดเข้าไปอ่าน</p></div>`).join('')}</div>`;
    w.querySelectorAll('.w-hit').forEach((el) => {
      const h = hits[el.dataset.k];
      el.onclick = (ev) => go({ type: 'site', url: h.url, title: h.title }, !!ev.target.closest('.nt'));
      el.onauxclick = (ev) => { if (ev.button === 1) go({ type: 'site', url: h.url, title: h.title }, true); };
    });
  } else {
    const h = host(p.url);
    w.innerHTML = `<div class="w-site"><div class="hero" style="background:${colorOf(h)}">${esc(h)}</div><div class="body">
      <h2>${esc(p.title || h)}</h2>${[92, 100, 85, 97, 60, 100, 88, 74, 95, 40].map((x) => `<div class="ln" style="width:${x}%"></div>`).join('')}
      <p class="cap">(หน้าเว็บ ${esc(h)} จริงจะแสดงตรงนี้ เลื่อนอ่าน กดลิงก์ ล็อกอินได้เหมือนเบราว์เซอร์ปกติ)</p></div></div>`;
  }
}
$('#nav-back').onclick = () => { if (cur().i > 0) { cur().i--; keep(); showPage(false, true); } };
$('#nav-fwd').onclick = () => { if (cur().i < cur().hist.length - 1) { cur().i++; keep(); showPage(false, true); } };
$('#nav-reload').onclick = () => showPage(false, true);
$('#nav-home').onclick = () => { if (page().type !== 'home') go({ type: 'home' }); };
$('#nav-out').onclick = () => toast(`${ico('external')}<span>เปิดหน้านี้ใน Microsoft Edge<small>${esc(urlOf(page()))}</small></span>`);
$('#addr').onsubmit = (e) => {
  e.preventDefault();
  const v = $('#addr-in').value; if (!v.trim()) return;
  const url = asUrl(v);
  if (url) go({ type: 'site', url });
  else { search(v); }
  $('#addr-in').blur();
};
$('#addr-in').addEventListener('focus', () => $('#addr-in').select());
// Ctrl+T new tab, Ctrl+W close, Alt+Left/Right back/forward (while the Browser is open)
addEventListener('keydown', (e) => {
  if (active !== 'browser') return;
  const k = e.key.toLowerCase();
  if (e.ctrlKey && k === 't') { e.preventDefault(); go({ type: 'home' }, true); }
  else if (e.ctrlKey && k === 'w') { e.preventDefault(); closeBtab(bon); }
  else if (e.ctrlKey && k === 'l') { e.preventDefault(); $('#addr-in').focus(); }
  else if (e.altKey && e.key === 'ArrowLeft') $('#nav-back').click();
  else if (e.altKey && e.key === 'ArrowRight') $('#nav-fwd').click();
});

window.fillIcons();
renderEngines(); renderTiles(); renderRecent();
// #browser opens straight on the Browser tab (for screenshots and phone links)
if (location.hash === '#browser') { if (!tabs.includes('browser')) tabs = [...tabs, 'browser']; active = 'browser'; }
if (location.hash.startsWith('#q=')) { if (!tabs.includes('browser')) tabs = [...tabs, 'browser']; active = 'browser'; }
if (!tabs.includes(active)) active = tabs[0];
openTab(active);
// #q=words opens on search results (for screenshots)
if (location.hash.startsWith('#q=')) { btabs = [{ hist: [{ type: 'home' }], i: 0 }, { hist: [{ type: 'site', url: 'https://www.reddit.com/r/Eldenring' }], i: 0 }]; bon = 0; go({ type: 'results', q: decodeURIComponent(location.hash.slice(3)), e: 'google' }); }
// #type=words shows the suggestion list open (for screenshots)
if (location.hash.startsWith('#type=')) {
  if (!tabs.includes('browser')) { tabs = [...tabs, 'browser']; }
  btabs = [{ hist: [{ type: 'home' }], i: 0 }]; bon = 0; active = 'browser'; openTab('browser');
  { const i = $('#q'); i.value = decodeURIComponent(location.hash.slice(6)); refresh(); }
}
