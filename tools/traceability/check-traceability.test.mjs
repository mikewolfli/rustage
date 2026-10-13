// SPDX-License-Identifier: MIT
//
// Negative and positive cases for the traceability checks. The rules worth refusing are the ones
// the acceptance line names: a requirement with no owner, a gate declared but not run, a journey or
// task that does not exist, and a baseline task the roadmap has never heard of.

import assert from 'node:assert/strict';
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { after, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  checkTraceability, checkTraceabilityData, checkTraceabilityDocument, commandRunsInWorkflow,
  describe, parseManifestNamespaces, parseRoadmapIds, SCHEMA, TRACEABILITY_PATH,
} from './check-traceability.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const data = JSON.parse(readFileSync(path.join(root, TRACEABILITY_PATH), 'utf8'));
const copied = [
  TRACEABILITY_PATH,
  'docs/plan/traceability.md',
  'docs/plan/roadmap.md',
  'docs/plan/benchmark.json',
  'docs/plan/journeys.json',
  'resources/host/icons/manifest.json',
  data.sources.workflow,
];
const temporaryDirectories = [];

function fixture(mutateData, mutateDocument, mutateWorkflow) {
  const directory = mkdtempSync(path.join(tmpdir(), 'rustage-traceability-'));
  temporaryDirectories.push(directory);
  for (const file of copied) {
    mkdirSync(path.join(directory, path.dirname(file)), { recursive: true });
    cpSync(path.join(root, file), path.join(directory, file));
  }
  if (mutateData) {
    const target = path.join(directory, TRACEABILITY_PATH);
    const mutated = JSON.parse(readFileSync(target, 'utf8'));
    mutateData(mutated);
    writeFileSync(target, `${JSON.stringify(mutated, null, 2)}\n`);
  }
  if (mutateDocument) {
    const target = path.join(directory, 'docs/plan/traceability.md');
    writeFileSync(target, mutateDocument(readFileSync(target, 'utf8')));
  }
  if (mutateWorkflow) {
    const target = path.join(directory, data.sources.workflow);
    writeFileSync(target, mutateWorkflow(readFileSync(target, 'utf8')));
  }
  return directory;
}

function messages(directory) {
  return checkTraceability(directory).errors.join('\n');
}

function replace(text, from, to) {
  if (!text.includes(from)) throw new Error(`fixture text not found: ${from}`);
  return text.replace(from, to);
}

after(() => {
  for (const directory of temporaryDirectories) rmSync(directory, { recursive: true, force: true });
});

test('the committed traceability data and document pass', () => {
  const result = checkTraceability(root);
  assert.deepEqual(result.errors, []);
  assert.equal(result.stats.requirements, 16);
  assert.equal(result.stats.gates, 21);
  assert.equal(result.stats.ciGates, 16);
  assert.equal(result.stats.manualGates, 5);
  assert.equal(result.stats.baseline, 9);
});

test('describe names the chain and both kinds of gate', () => {
  assert.equal(
    describe({ requirements: 16, gates: 21, ciGates: 16, manualGates: 5, baseline: 9, namespaces: 16 }),
    '16 requirements, 21 gates (16 in CI, 5 manual), baseline of 9 tasks, 16 command namespaces',
  );
});

test('validating data and document separately reports the same shape of errors', () => {
  assert.deepEqual(checkTraceabilityData(data, root), []);
  assert.deepEqual(
    checkTraceabilityDocument(data, readFileSync(path.join(root, 'docs/plan/traceability.md'), 'utf8')),
    [],
  );
});

test('a data file with the wrong schema is refused', () => {
  const directory = fixture((mutated) => {
    mutated.schema = 'rustage.traceability/v2';
  });
  assert.match(messages(directory), new RegExp(`expected ${SCHEMA}`));
});

test('a requirement with no owning task is refused', () => {
  const directory = fixture((mutated) => {
    mutated.requirements[0].tasks = [];
  });
  assert.match(messages(directory), /G1\.tasks must be a non-empty list/);
});

test('a requirement owned by a task the roadmap does not define is refused', () => {
  const directory = fixture((mutated) => {
    mutated.requirements[0].tasks = ['T99.99'];
  });
  assert.match(messages(directory), /G1 is owned by T99\.99, which the roadmap does not define/);
});

test('a requirement claiming a journey the roadmap does not define is refused', () => {
  const directory = fixture((mutated) => {
    mutated.requirements[1].journeys = ['J99'];
  });
  assert.match(messages(directory), /G2 claims journey J99/);
});

