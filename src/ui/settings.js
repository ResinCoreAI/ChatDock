'use strict';

// Settings screen inside the panel. The controls are plain HTML with data-* hooks:
//   data-pref="key"        checkbox / select / range, or a button with data-value  -> 'settings:set'
//   data-action="name"     button -> 'settings:action' (data-arg = its argument)
//   data-confirm="text"    risky action: the first click only arms the button for a few seconds
//   data-goto="section"    nav chip -> scroll to that section
// Values are checked again in the main process before anything uses them.

(() => {
  const root = document.getElementById('settings');
  const stage = document.getElementById('stage');
  const q = (sel) => root.querySelector(sel);
  const qa = (sel) => root.querySelectorAll(sel);
  let st = null;
  let appsKey = '';
  let displaysKey = '';
  let hotkeysBuilt = false;
  let shown = false;

  const CORNERS = {
    'top-left': 'มุมซ้ายบน', 'top-right': 'มุมขวาบน', 'bottom-left': 'มุมซ้ายล่าง', 'bottom-right': 'มุมขวาล่าง',
  };
  const clock = (ms) => new Date(ms).toLocaleTimeString('th-TH', { hour: '2-digit', minute: '2-digit' });
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
    if (el.dataset.appToggle) {
      chatdock.send('settings:app', el.dataset.appToggle, el.checked);
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
    if (btn.dataset.goto) {
      goto(btn.dataset.goto);
    } else if (btn.dataset.pref && btn.dataset.value !== undefined) {
      chatdock.send('settings:set', btn.dataset.pref, valueOf(btn, btn.dataset.value));
    } else if (btn.dataset.popupApp) {
      chatdock.send('settings:popup-app', btn.dataset.popupApp, !btn.classList.contains('on'));
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
    btn.textContent = btn.dataset.confirm;
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
  function buildOnce() {
    if (hotkeysBuilt) return;
    hotkeysBuilt = true;
    const sel = q('[data-hotkeys]');
    for (const h of st.hotkeys) sel.append(new Option(h.label, h.acc));
    sel.append(new Option('ไม่ใช้ปุ่มลัด', ''));
  }

  function renderDisplays() {
    const key = st.displays.map((d) => `${d.id}:${d.label}`).join('|');
    if (key === displaysKey) return;
    displaysKey = key;
    const sel = q('[data-displays]');
    sel.textContent = '';
    for (const d of st.displays) sel.append(new Option(d.label, String(d.id)));
  }

  function renderApps() {
    const card = q('[data-apps]');
    const chips = q('[data-popup-apps]');
    const key = st.catalog.map((a) => a.id).join(',');
    if (key !== appsKey) {
      appsKey = key;
      card.textContent = '';
      chips.textContent = '';
      for (const a of st.catalog) {
        const row = document.createElement('div');
        row.className = 'row app-row';
        row.dataset.app = a.id;
        const ico = document.createElement('span');
        ico.className = 'ico';
        ico.innerHTML = window.iconHTML(a.icon); // our own static SVG
        const label = document.createElement('div');
        label.className = 'label';
        const b = document.createElement('b');
        b.textContent = a.name;
        const small = document.createElement('small');
        label.append(b, small);
        const actions = document.createElement('div');
        actions.className = 'actions';
        const clear = document.createElement('button');
        clear.className = 'link-btn';
        clear.textContent = 'ล้างข้อมูล';
        clear.title = `ออกจากระบบ ${a.name} และลบคุกกี้/แคชของแอปนี้ในเครื่อง`;
        clear.dataset.action = 'clear-app';
        clear.dataset.arg = a.id;
        clear.dataset.confirm = 'กดอีกครั้งเพื่อยืนยัน';
        const sw = document.createElement('label');
        sw.className = 'switch';
        const input = document.createElement('input');
        input.type = 'checkbox';
        input.dataset.appToggle = a.id;
        input.setAttribute('aria-label', a.name);
        sw.append(input, document.createElement('span'));
        actions.append(clear, sw);
        row.append(ico, label, actions);
        card.append(row);

        const chip = document.createElement('button');
        chip.dataset.popupApp = a.id;
        const cico = document.createElement('span');
        cico.innerHTML = window.iconHTML(a.icon);
        chip.append(cico, document.createTextNode(a.name));
        chips.append(chip);
      }
    }
    const onCount = st.catalog.filter((a) => a.on).length;
    for (const a of st.catalog) {
      const row = card.querySelector(`[data-app="${a.id}"]`);
      const input = row.querySelector('input');
      input.checked = a.on;
      input.disabled = a.on && onCount === 1; // keep at least one app
      row.querySelector('small').textContent = input.disabled ? 'ต้องเปิดไว้อย่างน้อย 1 แอป' : a.on ? 'เปิดอยู่' : 'ปิดอยู่';
      const chip = chips.querySelector(`[data-popup-app="${a.id}"]`);
      chip.hidden = !a.on;
      chip.classList.toggle('on', a.popups);
      chip.classList.toggle('off', !a.popups);
      chip.title = a.popups ? `${a.name}: เด้งป๊อปอัพ (คลิกเพื่อปิด)` : `${a.name}: ไม่เด้ง (คลิกเพื่อเปิด)`;
    }
  }

  function renderPrefs(p) {
    for (const el of qa('[data-pref]')) {
      const key = el.dataset.pref;
      if (!(key in p)) continue;
      const v = p[key];
      if (el.dataset.value !== undefined) {
        const on = String(v) === el.dataset.value;
        el.classList.toggle('on', on);
        el.setAttribute('aria-checked', String(on));
      } else if (el.type === 'checkbox') {
        el.checked = !!v;
      } else if (el.type === 'range') {
        if (document.activeElement === el) continue; // don't fight the user's drag
        el.value = String(Math.round(Number(v) * Number(el.dataset.scale || 1)));
        const out = q(`[data-out="${key}"]`);
        if (out) out.textContent = `${el.value}%`;
      } else if (el.tagName === 'SELECT') {
        el.value = String(v);
      }
    }
  }

  function renderNotes() {
    const autostart = q('[data-pref="autostart"]');
    autostart.disabled = !st.autostartAvailable;
    setText('autostartNote', st.autostartAvailable
      ? 'รออยู่ที่ถาดไอคอน ไม่เด้งหน้าต่างตอนเปิดเครื่อง'
      : 'ใช้ได้เมื่อติดตั้งด้วยตัวติดตั้ง (ChatDock-Setup)');

    const p = st.prefs;
    if (p.hotkey && !st.hotkeyOk) setText('hotkeyNote', 'ใช้ไม่ได้ — โปรแกรมอื่นจองปุ่มนี้อยู่ ลองเลือกปุ่มอื่น', 'warn');
    else setText('hotkeyNote', p.hotkey ? 'กดได้ทุกที่ แม้กำลังเล่นเกม' : 'ปิดอยู่ — เปิดแชทจากขอบจอหรือไอคอนในถาดแทน', '');

    for (const el of qa('[data-show="multiDisplay"]')) el.hidden = st.displays.length < 2;
    for (const el of qa('[data-show="packaged"]')) el.hidden = !st.packaged;

    const until = st.dndUntil;
    const dnd = until === -1 || until > Date.now();
    setText('dnd', !dnd ? 'ปิดอยู่ — ป๊อปอัพเด้งตามปกติ'
      : until === -1 ? 'เปิดอยู่ จนกว่าจะปิดเอง — ไม่เด้งป๊อปอัพ (ตัวเลขยังขึ้นตามปกติ)'
        : `เงียบถึง ${clock(until)} น. — ไม่เด้งป๊อปอัพ (ตัวเลขยังขึ้นตามปกติ)`, dnd ? 'warn' : '');
    for (const b of qa('[data-dnd] button')) {
      const arg = Number(b.dataset.arg);
      b.classList.toggle('on', arg === 0 ? !dnd : arg === -1 && until === -1);
    }

    setText('cornerLabel', CORNERS[p.popupPosition] || '');
    for (const el of qa('[data-dim-when="popups-off"]')) el.classList.toggle('dim', !p.popups);
  }

  function renderSecurity() {
    const box = q('[data-sec]');
    box.classList.toggle('warn', !st.cookieEncryption);
    if (st.cookieEncryption) {
      setText('secTitle', 'คุกกี้และการล็อกอินถูกเข้ารหัสแล้ว');
      setText('secDesc', `เข้ารหัสด้วยกุญแจของบัญชี Windows นี้ (DPAPI) — ก๊อปไฟล์ไปเครื่องอื่นหรือบัญชีอื่นก็เปิดอ่านไม่ได้${
        st.cookiesMigrated ? '' : ' · กำลังเข้ารหัสคุกกี้ที่เก็บไว้จากเวอร์ชันก่อน…'}`);
    } else {
      setText('secTitle', 'โหมดนักพัฒนา: คุกกี้ยังไม่ถูกเข้ารหัส');
      setText('secDesc', 'การเข้ารหัสคุกกี้และการล็อกไฟล์โปรแกรมเปิดในตัวติดตั้งจริง (ChatDock-Setup) เท่านั้น');
    }
  }

  function renderUpdates() {
    const u = st.update;
    setText('version', `v${st.version}`);
    const bar = q('.uc-bar');
    let text = '';
    let cls = '';
    switch (u.status) {
      case 'dev': text = 'โหมดนักพัฒนา — ระบบอัปเดตทำงานในตัวที่ติดตั้งแล้วเท่านั้น'; break;
      case 'checking': text = 'กำลังตรวจหาอัปเดต…'; break;
      case 'latest': text = `เป็นเวอร์ชันล่าสุดแล้ว${u.checkedAt ? ` · ตรวจเมื่อ ${clock(u.checkedAt)} น.` : ''}`; cls = 'ok'; break;
      case 'available': text = `มีเวอร์ชันใหม่ ${u.version} — กดดาวน์โหลดได้เลย`; cls = 'new'; break;
      case 'downloading': text = `กำลังดาวน์โหลด ${u.version}… ${u.percent || 0}%`; cls = 'new'; break;
      case 'ready': text = `${u.version} ดาวน์โหลดแล้ว — กด “อัปเดตเลย” เมื่อสะดวก (ChatDock จะเปิดขึ้นมาเองใน ~5 วินาที)`; cls = 'new'; break;
      case 'error': {
        const offline = /net::ERR_|ENOTFOUND|ETIMEDOUT|ECONN|EAI_AGAIN|getaddrinfo|socket hang up/i.test(u.error || '');
        text = offline ? 'ต่อ GitHub ไม่ได้ — เช็กอินเทอร์เน็ต แล้วลองใหม่' : `ตรวจ/ดาวน์โหลดไม่สำเร็จ: ${u.error || 'ไม่ทราบสาเหตุ'} — ลองใหม่ได้`;
        cls = 'err';
        break;
      }
      default: text = st.prefs.updateAutoCheck ? 'จะตรวจหาอัปเดตให้เองหลังเปิดโปรแกรมสักครู่' : 'ยังไม่ได้ตรวจ';
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
    buildOnce();
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
