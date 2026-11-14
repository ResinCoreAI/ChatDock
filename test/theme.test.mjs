import { test } from 'node:test';
import assert from 'node:assert/strict';
import { pickTheme } from '../assets/lib.js';

test('with no saved choice the theme follows the system', () => {
  assert.equal(pickTheme(null, true), 'dark');
  assert.equal(pickTheme(null, false), 'light');
});

test('a saved choice wins over the system', () => {
  assert.equal(pickTheme('light', true), 'light');
  assert.equal(pickTheme('dark', false), 'dark');
});

test('a saved value that is not a theme is ignored', () => {
  assert.equal(pickTheme('purple', true), 'dark');
});
