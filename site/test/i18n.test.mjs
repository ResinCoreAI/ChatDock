import { test } from 'node:test';
import assert from 'node:assert/strict';
import { STRINGS, SCREEN, LANG_NAMES } from '../assets/i18n.js';
import { LANGS } from '../assets/lib.js';

test('every site language has a name, page text and screen text', () => {
  for (const l of LANGS) {
    assert.ok(LANG_NAMES[l], `name for ${l}`);
    assert.ok(STRINGS[l], `page text for ${l}`);
    assert.ok(SCREEN[l], `screen text for ${l}`);
  }
});

test('no language is missing a line that English has', () => {
  const keys = Object.keys(STRINGS.en).sort();
  for (const l of LANGS) assert.deepEqual(Object.keys(STRINGS[l]).sort(), keys, `page text in ${l}`);
});

test('the demo screen has every app, three quick replies and the music labels in each language', () => {
  for (const l of LANGS) {
    const s = SCREEN[l];
    assert.deepEqual(Object.keys(s.chats).sort(), ['discord', 'facebook', 'instagram', 'telegram', 'whatsapp', 'x'], l);
    assert.equal(s.quick.length, 3, l);
    assert.ok(s.reply && s.type && s.music.now && s.music.next, l);
  }
});

test('headlines keep their line break in every language', () => {
  for (const l of LANGS) assert.match(STRINGS[l].hero_title, /<br>/, l);
});
