// SPDX-License-Identifier: MIT
//
// Negative and positive cases for the benchmark-environment checks. The rules that matter are the
// ones that keep a threshold honest (a softened threshold is refused), keep a claim checkable (a
// measured scenario without an artifact is refused) and keep the data sets connected (a scenario
// whose fixture does not advertise it is refused).

import assert from 'node:assert/strict';
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { after, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  checkBenchmark, checkBenchmarkData, checkBenchmarkDocument, describe, parseRoadmapJourneyIds,
  parseRoadmapScenarios, SCHEMA, BENCHMARK_PATH,
} from './check-benchmark.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const benchmark = JSON.parse(readFileSync(path.join(root, BENCHMARK_PATH), 'utf8'));
const roadmapPath = benchmark.roadmap.file;
const fixturesPath = benchmark.fixtures;
const temporaryDirectories = [];

function fixture(mutateData, mutateDocument, mutateFixtures) {
  const directory = mkdtempSync(path.join(tmpdir(), 'rustage-benchmark-'));
  temporaryDirectories.push(directory);
  for (const file of [BENCHMARK_PATH, 'docs/plan/benchmark.md', roadmapPath, fixturesPath]) {
    mkdirSync(path.join(directory, path.dirname(file)), { recursive: true });
    cpSync(path.join(root, file), path.join(directory, file));
  }
  for (const [file, mutate] of [[BENCHMARK_PATH, mutateData], [fixturesPath, mutateFixtures]]) {
    if (!mutate) continue;
    const target = path.join(directory, file);
    const data = JSON.parse(readFileSync(target, 'utf8'));
    mutate(data);
    writeFileSync(target, `${JSON.stringify(data, null, 2)}\n`);
  }
  if (mutateDocument) {
    const target = path.join(directory, 'docs/plan/benchmark.md');
    writeFileSync(target, mutateDocument(readFileSync(target, 'utf8')));
  }
  return directory;
}

function messages(directory) {
  return checkBenchmark(directory).errors.join('\n');
}

function replace(text, from, to) {
  if (!text.includes(from)) throw new Error(`fixture text not found: ${from}`);
  return text.replace(from, to);
}

after(() => {
  for (const directory of temporaryDirectories) rmSync(directory, { recursive: true, force: true });
});

test('the committed benchmark document, roadmap and fixtures agree', () => {
  const result = checkBenchmark(root);
  assert.deepEqual(result.errors, []);
  assert.equal(result.stats.scenarios, 18);
  assert.equal(result.stats.measured, 0);
  assert.equal(result.stats.tally.unmeasured, 16);
  assert.equal(result.stats.tally.future, 2);
});

test('describe names the statuses and the recorded facts', () => {
  assert.equal(
    describe({ scenarios: 18, measured: 0, tally: { unmeasured: 16, future: 2 }, hardware: 8 }),
    '18 scenarios (16 unmeasured, 2 future), 8 hardware facts recorded',
  );
});

test('validating data and document separately reports the same shape of errors', () => {
  assert.deepEqual(checkBenchmarkData(benchmark, root), []);
  assert.deepEqual(checkBenchmarkDocument(benchmark, readFileSync(path.join(root, 'docs/plan/benchmark.md'), 'utf8')), []);
});

test('a data file with the wrong schema is refused', () => {
  const directory = fixture((data) => {
    data.schema = 'rustage.benchmark/v2';
  });
  assert.match(messages(directory), new RegExp(`expected ${SCHEMA}`));
});

test('a softened threshold is refused and told to change the roadmap first', () => {
  const directory = fixture((data) => {
    data.scenarios[0].threshold = 'p95 ≤ 500 ms';
  });
  assert.match(messages(directory), /change the roadmap first/);
});

test('a renamed or reordered scenario is refused', () => {
  const directory = fixture((data) => {
    data.scenarios[1].scenario = '差不多的场景';
  });
  assert.match(messages(directory), /describes "差不多的场景", roadmap row 2/);
});

