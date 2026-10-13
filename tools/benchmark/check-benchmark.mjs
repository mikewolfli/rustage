#!/usr/bin/env node
// SPDX-License-Identifier: MIT
//
// Benchmark-environment checks: `docs/plan/benchmark.json`, the readable copy in
// `docs/plan/benchmark.md`, roadmap section 35.2 (the thresholds), and `fixtures/spec.json` (the
// data sets).
//
// Scope, stated honestly. This gate checks that the measurement contract is complete and does not
// drift from the roadmap:
//
//   1. Every scenario row in roadmap 35.2 has exactly one entry here - same scenario text, same
//      threshold text, same acceptance phase, in the same order. A threshold may not be softened
//      here without changing the roadmap first; the roadmap says so in as many words.
//   2. Every scenario names fixtures that exist, and every fixture's own `measurement` list names
//      scenarios that exist. Traceability both ways, so a new scenario cannot float free of data.
//   3. The hardware record and the measurement protocol are filled in and reproducible: recorded
//      commands, warm-up, repetitions, statistic, start and end points.
//   4. A scenario may only claim `measured` with the artifact, revision and date that make the
//      claim checkable. Everything else is unmeasured, blocked or future - the words the roadmap
//      uses for "we do not know yet".
//   5. The document's tables agree with the data file.
//
// What it cannot check: whether the thresholds are achievable, and whether a measurement is
// honest. This gate makes the claim sites checkable; the measurement itself has to exist.

import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const BENCHMARK_PATH = 'docs/plan/benchmark.json';
export const DOCUMENT_PATH = 'docs/plan/benchmark.md';

export const SCHEMA = 'rustage.benchmark/v1';

/** The order these appear in the document's protocol table is the order they are written in. */
export const PROTOCOL_FIELDS = [
  'state', 'warmup', 'repetitions', 'statistic', 'startPoint', 'endPoint', 'excluded', 'noise', 'reporting',
];

/** The same normalisation the table parser applies to a cell, so both sides compare alike. */
function normaliseCell(cell) {
  return String(cell).trim().replace(/^`|`$/g, '');
}

function splitRow(line) {
  const trimmed = line.trim();
  if (!trimmed.startsWith('|') || !trimmed.endsWith('|') || trimmed.length < 2) return null;
  return trimmed
    .slice(1, -1)
    .split('|')
    .map((cell) => cell.trim().replace(/^`|`$/g, ''));
}

/** Collects every Markdown table of `markdown`, keyed by its header cells. */
export function parseTables(markdown) {
  const tables = new Map();
  let current = [];
  const flush = () => {
    if (current.length >= 2) {
      const [header, , ...body] = current;
      tables.set(header.join('\u0000'), body);
    }
    current = [];
  };
  for (const line of markdown.split('\n')) {
    const cells = splitRow(line);
    if (cells) current.push(cells);
    else flush();
  }
  flush();
  return tables;
}

