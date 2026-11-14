// "What's new": every release from GitHub, newest first. Release text is shown as text, never as HTML.
import { initCommon, onLang, t, getLang, allReleases, RELEASES } from './common.js';
import { parseRelease, installer, versionOf, formatSize } from './lib.js';

let list = null;
let failed = false;
const el = (tag, cls, text) => {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text != null) n.textContent = text;
  return n;
};

function render() {
  const box = document.getElementById('releases');
  const small = document.getElementById('notes-lang');
  small.textContent = list && list.length ? t('notes_en') : '';
  small.hidden = !small.textContent;
  box.textContent = '';
  if (failed || (list && !list.length)) {
    const p = el('p', 'state');
    const a = el('a', null, t('wn_error'));
    a.href = RELEASES;
    p.append(a);
    box.append(p);
    return;
  }
  if (!list) { box.append(el('p', 'state', t('wn_loading'))); return; }
  const date = new Intl.DateTimeFormat(getLang(), { year: 'numeric', month: 'long', day: 'numeric' });
  list.forEach((r, i) => {
    const art = el('article', 'release puff');
    const head = el('header');
    head.append(el('h2', null, r.name || `v${versionOf(r.tag_name)}`));
    if (i === 0 && !r.prerelease) head.append(el('span', 'badge', t('wn_latest')));
    if (r.published_at) {
      const tm = el('time', null, date.format(new Date(r.published_at)));
      tm.dateTime = r.published_at;
      head.append(tm);
    }
    art.append(head);
    const body = parseRelease(r.body);
    body.sections.forEach((sec) => {
      if (sec.title) art.append(el('h3', null, sec.title));
      const ul = el('ul');
      sec.items.forEach((it) => {
        const li = el('li');
        if (it.h) { li.append(el('b', null, it.h)); li.append(document.createTextNode(' ')); }
        li.append(document.createTextNode(it.p));
        ul.append(li);
      });
      art.append(ul);
    });
    body.paras.forEach((p) => art.append(el('p', 'rp', p)));
    const acts = el('div', 'acts');
    const ins = installer(r.assets);
    if (ins) {
      const a = el('a', 'btn', `${t('wn_download')} · ${formatSize(ins.size)}`);
      a.href = ins.url;
      acts.append(a);
    }
    const gh = el('a', null, t('wn_github'));
    gh.href = r.html_url || RELEASES;
    acts.append(gh);
    art.append(acts);
    document.getElementById('releases').append(art);
  });
}

onLang(render);
initCommon();
allReleases()
  .then((rows) => { list = (rows || []).filter((r) => !r.draft); render(); })
  .catch(() => { failed = true; render(); });
