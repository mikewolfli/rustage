import assert from 'node:assert/strict';
import path from 'node:path';
import test from 'node:test';
import { checkLinks, checkRoadmap, checkText } from './check-docs.mjs';

test('accepts clean text', () => {
  assert.deepEqual(checkText('# Title\n\nText.\n', 'README.md'), []);
});

test('reports whitespace, conflict markers, missing newline, and CRLF', () => {
  const errors = checkText('text \n<<<<<<< branch\n=======\n>>>>>>> main\r\nend', 'a.md');
  assert.ok(errors.some(error => error.includes('trailing whitespace')));
  assert.equal(errors.filter(error => error.includes('conflict marker')).length, 3);
  assert.ok(errors.some(error => error.includes('final newline')));
  assert.ok(errors.some(error => error.includes('LF line endings')));
});

test('resolves repository-relative files and directories', () => {
  const root = path.resolve('test-repository');
  const targets = new Set([path.join(root, 'README.md'), path.join(root, 'docs')]);
  assert.deepEqual(checkLinks(
    '[Home](../README.md) [Docs](./) [Web](https://example.com) [Heading](#title)',
    'docs/index.md', root, target => targets.has(target),
  ), []);
});

test('supports image, encoded, and angle-bracket paths', () => {
  const root = path.resolve('test-repository');
  const targets = new Set([path.join(root, 'a b.md'), path.join(root, 'image.png')]);
  assert.deepEqual(checkLinks(
    '[File](a%20b.md#title) [File](<a b.md>) ![Image](image.png)',
    'README.md', root, target => targets.has(target),
  ), []);
});

test('rejects missing files, escaped paths, absolute paths, and invalid encoding', () => {
  const errors = checkLinks(
    '[Missing](missing.md) [Outside](../outside.md) [Absolute](/tmp/a.md) [Bad](%ZZ.md)',
    'README.md', path.resolve('test-repository'), () => false,
  );
  assert.equal(errors.length, 4);
});

test('ignores example links in fenced code blocks', () => {
  assert.deepEqual(checkLinks(
    '```md\n[Example](missing.md)\n```\n',
    'README.md', path.resolve('test-repository'), () => false,
  ), []);
});

function row(id, dependencies = '无') {
  return `| ${id} | Task | ${dependencies} | P0/M | Evidence |`;
}

test('accepts a valid task dependency graph', () => {
  assert.deepEqual(checkRoadmap([row('T00.01'), row('T00.02', 'T00.01')].join('\n')), []);
});

test('rejects empty or malformed task tables', () => {
  assert.ok(checkRoadmap('# Roadmap').some(error => error.includes('no implementation')));
  assert.ok(checkRoadmap('| T00.01 | Task | 无 | P0/M |')
    .some(error => error.includes('malformed')));
  assert.ok(checkRoadmap('| T00.01 |')
    .some(error => error.includes('malformed')));
});

test('rejects duplicate and missing tasks', () => {
  const errors = checkRoadmap([
    row('T00.01'), row('T00.01'), row('T00.02', 'T99.01'),
  ].join('\n'));
  assert.ok(errors.some(error => error.includes('duplicate')));
  assert.ok(errors.some(error => error.includes('missing T99.01')));
});

test('rejects self dependencies and multi-task cycles', () => {
  assert.ok(checkRoadmap(row('T00.01', 'T00.01')).some(error => error.includes('cycle')));
  assert.ok(checkRoadmap([
    row('T00.01', 'T00.02'), row('T00.02', 'T00.01'),
  ].join('\n')).some(error => error.includes('cycle')));
});
