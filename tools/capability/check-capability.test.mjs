// SPDX-License-Identifier: MIT
//
// Negative and positive cases for the capability-reference checks. The rules that matter are the
// acceptance line: a new requirement without a task, journey or gate is refused, and a dependency
// that lands after the phase signing the item off is refused.

import assert from 'node:assert/strict';
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { after, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  checkCapability, checkCapabilityData, checkCapabilityDocument, describe, milestoneOrder,
  parseMilestones, parseTaskPhases, SCHEMA, CAPABILITY_PATH,
} from './check-capability.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const data = JSON.parse(readFileSync(path.join(root, CAPABILITY_PATH), 'utf8'));
const temporaryDirectories = [];

function fixture(mutateData, mutateDocument) {
  const directory = mkdtempSync(path.join(tmpdir(), 'rustage-capability-'));
  temporaryDirectories.push(directory);
  for (const file of [CAPABILITY_PATH, 'docs/plan/capability-reference.md', 'docs/plan/roadmap.md', 'docs/plan/traceability.json']) {
    mkdirSync(path.join(directory, path.dirname(file)), { recursive: true });
    cpSync(path.join(root, file), path.join(directory, file));
  }
  if (mutateData) {
    const target = path.join(directory, CAPABILITY_PATH);
    const mutated = JSON.parse(readFileSync(target, 'utf8'));
    mutateData(mutated);
    writeFileSync(target, `${JSON.stringify(mutated, null, 2)}\n`);
  }
  if (mutateDocument) {
    const target = path.join(directory, 'docs/plan/capability-reference.md');
    writeFileSync(target, mutateDocument(readFileSync(target, 'utf8')));
  }
  return directory;
}

function messages(directory) {
  return checkCapability(directory).errors.join('\n');
}

function replace(text, from, to) {
  if (!text.includes(from)) throw new Error(`fixture text not found: ${from}`);
  return text.replace(from, to);
}

after(() => {
  for (const directory of temporaryDirectories) rmSync(directory, { recursive: true, force: true });
});

test('the committed capability reference passes', () => {
  const result = checkCapability(root);
  assert.deepEqual(result.errors, []);
  assert.equal(result.stats.items, 12);
  assert.equal(result.stats.protocols, 7);
  assert.deepEqual(result.stats.tally, { adopt: 9, defer: 2, 'not-do': 1 });
});

test('describe names the decisions and the protocol count', () => {
  assert.equal(
    describe({ items: 12, protocols: 7, tally: { adopt: 9, defer: 2, 'not-do': 1 } }),
    '12 supplementary requirements (9 adopt, 2 defer, 1 not-do), 7 protocol versions',
  );
});

test('validating data and document separately reports the same shape of errors', () => {
  assert.deepEqual(checkCapabilityData(data, root), []);
  assert.deepEqual(
    checkCapabilityDocument(data, readFileSync(path.join(root, 'docs/plan/capability-reference.md'), 'utf8')),
    [],
  );
});

test('a data file with the wrong schema is refused', () => {
  const directory = fixture((mutated) => {
    mutated.schema = 'rustage.capability/v2';
  });
  assert.match(messages(directory), new RegExp(`expected ${SCHEMA}`));
});

test('a requirement without a task, journey or gate is refused', () => {
  const tasks = fixture((mutated) => {
    mutated.items[0].tasks = [];
  });
  assert.match(messages(tasks), /C-MODEL-SINGLE-SOURCE\.tasks must be a non-empty list/);

  const journeys = fixture((mutated) => {
    mutated.items[0].journeys = [];
  });
  assert.match(messages(journeys), /C-MODEL-SINGLE-SOURCE\.journeys must be a non-empty list/);

  const gates = fixture((mutated) => {
    mutated.items[0].gates = [];
  });
  assert.match(messages(gates), /C-MODEL-SINGLE-SOURCE\.gates must be a non-empty list/);
});