test('a scenario with no entry in the walkthrough is refused', () => {
  const directory = fixture((data) => {
    data.scenarios = data.scenarios.filter((scenario) => scenario.id !== 'ai-session-budget');
  });
  assert.match(messages(directory), /roadmap scenario "双 AI 会话与共享材料" has no entry/);
  assert.match(messages(directory), /17 scenarios recorded, the roadmap defines 18/);
});

test('an unknown status is refused', () => {
  const directory = fixture((data) => {
    data.scenarios[0].status = 'probably-fine';
  });
  assert.match(messages(directory), /status "probably-fine" outside the vocabulary/);
});

test('claiming a measurement without the artifact is refused', () => {
  const directory = fixture((data) => {
    data.scenarios[0].status = 'measured';
  });
  assert.match(messages(directory), /design-update-100\.artifact \(a measured scenario must be checkable\)/);
});

test('carrying measurement fields while unmeasured is refused', () => {
  const directory = fixture((data) => {
    data.scenarios[0].artifact = 'target/bench/design-update-100.json';
  });
  assert.match(messages(directory), /is unmeasured but carries artifact/);
});

test('a scenario using a fixture that does not exist is refused', () => {
  const directory = fixture((data) => {
    data.scenarios[0].fixtures = ['imaginary'];
  });
  assert.match(messages(directory), /uses fixture imaginary, which the spec does not define/);
});

test('a scenario whose fixture does not advertise it is refused', () => {
  const directory = fixture(null, null, (spec) => {
    const normal = spec.fixtures.find((entry) => entry.id === 'normal');
    normal.measurement = normal.measurement.filter((id) => id !== 'churn-100');
  });
  assert.match(messages(directory), /uses fixture normal, but that fixture does not advertise the scenario/);
});

test('a fixture claiming a journey the roadmap does not define is refused', () => {
  const directory = fixture(null, null, (spec) => {
    spec.fixtures[0].journeys = ['J99'];
  });
  assert.match(messages(directory), /claims journey J99, which the roadmap does not define/);
});

test('a protocol without a start point is refused', () => {
  const directory = fixture((data) => {
    delete data.protocol.startPoint;
  });
  assert.match(messages(directory), /protocol\.startPoint must be a non-empty string/);
});

test('a protocol with no repetitions is refused', () => {
  const directory = fixture((data) => {
    data.protocol.repetitions = 0;
  });
  assert.match(messages(directory), /protocol\.repetitions must be a positive integer/);
});

test('a hardware record without commands is refused', () => {
  const directory = fixture((data) => {
    data.hardware.commands = [];
  });
  assert.match(messages(directory), /hardware\.commands must list how the values were obtained/);
});

test('a missing data set reference is refused', () => {
  const directory = fixture((data) => {
    data.fixtures = 'fixtures/nowhere.json';
  });
  assert.match(messages(directory), /missing fixtures\/nowhere\.json/);
});

test('a document edited out of step with the data is refused', () => {
  const directory = fixture(null, (markdown) => replace(
    markdown,
    '| design-update-100 |',
    '| design-update-100-renamed |',
  ));
  assert.match(messages(directory), /the scenario table is/);
});

test('a missing document table is refused', () => {
  const directory = fixture(null, (markdown) => markdown
    .split('\n')
    .filter((line) => !line.startsWith('| 状态 |') && !/^\| (measured|unmeasured|blocked|future) \|/.test(line))
    .join('\n'));
  assert.match(messages(directory), /the status table is missing/);
});

test('roadmap parsing keeps the scenario order and the journey ids', () => {
  const roadmap = readFileSync(path.join(root, roadmapPath), 'utf8');
  const scenarios = parseRoadmapScenarios(roadmap, benchmark.roadmap.section);

  assert.equal(scenarios.length, 18);
  assert.equal(scenarios[0][0], '100 节点单属性/CSS 更新');
  assert.match(scenarios[0][1], /^提交到可见帧 p95 ≤ 100 ms/);
  const journeys = parseRoadmapJourneyIds(roadmap);
  assert.ok(journeys.has('J01') && journeys.has('J26'));
  assert.ok(!journeys.has('J99'));
});
