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

function milestone(id, scope, dependencies = '无') {
  return `| ${id} Stage | ${scope} | ${dependencies} | Evidence |`;
}

test('checks declared roadmap counts against tasks, milestones, and journeys', () => {
  const text = [
    '共 **1 个功能域、1 项实施任务、1 个里程碑、1 条用户旅程**。',
    row('T00.01'), milestone('MS00', 'F00'), '| J01 Journey | Steps | Evidence | MS00 |',
  ].join('\n');
  assert.deepEqual(checkRoadmap(text), []);
  assert.ok(checkRoadmap(text.replace('1 项实施任务', '2 项实施任务'))
    .some(error => error.includes('declared total')));
});

test('accepts staged domain and task ranges without rescheduling integration gates', () => {
  assert.deepEqual(checkRoadmap([
    row('T00.01'), row('T00.02', 'T00.01'), row('T01.01', 'T00.02'),
    row('T02.01', 'T01.01'),
    milestone('MS00', 'T00.01–T00.02'),
    milestone('MS01', 'F01/F02', 'MS00'),
    milestone('MS02', 'F00–F02', 'MS00–MS01'),
  ].join('\n')), []);
});

test('rejects task prerequisites scheduled in later milestones', () => {
  assert.ok(checkRoadmap([
    row('T00.01', 'T01.01'), row('T01.01'),
    milestone('MS00', 'F00'), milestone('MS01', 'F01'),
  ].join('\n')).some(error => error.includes('requires later')));
});

test('requires an entry dependency rather than inferring one from milestone order', () => {
  const tasks = [row('T00.01'), row('T01.01', 'T00.01'), milestone('MS00', 'F00')];
  assert.ok(checkRoadmap([...tasks, milestone('MS01', 'F01')].join('\n'))
    .some(error => error.includes('without an entry dependency')));
  assert.deepEqual(checkRoadmap([...tasks, milestone('MS01', 'F01', 'MS00')].join('\n')), []);
});

test('accepts explicitly deferred scope without making it a release milestone', () => {
  const text = [
    '共 **2 个功能域、2 项实施任务、1 个里程碑、1 条用户旅程**。',
    '正式版范围为 1 个功能域、1 项任务、0 条旅程。',
    row('T00.01'), row('T28.01', 'T00.01'),
    milestone('MS00', 'F00'), milestone('FUTURE', 'F28'),
    '| J01 Journey | Steps | Evidence | FUTURE |',
  ].join('\n');
  assert.deepEqual(checkRoadmap(text), []);
  assert.ok(checkRoadmap(text.replace('1 项任务', '2 项任务'))
    .some(error => error.includes('declared release total')));
});

test('rejects release tasks that depend on future extension tasks', () => {
  assert.ok(checkRoadmap([
    row('T00.01', 'T28.01'), row('T28.01'),
    milestone('MS00', 'F00'), milestone('FUTURE', 'F28'),
  ].join('\n')).some(error => error.includes('requires future task')));
});

test('rejects a task assigned to both release and future scope', () => {
  assert.ok(checkRoadmap([
    row('T00.01'), milestone('MS00', 'F00'), milestone('FUTURE', 'F00'),
  ].join('\n')).some(error => error.includes('both release scope and future scope')));
});

test('rejects missing assignments and invalid milestone scopes', () => {
  assert.ok(checkRoadmap([
    row('T00.01'), row('T01.01'), milestone('MS00', 'F00'),
  ].join('\n')).some(error => error.includes('no milestone')));
  for (const scope of [
    'F99', 'F00–F99', 'F00.01', 'T00.02–T00.01', 'T00.01–T00.99', 'T00.01–F01', 'T00',
  ]) {
    assert.ok(checkRoadmap([row('T00.01'), milestone('MS00', scope)].join('\n'))
      .some(error => error.includes('scope')));
  }
});
