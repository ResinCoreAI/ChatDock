import { test } from 'node:test';
import assert from 'node:assert/strict';
import { latestStable, summaryLine } from '../assets/lib.js';

test('the latest release is the first one that is neither a draft nor a pre-release', () => {
  const list = [
    { tag_name: 'v1.9.0-beta', prerelease: true },
    { tag_name: 'v1.8.2', draft: true },
    { tag_name: 'v1.8.1' },
    { tag_name: 'v1.8.0' },
  ];
  assert.equal(latestStable(list).tag_name, 'v1.8.1');
  assert.equal(latestStable([]), null);
  assert.equal(latestStable(undefined), null);
});

test('a summary line joins the leads of the first section', () => {
  const body = '### New\n- **Intro card.** text\n- **A “+” button.** text\n- **Right-click menus.** text\n### Fixed\n- **Tab click.** text';
  assert.equal(summaryLine(body, 2), 'Intro card · A “+” button');
  assert.equal(summaryLine(body), 'Intro card · A “+” button · Right-click menus');
});

test('items without a lead use their text, and an empty body gives an empty line', () => {
  assert.equal(summaryLine('- plain change\n- another one'), 'plain change · another one');
  assert.equal(summaryLine(''), '');
});
