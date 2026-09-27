'use strict';

// Every piece of ChatDock's own text, in each language it speaks: the strings are in i18n-data.js
// (window.CHATDOCK_STRINGS, which the Rust side also reads); this file adds the helpers. Pages load
// both with <script src="i18n-data.js"></script><script src="i18n.js"></script> and get window.i18n.
// Keys are dotted names; {name} placeholders are filled in by t(key, { name: ... }).
// A key missing in a language falls back to English.

(function (root) {
  const LANGS = [
    { id: 'en', name: 'English', locale: 'en-US' },
    { id: 'th', name: 'ไทย', locale: 'th-TH' },
    { id: 'zh', name: '简体中文', locale: 'zh-CN' },
    { id: 'ja', name: '日本語', locale: 'ja-JP' },
    { id: 'de', name: 'Deutsch', locale: 'de-DE' },
  ];

  // The texts themselves: i18n-data.js (loaded before this file; the Rust side reads the same file).
  const STRINGS = root.CHATDOCK_STRINGS || { en: {} };

  const IDS = LANGS.map((l) => l.id);

  // Windows language (e.g. "th-TH", "zh-Hans-CN") -> one of ours, English otherwise
  function pick(tags) {
    for (const tag of [].concat(tags || [])) {
      const base = String(tag || '').toLowerCase().split(/[-_]/)[0];
      if (IDS.includes(base)) return base;
    }
    return 'en';
  }

  function make(lang) {
    const table = STRINGS[lang] || STRINGS.en;
    return function t(key, vars) {
      let s = table[key];
      if (s === undefined) s = STRINGS.en[key];
      if (s === undefined) return key;
      if (vars) s = s.replace(/\{(\w+)\}/g, (m, k) => (vars[k] !== undefined ? String(vars[k]) : m));
      return s;
    };
  }

  // Releases are called "Beta Build 1.4" for version 1.4.0: the version without its trailing ".0".
  function build(version) {
    const m = /^(\d+)\.(\d+)\.(\d+)/.exec(String(version || ''));
    if (!m) return null;
    return m[3] === '0' ? `${m[1]}.${m[2]}` : `${m[1]}.${m[2]}.${m[3]}`;
  }

  // "1.4.0" -> "Beta Build 1.4" (anything that isn't a version stays as it is)
  function buildName(t, version) {
    const n = build(version);
    return n ? t('build.name', { n }) : String(version || '');
  }

  const api = { LANGS, IDS, STRINGS, pick, make, build, buildName };
  if (typeof module === 'object' && module.exports) {
    module.exports = api;
    return;
  }

  // ---- ChatDock's pages: translate every element that asks for it
  //   data-i18n="key"        text          data-i18n-html="key"   our own markup (<b>, <kbd>, <small>)
  //   data-i18n-title="key"  tooltip       data-i18n-aria="key"   aria-label
  //   data-n="5"             fills {n}
  const page = {
    lang: 'en',
    locale: 'en-US',
    t: make('en'),
    vars: {}, // extra placeholders for data-i18n-html, e.g. { hotkey: '<span class="hotkey-keys"></span>' }
    set(lang) {
      if (!IDS.includes(lang)) lang = 'en';
      page.lang = lang;
      page.locale = (LANGS.find((l) => l.id === lang) || LANGS[0]).locale;
      page.t = make(lang);
      document.documentElement.lang = page.locale;
    },
    apply(scope) {
      const t = page.t;
      // "0.5" is written the local way ("0,5" in German)
      const num = (n) => (/^\d+\.\d+$/.test(n) ? Number(n).toLocaleString(page.locale) : n);
      const vars = (el) => (el.dataset.n !== undefined ? { ...page.vars, n: num(el.dataset.n) } : page.vars);
      for (const el of (scope || document).querySelectorAll('[data-i18n]')) el.textContent = t(el.dataset.i18n, vars(el));
      for (const el of (scope || document).querySelectorAll('[data-i18n-html]')) el.innerHTML = t(el.dataset.i18nHtml, vars(el));
      for (const el of (scope || document).querySelectorAll('[data-i18n-title]')) el.title = t(el.dataset.i18nTitle, vars(el));
      for (const el of (scope || document).querySelectorAll('[data-i18n-aria]')) el.setAttribute('aria-label', t(el.dataset.i18nAria, vars(el)));
    },
  };
  root.CHATDOCK_I18N = api;
  root.i18n = page;
})(typeof window !== 'undefined' ? window : globalThis);
