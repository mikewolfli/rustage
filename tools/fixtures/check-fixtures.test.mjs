// SPDX-License-Identifier: MIT
//
// Cases for the fixture spec and the generator. The two things worth refusing are an unfixed size
// (a benchmark without a data definition measures nothing) and a fixture nobody asked for; the
// thing worth proving is reproducibility, because a generator that drifts makes every later
// measurement incomparable.

import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { after, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { checkSpec, readSpec, SCHEMA } from './spec.mjs';
import { digestFixture, fixtureFiles, fixtureIds, writeFixture } from './generate-fixtures.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const spec = readSpec(path.join(root, 'fixtures/spec.json'));
const temporaryDirectories = [];

function clone() {
  return JSON.parse(JSON.stringify(spec));
}

after(() => {
  for (const directory of temporaryDirectories) rmSync(directory, { recursive: true, force: true });
});

test('the committed spec passes', () => {
  assert.deepEqual(checkSpec(spec, root), []);
  assert.equal(fixtureIds(spec).length, 5);
});

test('the spec must declare its schema', () => {
  const mutated = clone();
  mutated.schema = 'rustage.fixtures/v2';
  assert.match(checkSpec(mutated).join('\n'), new RegExp(`expected ${SCHEMA}`));
});

test('a size that is not a fixed positive integer is refused', () => {
  const mutated = clone();
  mutated.sizes.linesMedium = 0;
  assert.match(checkSpec(mutated).join('\n'), /sizes\.linesMedium is not a fixed positive integer: 0/);

  const fractional = clone();
  fractional.sizes.pagesMultiPage = 12.5;
  assert.match(checkSpec(fractional).join('\n'), /sizes\.pagesMultiPage is not a fixed positive integer/);
});

test('a duplicated fixture id is refused', () => {
  const mutated = clone();
  mutated.fixtures[1].id = 'normal';
  assert.match(checkSpec(mutated).join('\n'), /duplicate id normal/);
});

test('a fixture that serves no journey is refused', () => {
  const mutated = clone();
  mutated.fixtures[0].journeys = [];
  assert.match(checkSpec(mutated).join('\n'), /normal serves no journey/);
});

test('a fixture that states nothing it covers or expects is refused', () => {
  const empty = clone();
  empty.fixtures[0].covers = [];
  assert.match(checkSpec(empty).join('\n'), /normal states nothing it covers/);

  const expectation = clone();
  delete expectation.fixtures[0].expect.notes;
  assert.match(checkSpec(expectation).join('\n'), /normal\.expect\.notes must be a non-empty string/);
});

test('an unknown cargo check expectation is refused', () => {
  const mutated = clone();
  mutated.fixtures[3].expect.cargoCheck = 'probably';
  assert.match(checkSpec(mutated).join('\n'), /expect\.cargoCheck is "probably"/);
});

test('a spec without a determinism statement is refused', () => {
  const mutated = clone();
  delete mutated.determinism;
  assert.match(checkSpec(mutated).join('\n'), /`determinism` must say how reproducibility is shown/);
});

test('generating a fixture twice produces identical bytes', () => {
  for (const id of fixtureIds(spec)) {
    assert.equal(digestFixture(spec, id), digestFixture(spec, id), `${id} is not reproducible`);
  }
});

test('different fixtures are different data', () => {
  const digests = fixtureIds(spec).map((id) => digestFixture(spec, id));
  assert.equal(new Set(digests).size, digests.length);
});

test('an unknown fixture id has no files instead of an empty fixture', () => {
  assert.equal(fixtureFiles(spec, 'imaginary'), null);
});

test('the sizes in the spec are the sizes in the files', () => {
  const files = fixtureFiles(spec, 'large-file');
  const twoMillion = files.find((file) => file.path === 'large/2m.rs').content;
  const lines = twoMillion.split('\n').length - 1;
  assert.ok(lines >= spec.sizes.linesBoundaryHigh, `${lines} lines is below the fixed boundary`);

  const longest = files.find((file) => file.path === 'large/longest-line.rs').content;
  const longestLine = Math.max(...longest.split('\n').map((line) => line.length));
  assert.equal(longestLine, spec.sizes.longestLineBytes - 1, 'the longest line stops one byte short');

  const unicode = files.find((file) => file.path === 'large/unicode.rs').content;
  assert.ok(unicode.includes('中文标签'), 'the unicode fixture must carry non-ASCII text');
});

test('writing a fixture creates the files it declares', () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'rustage-fixtures-'));
  temporaryDirectories.push(directory);

  const written = writeFixture(spec, 'normal', directory);

  assert.equal(written, fixtureFiles(spec, 'normal').length);
  for (const file of fixtureFiles(spec, 'normal')) {
    assert.ok(existsSync(path.join(directory, 'normal', file.path)), `${file.path} was not written`);
  }
  assert.equal(
    readFileSync(path.join(directory, 'normal', 'designer-project.json'), 'utf8'),
    fixtureFiles(spec, 'normal').find((file) => file.path === 'designer-project.json').content,
  );
});

test('the failing fixture really is broken JSON and broken Cargo', () => {
  const files = fixtureFiles(spec, 'failing');
  const document = files.find((file) => file.path === 'ui/broken.json').content;
  const manifest = files.find((file) => file.path === 'Cargo.toml').content;

  assert.throws(() => JSON.parse(document), 'the broken document must not parse');
  assert.match(manifest, /this-line-is-not-a-dependency|\[features$/m);
});
