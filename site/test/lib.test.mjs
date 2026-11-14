import { test } from 'node:test';
import assert from 'node:assert/strict';
import { pickLang, parseRelease, highlights, installer, formatSize, versionOf, mmss } from '../assets/lib.js';

test('a saved language wins', () => {
  assert.equal(pickLang('ja', ['th-TH']), 'ja');
});

test('a saved value that is not a language is ignored', () => {
  assert.equal(pickLang('xx', ['de-DE']), 'de');
});

test('the first browser language we have is used, region or not', () => {
  assert.equal(pickLang(null, ['fr-FR', 'zh-CN', 'th']), 'zh');
  assert.equal(pickLang(null, ['TH-th']), 'th');
});

test('anything else falls back to English', () => {
  assert.equal(pickLang(null, ['fr', 'es']), 'en');
  assert.equal(pickLang(undefined, undefined), 'en');
});

const BODY = `### New in Beta Build 1.8.1
- **An intro when ChatDock starts.** A small card shows the logo while your chats **really** load.
- **A “+” after the app tabs.** It lists the apps that are off.
- A line without a bold lead.

### Fixed
- **The chat stays open when you click the white tab.** Windows handed the focus back.

Updating from 1.8: ChatDock updates itself.`;

test('a release body splits into its headed sections and items', () => {
  const r = parseRelease(BODY);
  assert.deepEqual(r.sections.map((s) => s.title), ['New in Beta Build 1.8.1', 'Fixed']);
  assert.deepEqual(r.sections[0].items[0], {
    h: 'An intro when ChatDock starts',
    p: 'A small card shows the logo while your chats really load.',
  });
  assert.deepEqual(r.sections[0].items[2], { h: '', p: 'A line without a bold lead.' });
  assert.equal(r.sections[1].items.length, 1);
});

test('loose lines after the lists are kept as paragraphs', () => {
  assert.deepEqual(parseRelease(BODY).paras, ['Updating from 1.8: ChatDock updates itself.']);
});

test('an empty or missing body gives nothing, not an error', () => {
  assert.deepEqual(parseRelease(''), { sections: [], paras: [] });
  assert.deepEqual(parseRelease(null), { sections: [], paras: [] });
});

test('highlights are the first items of the first section', () => {
  assert.deepEqual(highlights(BODY, 2).map((i) => i.h), ['An intro when ChatDock starts', 'A “+” after the app tabs']);
});

test('the installer is the -setup.exe asset, not the signature or feeds', () => {
  const assets = [
    { name: 'latest.json', browser_download_url: 'u1', size: 1 },
    { name: 'ChatDock_1.8.1_x64-setup.exe.sig', browser_download_url: 'u2', size: 420 },
    { name: 'ChatDock_1.8.1_x64-setup.exe', browser_download_url: 'u3', size: 2663655 },
  ];
  assert.deepEqual(installer(assets), { name: 'ChatDock_1.8.1_x64-setup.exe', url: 'u3', size: 2663655 });
  assert.equal(installer([]), null);
  assert.equal(installer(undefined), null);
});

test('sizes read like Windows shows them', () => {
  assert.equal(formatSize(2663655), '2.5 MB');
  assert.equal(formatSize(0), '');
});

test('a tag becomes a version', () => {
  assert.equal(versionOf('v1.8.1'), '1.8.1');
  assert.equal(versionOf('1.9.0'), '1.9.0');
  assert.equal(versionOf(''), '');
});

test('seconds read as minutes and seconds', () => {
  assert.equal(mmss(0), '0:00');
  assert.equal(mmss(184), '3:04');
  assert.equal(mmss(59.9), '0:59');
});