test('an unknown decision is refused', () => {
  const directory = fixture((mutated) => {
    mutated.items[0].decision = 'maybe';
  });
  assert.match(messages(directory), /has decision "maybe"/);
});

test('a phase that is not a milestone is refused', () => {
  const directory = fixture((mutated) => {
    mutated.items[0].phase = 'MS99';
  });
  assert.match(messages(directory), /is signed off in "MS99", which is not a roadmap milestone/);
});

test('a dependency landing after the sign-off phase is refused', () => {
  const directory = fixture((mutated) => {
    mutated.items.find((item) => item.id === 'C-LARGE-FILE').dependsOn = ['T16.02'];
  });
  assert.match(messages(directory), /is signed off in MS04 but depends on T16\.02 \(MS07\)/);
});

test('a missing dependency list is refused', () => {
  const directory = fixture((mutated) => {
    mutated.items[0].dependsOn = [];
  });
  assert.match(messages(directory), /C-MODEL-SINGLE-SOURCE\.dependsOn must name at least one task/);
});

test('a task, journey or gate that does not exist is refused', () => {
  const task = fixture((mutated) => {
    mutated.items[0].tasks = ['T99.99'];
  });
  assert.match(messages(task), /names task T99\.99/);

  const journey = fixture((mutated) => {
    mutated.items[0].journeys = ['J99'];
  });
  assert.match(messages(journey), /names journey J99/);

  const gate = fixture((mutated) => {
    mutated.items[0].gates = ['imaginary-gate'];
  });
  assert.match(messages(gate), /names gate imaginary-gate/);
});

test('a duplicated item or protocol is refused', () => {
  const item = fixture((mutated) => {
    mutated.items.push({ ...mutated.items[0] });
  });
  assert.match(messages(item), /duplicate item C-MODEL-SINGLE-SOURCE/);

  const protocol = fixture((mutated) => {
    mutated.protocols.push({ ...mutated.protocols[0] });
  });
  assert.match(messages(protocol), /duplicate protocol P-FORMAT/);
});

test('a protocol without an owner or meaning is refused', () => {
  const owner = fixture((mutated) => {
    mutated.protocols[0].owner = 'T99.99';
  });
  assert.match(messages(owner), /P-FORMAT is owned by T99\.99/);

  const meaning = fixture((mutated) => {
    mutated.protocols[0].meaning = '';
  });
  assert.match(messages(meaning), /protocols\[0\]\.meaning must be a non-empty string/);
});

test('a document edited out of step with the data is refused', () => {
  const directory = fixture(null, (markdown) => replace(
    markdown,
    '| C-HOT-UPDATE |',
    '| C-HOT-UPDATE-RENAMED |',
  ));
  assert.match(messages(directory), /the item table is/);
});

test('a missing document table is refused', () => {
  const directory = fixture(null, (markdown) => markdown
    .split('\n')
    .filter((line) => !line.startsWith('| 编号 | 协议 |'))
    .filter((line) => !/^\| P-[A-Z]+ \|/.test(line))
    .join('\n'));
  assert.match(messages(directory), /the protocol table is missing/);
});

test('the milestone table is read in order and maps domains and explicit ranges to tasks', () => {
  const roadmap = readFileSync(path.join(root, 'docs/plan/roadmap.md'), 'utf8');
  const milestones = parseMilestones(roadmap, data.roadmap.section);

  assert.ok(milestones.length >= 20);
  assert.equal(milestones[0].id, 'MS00');
  assert.equal(milestones.at(-1).id, 'FUTURE');

  const order = milestoneOrder(milestones);
  assert.ok(order.get('MS00') < order.get('MS19'));
  assert.ok(order.get('MS19') < order.get('FUTURE'));

  const phases = parseTaskPhases(milestones);
  assert.equal(phases.get('T00.05'), 'MS00');
  assert.equal(phases.get('T15.01'), 'MS07', 'a domain-delivered task follows its milestone');
  assert.equal(phases.get('T13.06'), 'MS06', 'an explicit range wins over the domain default');
});