/** The rows of one table inside a section. */
function sectionTables(markdown, heading) {
  const lines = markdown.split('\n');
  const wanted = heading.trim();
  const start = lines.findIndex((line) => {
    const text = line.trim();
    return text === wanted || text.startsWith(`${wanted} `);
  });
  if (start === -1) return null;
  const level = wanted.match(/^#+/)[0].length;
  let end = lines.length;
  for (let index = start + 1; index < lines.length; index += 1) {
    const match = lines[index].match(/^(#+)\s/);
    if (match && match[1].length <= level) {
      end = index;
      break;
    }
  }
  return parseTables(lines.slice(start, end).join('\n'));
}

/** Roadmap 35.2's scenario rows: scenario text, threshold text, acceptance phase. */
export function parseRoadmapScenarios(markdown, heading) {
  const tables = sectionTables(markdown, heading);
  if (!tables) return null;
  for (const [, rows] of tables) {
    const scenarios = rows.filter((row) => row.length === 3 && row[0] !== '场景');
    if (scenarios.length > 0) return scenarios;
  }
  return null;
}

/** The journey ids roadmap 34 defines, so a fixture cannot claim a journey that does not exist. */
export function parseRoadmapJourneyIds(markdown) {
  const ids = new Set();
  for (const match of markdown.matchAll(/^\|\s*(J\d+)\s/gm)) ids.add(match[1]);
  return ids;
}

function requireText(errors, where, value) {
  if (typeof value !== 'string' || value.trim() === '') {
    errors.push(`benchmark: ${where} must be a non-empty string`);
  }
}

/** Validates the benchmark data against the roadmap and the fixture spec. */
export function checkBenchmarkData(benchmark, root) {
  const errors = [];
  if (!benchmark || typeof benchmark !== 'object') return ['benchmark: the data file is not an object'];
  if (benchmark.schema !== SCHEMA) {
    errors.push(`benchmark: schema is ${JSON.stringify(benchmark.schema)}, expected ${SCHEMA}`);
  }

  const roadmapFile = benchmark.roadmap?.file;
  if (typeof roadmapFile !== 'string' || !existsSync(path.join(root, roadmapFile))) {
    return [...errors, `benchmark: missing ${roadmapFile ?? 'roadmap'}`];
  }
  const expected = parseRoadmapScenarios(readFileSync(path.join(root, roadmapFile), 'utf8'), benchmark.roadmap.section);
  if (!expected) {
    return [...errors, `benchmark: the roadmap has no ${benchmark.roadmap.section} scenario table`];
  }

  const fixturesFile = benchmark.fixtures;
  if (typeof fixturesFile !== 'string' || !existsSync(path.join(root, fixturesFile))) {
    return [...errors, `benchmark: missing ${fixturesFile ?? 'fixtures/spec.json'}`];
  }
  const fixtureSpec = JSON.parse(readFileSync(path.join(root, fixturesFile), 'utf8'));
  const fixtureIds = new Set((fixtureSpec.fixtures ?? []).map((fixture) => fixture.id));
  const advertised = new Map((fixtureSpec.fixtures ?? []).map(
    (fixture) => [fixture.id, new Set(fixture.measurement ?? [])],
  ));
  const journeyIds = parseRoadmapJourneyIds(readFileSync(path.join(root, roadmapFile), 'utf8'));
  for (const fixture of fixtureSpec.fixtures ?? []) {
    for (const id of fixture.journeys ?? []) {
      if (!journeyIds.has(id)) {
        errors.push(`benchmark: fixture ${fixture.id} claims journey ${id}, which the roadmap does not define`);
      }
    }
  }

  const vocabulary = benchmark.statusVocabulary;
  if (!vocabulary || typeof vocabulary !== 'object' || Object.keys(vocabulary).length === 0) {
    errors.push('benchmark: `statusVocabulary` must define the statuses');
  } else {
    for (const [status, meaning] of Object.entries(vocabulary)) requireText(errors, `statusVocabulary.${status}`, meaning);
  }

  const hardware = benchmark.hardware;
  if (!hardware || typeof hardware !== 'object') {
    errors.push('benchmark: `hardware` must record the baseline');
  } else {
    requireText(errors, 'hardware.capturedOn', hardware.capturedOn);
    requireText(errors, 'hardware.note', hardware.note);
    if (!Array.isArray(hardware.commands) || hardware.commands.length === 0) {
      errors.push('benchmark: hardware.commands must list how the values were obtained');
    }
    if (!hardware.record || typeof hardware.record !== 'object' || Object.keys(hardware.record).length === 0) {
      errors.push('benchmark: hardware.record must hold the captured values');
    }
  }

  const protocol = benchmark.protocol;
  if (!protocol || typeof protocol !== 'object') {
    errors.push('benchmark: `protocol` must define how a measurement is taken');
  } else {
    for (const field of PROTOCOL_FIELDS) {
      if (field === 'excluded') {
        if (!Array.isArray(protocol.excluded) || protocol.excluded.length === 0) {
          errors.push('benchmark: protocol.excluded must list what a measurement does not include');
        }
        continue;
      }
      if (['warmup', 'repetitions'].includes(field)) {
        if (!Number.isInteger(protocol[field]) || protocol[field] <= 0) {
          errors.push(`benchmark: protocol.${field} must be a positive integer`);
        }
        continue;
      }
      requireText(errors, `protocol.${field}`, protocol[field]);
    }
  }

  const scenarios = benchmark.scenarios;
  if (!Array.isArray(scenarios) || scenarios.length === 0) {
    errors.push('benchmark: `scenarios` must be a non-empty list');
    return errors;
  }

  for (const [index, row] of expected.entries()) {
    const scenario = scenarios[index];
    if (!scenario) {
      errors.push(`benchmark: roadmap scenario ${JSON.stringify(row[0])} has no entry`);
      continue;
    }
    if (scenario.scenario !== row[0]) {
      errors.push(`benchmark: entry ${scenario.id} describes ${JSON.stringify(scenario.scenario)}, roadmap row ${index + 1} is ${JSON.stringify(row[0])}`);
    }
    if (scenario.threshold !== row[1]) {
      errors.push(
        `benchmark: ${scenario.id} threshold is ${JSON.stringify(scenario.threshold)}, `
        + `roadmap says ${JSON.stringify(row[1])} - change the roadmap first, not this file`,
      );
    }
    if (scenario.phase !== row[2]) {
      errors.push(`benchmark: ${scenario.id} phase is ${JSON.stringify(scenario.phase)}, roadmap says ${JSON.stringify(row[2])}`);
    }
  }
  if (scenarios.length !== expected.length) {
    errors.push(`benchmark: ${scenarios.length} scenarios recorded, the roadmap defines ${expected.length}`);
  }

  const seen = new Set();
  for (const [index, scenario] of scenarios.entries()) {
    const where = `scenarios[${index}]`;
    for (const field of ['id', 'metric', 'statistic', 'status', 'owner']) {
      requireText(errors, `${where}.${field}`, scenario?.[field]);
    }
    if (typeof scenario?.id === 'string') {
      if (seen.has(scenario.id)) errors.push(`benchmark: duplicate scenario ${scenario.id}`);
      seen.add(scenario.id);
    }
    if (!Object.hasOwn(vocabulary ?? {}, scenario?.status)) {
      errors.push(`benchmark: ${scenario?.id} has status ${JSON.stringify(scenario?.status)} outside the vocabulary`);
    }
    if (scenario?.status === 'measured') {
      for (const field of ['artifact', 'revision', 'measuredOn']) {
        requireText(errors, `${scenario.id}.${field} (a measured scenario must be checkable)`, scenario[field]);
      }
    } else {
      for (const field of ['artifact', 'revision', 'measuredOn']) {
        if (scenario?.[field] !== undefined) {
          errors.push(`benchmark: ${scenario.id} is ${scenario.status} but carries ${field}`);
        }
      }
    }
    if (!Array.isArray(scenario?.fixtures) || scenario.fixtures.length === 0) {
      errors.push(`benchmark: ${scenario?.id} names no data set`);
      continue;
    }
    for (const id of scenario.fixtures) {
      if (!fixtureIds.has(id)) {
        errors.push(`benchmark: ${scenario.id} uses fixture ${id}, which the spec does not define`);
        continue;
      }
      if (!advertised.get(id).has(scenario.id)) {
        errors.push(
          `benchmark: ${scenario.id} uses fixture ${id}, but that fixture does not advertise the scenario`,
        );
      }
    }
  }

  return errors;
}

/** Validates the readable document's tables against the data file. */
export function checkBenchmarkDocument(benchmark, markdown) {
  const errors = [];
  const tables = parseTables(markdown);

  const compare = (header, expected, label) => {
    const actual = tables.get(header.join('\u0000'));
    const wanted = expected.map((cells) => cells.map(normaliseCell));
    if (!actual) {
      errors.push(`benchmark document: the ${label} table is missing`);
    } else if (JSON.stringify(actual) !== JSON.stringify(wanted)) {
      errors.push(
        `benchmark document: the ${label} table is ${JSON.stringify(actual)}, expected ${JSON.stringify(wanted)}`,
      );
    }
  };

  compare(
    ['项', '取值'],
    Object.entries(benchmark.hardware?.record ?? {}).map(([key, value]) => [key, String(value)]),
    'hardware',
  );
  compare(
    ['项', '定义'],
    PROTOCOL_FIELDS.map((field) => {
      const value = benchmark.protocol?.[field];
      // Everything is compared as text: the table holds strings, and a number that renders the same
      // is not a difference worth failing on.
      return [field, Array.isArray(value) ? value.join(', ') : String(value)];
    }),
    'protocol',
  );
  compare(['状态', '含义'], Object.entries(benchmark.statusVocabulary ?? {}), 'status');
  compare(
    ['编号', '场景', '数据集', '指标', '统计', '门槛', '验收阶段', '状态', '归属'],
    (benchmark.scenarios ?? []).map((scenario) => [
      scenario.id,
      scenario.scenario,
      scenario.fixtures.map((id) => `\`${id}\``).join(', '),
      scenario.metric,
      scenario.statistic,
      scenario.threshold,
      scenario.phase,
      scenario.status,
      `\`${scenario.owner}\``,
    ]),
    'scenario',
  );

  return errors;
}

/** Runs every half over the repository's files. */
export function checkBenchmark(root) {
  const dataFile = path.join(root, BENCHMARK_PATH);
  const documentFile = path.join(root, DOCUMENT_PATH);
  for (const file of [dataFile, documentFile]) {
    if (!existsSync(file)) {
      return { errors: [`benchmark: missing ${path.relative(root, file)}`], stats: null };
    }
  }

  let benchmark;
  try {
    benchmark = JSON.parse(readFileSync(dataFile, 'utf8'));
  } catch (error) {
    return { errors: [`benchmark: ${BENCHMARK_PATH} is not valid JSON: ${error.message}`], stats: null };
  }

  const errors = [
    ...checkBenchmarkData(benchmark, root),
    ...checkBenchmarkDocument(benchmark, readFileSync(documentFile, 'utf8')),
  ];

  const tally = {};
  for (const scenario of benchmark.scenarios ?? []) {
    tally[scenario.status] = (tally[scenario.status] ?? 0) + 1;
  }

  return {
    errors,
    stats: {
      scenarios: (benchmark.scenarios ?? []).length,
      measured: tally.measured ?? 0,
      tally,
      hardware: Object.keys(benchmark.hardware?.record ?? {}).length,
    },
  };
}

export function describe(stats) {
  const statuses = Object.entries(stats.tally).map(([status, count]) => `${count} ${status}`).join(', ');
  return `${stats.scenarios} scenarios (${statuses}), ${stats.hardware} hardware facts recorded`;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
  const result = checkBenchmark(root);
  if (result.errors.length > 0) {
    console.error(result.errors.join('\n'));
    process.exitCode = 1;
  } else {
    console.log(`Benchmark environment checks passed (${describe(result.stats)}).`);
  }
}