test('a requirement with no gate is refused', () => {
  const directory = fixture((mutated) => {
    mutated.requirements[2].gates = [];
  });
  assert.match(messages(directory), /G3\.gates must be a non-empty list/);
});

test('a requirement naming a gate that is not declared is refused', () => {
  const directory = fixture((mutated) => {
    mutated.requirements[3].gates = ['imaginary-gate'];
  });
  assert.match(messages(directory), /G4 names gate imaginary-gate, which is not declared/);
});

test('a requirement naming a measurement scenario that does not exist is refused', () => {
  const directory = fixture((mutated) => {
    mutated.requirements[4].scenarios = ['imaginary-scenario'];
  });
  assert.match(messages(directory), /G5 names scenario imaginary-scenario/);
});

test('a CI gate that is not actually run is refused', () => {
  const directory = fixture((mutated) => {
    mutated.gates.find((gate) => gate.id === 'contracts').command = 'node tools/contracts/check-contracts.mjs --pretend';
  });
  assert.match(messages(directory), /is declared as a CI gate but .* does not appear in/);
});

test('a manual gate without a reason or owner is refused', () => {
  const reason = fixture((mutated) => {
    delete mutated.gates.find((gate) => gate.id === 'rust-test').reason;
  });
  assert.match(messages(reason), /rust-test\.reason must be a non-empty string/);

  const owner = fixture((mutated) => {
    delete mutated.gates.find((gate) => gate.id === 'rust-test').owner;
  });
  assert.match(messages(owner), /rust-test\.owner must be a non-empty string/);
});

test('a gate with a mode outside the vocabulary is refused', () => {
  const directory = fixture((mutated) => {
    mutated.gates[0].mode = 'sometimes';
  });
  assert.match(messages(directory), /rust-test has mode "sometimes"/);
});

test('a duplicated gate is refused', () => {
  const directory = fixture((mutated) => {
    mutated.gates.push({ ...mutated.gates[0] });
  });
  assert.match(messages(directory), /duplicate gate rust-test/);
});

test('a command namespace the manifest does not use is refused', () => {
  const directory = fixture((mutated) => {
    mutated.commands.namespaces.push('teleport');
  });
  assert.match(messages(directory), /command namespaces are .*teleport.* the manifest uses/);
});

test('a baseline task the roadmap does not define is refused', () => {
  const directory = fixture((mutated) => {
    mutated.release.baseline.push('T00.99');
  });
  assert.match(messages(directory), /the baseline names T00\.99, which the roadmap does not define/);
});

test('a scope the roadmap does not state is refused', () => {
  const directory = fixture((mutated) => {
    mutated.release.officialScope = '28 个功能域、300 项任务';
  });
  assert.match(messages(directory), /the roadmap does not state "28 个功能域、300 项任务"/);
});

test('a missing source document is refused', () => {
  const directory = fixture((mutated) => {
    mutated.sources.roadmap = 'docs/plan/nowhere.md';
  });
  assert.match(messages(directory), /sources\.roadmap does not exist/);
});

test('a document edited out of step with the data is refused', () => {
  const directory = fixture(null, (markdown) => replace(
    markdown,
    '| G1 | 所见即所得',
    '| G1-renamed | 所见即所得',
  ));
  assert.match(messages(directory), /the requirement table is/);
});

test('a missing document table is refused', () => {
  const directory = fixture(null, (markdown) => markdown
    .split('\n')
    .filter((line) => !line.startsWith('| 范围项 |'))
    .join('\n'));
  assert.match(messages(directory), /the release table is missing/);
});

test('roadmap parsing finds the task and journey ids, and the manifest its namespaces', () => {
  const roadmap = readFileSync(path.join(root, data.sources.roadmap), 'utf8');
  const { tasks, journeys } = parseRoadmapIds(roadmap);

  assert.ok(tasks.has('T00.05') && tasks.has('T27.08'));
  assert.ok(journeys.has('J01') && journeys.has('J26'));
  assert.ok(!tasks.has('T99.99'));

  const manifest = JSON.parse(readFileSync(path.join(root, 'resources/host/icons/manifest.json'), 'utf8'));
  const { namespaces, groups } = parseManifestNamespaces(manifest);
  assert.equal(groups.length, 71);
  assert.deepEqual(namespaces, data.commands.namespaces);
});

test('a gate command is only considered CI when the workflow really runs it', () => {
  assert.ok(commandRunsInWorkflow('node tools/check-docs.mjs', 'run: node tools/check-docs.mjs'));
  assert.ok(!commandRunsInWorkflow('node tools/check-docs.mjs', 'run: node tools/other.mjs'));
});
