import { test } from 'node:test';
import assert from 'node:assert/strict';
import { people } from '../assets/lib.js';

const contributors = [
  { login: 'GolfZGezz', type: 'User', contributions: 25, avatar_url: 'https://a/2?v=4', html_url: 'https://github.com/GolfZGezz' },
  { login: 'dependabot[bot]', type: 'Bot', contributions: 9, avatar_url: 'x', html_url: 'x' },
  { login: 'luraselenehalo', type: 'User', contributions: 75, avatar_url: 'https://a/1?v=4', html_url: 'https://github.com/luraselenehalo' },
];
const profiles = {
  GolfZGezz: { name: 'GolfZzz', bio: "CEO & Founder of ResinCore\r\nZGezz Channel I'm Gamer & Full-Stack Dev" },
  luraselenehalo: { name: 'Lura Dev', bio: 'CTO of ResinCore' },
};

test('people are the human contributors, most commits first', () => {
  assert.deepEqual(people(contributors, profiles).map((p) => p.login), ['luraselenehalo', 'GolfZGezz']);
});

test('a profile gives the name, the first bio line as the role and the rest as about', () => {
  const golf = people(contributors, profiles)[1];
  assert.equal(golf.name, 'GolfZzz');
  assert.equal(golf.role, 'CEO & Founder of ResinCore');
  assert.equal(golf.about, "ZGezz Channel I'm Gamer & Full-Stack Dev");
  assert.equal(golf.commits, 25);
  assert.equal(golf.url, 'https://github.com/GolfZGezz');
  assert.equal(golf.avatar, 'https://a/2?v=4&s=192');
});

test('without a profile the login stands in for the name', () => {
  const p = people(contributors, {})[0];
  assert.equal(p.name, 'luraselenehalo');
  assert.equal(p.role, '');
  assert.equal(p.about, '');
});

test('nothing to show gives an empty list', () => {
  assert.deepEqual(people(null, null), []);
});
