// Pure helpers for the site: no DOM, so node --test can check them.

export const LANGS = ['th', 'en', 'zh', 'ja', 'de'];

// A saved choice first, then the browser's languages in order, then English.
export function pickLang(saved, prefs) {
  if (LANGS.includes(saved)) return saved;
  for (const p of prefs || []) {
    const base = String(p).toLowerCase().split('-')[0];
    if (LANGS.includes(base)) return base;
  }
  return 'en';
}

const clean = (s) => s.replace(/\*\*/g, '').trim();

// A GitHub release body ("### heading", "- **Lead.** text") as sections of items, plus loose lines.
export function parseRelease(body) {
  const out = { sections: [], paras: [] };
  let cur = null;
  for (const raw of String(body || '').split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) continue;
    const head = line.match(/^#{1,6}\s+(.*)$/);
    if (head) {
      cur = { title: clean(head[1]), items: [] };
      out.sections.push(cur);
      continue;
    }
    const item = line.match(/^[-*]\s+(.*)$/);
    if (item) {
      if (!cur) {
        cur = { title: '', items: [] };
        out.sections.push(cur);
      }
      const lead = item[1].match(/^\*\*(.+?)\*\*\s*(.*)$/);
      cur.items.push(lead ? { h: clean(lead[1]).replace(/[.:]$/, ''), p: clean(lead[2]) } : { h: '', p: clean(item[1]) });
      continue;
    }
    out.paras.push(clean(line));
  }
  return out;
}

export function highlights(body, n = 5) {
  const first = parseRelease(body).sections[0];
  return first ? first.items.slice(0, n) : [];
}

export function installer(assets) {
  const a = (assets || []).find((x) => /-setup\.exe$/i.test(x.name || ''));
  return a ? { name: a.name, url: a.browser_download_url, size: a.size } : null;
}

// Windows counts a megabyte as 1024 × 1024 bytes.
export function formatSize(bytes) {
  return bytes > 0 ? `${(bytes / 1048576).toFixed(1)} MB` : '';
}

export const versionOf = (tag) => String(tag || '').replace(/^v/i, '');

export function mmss(sec) {
  const s = Math.max(0, Math.floor(sec));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

// The people who made ChatDock, from GitHub: contributors (bots left out, most commits first)
// joined with their public profiles. The bio's first line is the role, the rest is "about".
export function people(contributors, profiles) {
  return (contributors || [])
    .filter((c) => c && c.type !== 'Bot' && !/\[bot\]$/.test(c.login || ''))
    .sort((a, b) => (b.contributions || 0) - (a.contributions || 0))
    .map((c) => {
      const p = (profiles || {})[c.login] || {};
      const lines = String(p.bio || '').split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
      const avatar = c.avatar_url ? `${c.avatar_url}${c.avatar_url.includes('?') ? '&' : '?'}s=192` : '';
      return {
        login: c.login,
        name: p.name || c.login,
        role: lines[0] || '',
        about: lines.slice(1).join(' · '),
        commits: c.contributions || 0,
        url: c.html_url || `https://github.com/${c.login}`,
        avatar,
      };
    });
}

// The newest release people should download: not a draft, not a pre-release.
export function latestStable(list) {
  return (list || []).find((r) => r && !r.draft && !r.prerelease) || null;
}

// One line for a release in the update log: the leads of its first section.
export function summaryLine(body, n = 3) {
  return highlights(body, n).map((i) => i.h || i.p).join(' · ');
}
