// What both pages share: the language (menu, text, title), the header, and GitHub release data.
import { LANGS, pickLang } from './lib.js';
import { STRINGS, LANG_NAMES } from './i18n.js';

export const REPO = 'ResinCoreAI/ChatDock';
export const RELEASES = `https://github.com/${REPO}/releases`;
const KEY = 'chatdock-lang';
const subs = [];
let lang = 'en';

export const getLang = () => lang;
export const t = (k) => STRINGS[lang]?.[k] ?? STRINGS.en[k] ?? '';
export const onLang = (f) => subs.push(f);

// Thai has no spaces between words: mark where loanwords end so a line never breaks inside "แชท"
const thaiBreaks = (v) => v.replace(/(แชท|แท็บ|เมาส์|เกม|แอป|ล็อกอิน|อัปเดต|ดาวน์โหลด|โอเพนซอร์ส)/g, '​$1​');

function apply() {
  document.documentElement.lang = lang;
  document.querySelectorAll('[data-t]').forEach((el) => {
    const v = t(el.dataset.t);
    el.innerHTML = lang === 'th' ? thaiBreaks(v) : v;
  });
  document.querySelectorAll('[data-t-label]').forEach((el) => el.setAttribute('aria-label', t(el.dataset.tLabel)));
  document.title = t(document.body.dataset.title || 'meta_title');
  document.querySelector('meta[name="description"]')?.setAttribute('content', t('meta_desc'));
  const menu = document.querySelector('.lang');
  if (menu) {
    menu.querySelector('.now').textContent = LANG_NAMES[lang];
    menu.querySelectorAll('li button').forEach((b) => b.setAttribute('aria-current', String(b.dataset.l === lang)));
  }
  subs.forEach((f) => f(lang));
}

export function setLang(l) {
  lang = LANGS.includes(l) ? l : 'en';
  try { localStorage.setItem(KEY, lang); } catch { /* private window: the choice lasts this visit */ }
  apply();
}

function menu() {
  const box = document.querySelector('.lang');
  if (!box) return;
  const btn = box.querySelector('button');
  const ul = box.querySelector('ul');
  LANGS.forEach((l) => {
    const li = document.createElement('li');
    const b = document.createElement('button');
    b.type = 'button';
    b.dataset.l = l;
    b.lang = l;
    b.textContent = LANG_NAMES[l];
    b.addEventListener('click', () => { setLang(l); close(); btn.focus(); });
    li.append(b);
    ul.append(li);
  });
  const close = () => { box.classList.remove('open'); btn.setAttribute('aria-expanded', 'false'); };
  btn.addEventListener('click', () => {
    const on = !box.classList.contains('open');
    box.classList.toggle('open', on);
    btn.setAttribute('aria-expanded', String(on));
    if (on) ul.querySelector('[aria-current="true"]')?.focus();
  });
  document.addEventListener('click', (e) => { if (!box.contains(e.target)) close(); });
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape') close(); });
}

function header() {
  const h = document.querySelector('header.top');
  if (h) {
    const solid = () => h.classList.toggle('solid', scrollY > 8);
    addEventListener('scroll', solid, { passive: true });
    solid();
  }
  const brand = document.querySelector('.brand');
  brand?.addEventListener('click', (e) => {
    if (location.pathname.endsWith('/') || location.pathname.endsWith('index.html')) e.preventDefault();
    brand.classList.remove('spin');
    void brand.offsetWidth;
    brand.classList.add('spin');
    setTimeout(() => brand.classList.remove('spin'), 800);
    if (e.defaultPrevented) scrollTo({ top: 0 });
  });
}

export function initCommon() {
  let saved = null;
  try { saved = localStorage.getItem(KEY); } catch { /* fine */ }
  const asked = new URLSearchParams(location.search).get('lang');
  lang = pickLang(asked || saved, navigator.languages || [navigator.language]);
  menu();
  header();
  apply();
}

// GitHub allows 60 calls an hour without a login, so answers are kept for ten minutes
async function gh(path, root = `repos/${REPO}/`) {
  const key = 'gh:' + root + path;
  try {
    const hit = JSON.parse(sessionStorage.getItem(key) || 'null');
    if (hit && Date.now() - hit.at < 600000) return hit.data;
  } catch { /* fine */ }
  const r = await fetch(`https://api.github.com/${root}${path}`, { headers: { Accept: 'application/vnd.github+json' } });
  if (!r.ok) throw new Error(`GitHub ${r.status}`);
  const data = await r.json();
  try { sessionStorage.setItem(key, JSON.stringify({ at: Date.now(), data })); } catch { /* fine */ }
  return data;
}

export const latestRelease = () => gh('releases/latest').catch(() => null);
export const allReleases = () => gh('releases?per_page=30');

// who made it: the repo's contributors and each one's public profile (name, bio)
export async function team() {
  const list = await gh('contributors?per_page=20');
  const humans = (list || []).filter((c) => c.type !== 'Bot').slice(0, 8);
  const profiles = {};
  await Promise.all(humans.map((c) => gh(c.login, 'users/').then((p) => { profiles[c.login] = p; }).catch(() => {})));
  return { list, profiles };
}
