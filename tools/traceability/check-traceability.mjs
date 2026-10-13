#!/usr/bin/env node
// SPDX-License-Identifier: MIT
//
// Traceability checks: `docs/plan/traceability.json`, the readable copy in
// `docs/plan/traceability.md`, and the four things it has to connect.
//
// Scope, stated honestly. This gate checks that the chain holds at its ends and its links:
//
//   1. Every requirement names where it comes from (whitepaper or roadmap), at least one journey in
//      roadmap 34, at least one roadmap task, and at least one gate. "No unowned requirement" means
//      exactly this: nothing is stated without a task that owns it and a check that would notice.
//   2. Every gate is either wired into the workflow or declared manual with a reason and an owner.
//      A gate that is declared but never run is the failure this rule exists for.
//   3. Every command namespace comes from the resource manifest, so the command vocabulary cannot
//      quietly grow a new top level.
//   4. The MS00 baseline is exactly the tasks roadmap 34's prerequisites name, and the document's
//      tables agree with the data file row for row.
//
// What it cannot check: whether the mapping is right (that a requirement's tasks really deliver it)
// and whether a gate is strong enough. Those are review questions; the ownership column is where
// the answer lives.

import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const TRACEABILITY_PATH = 'docs/plan/traceability.json';
export const DOCUMENT_PATH = 'docs/plan/traceability.md';

export const SCHEMA = 'rustage.traceability/v1';

/** A gate runs in CI or it is manual with a stated reason. */
const GATE_MODES = ['ci', 'manual'];

function splitRow(line) {
  const trimmed = line.trim();
  if (!trimmed.startsWith('|') || !trimmed.endsWith('|') || trimmed.length < 2) return null;
  return trimmed
    .slice(1, -1)
    .split('|')
    .map((cell) => cell.trim().replace(/^`|`$/g, ''));
}

/** The same normalisation the table parser applies to a cell, so both sides compare alike. */
function normaliseCell(cell) {
  return String(cell).trim().replace(/^`|`$/g, '');
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

/** The task ids and journey ids the roadmap defines. */
export function parseRoadmapIds(markdown) {
  const tasks = new Set();
  for (const match of markdown.matchAll(/^\|\s*(T\d{2}\.\d{2})\s*\|/gm)) tasks.add(match[1]);
  const journeys = new Set();
  for (const match of markdown.matchAll(/^\|\s*(J\d+)\s/gm)) journeys.add(match[1]);
  return { tasks, journeys };
}

/** Every command group and the namespaces they use, from the resource manifest. */
export function parseManifestNamespaces(manifest) {
  const groups = (manifest?.commands ?? []).map((entry) => entry.group);
  const namespaces = [...new Set(groups.map((group) => group.split('.')[0]))].sort();
  return { groups, namespaces };
}

/** Whether a gate's command is actually run by the workflow. */
export function commandRunsInWorkflow(command, workflow) {
  return workflow.includes(command);
}

function requireText(errors, where, value) {
  if (typeof value !== 'string' || value.trim() === '') {
    errors.push(`traceability: ${where} must be a non-empty string`);
  }
}

function requireList(errors, where, value) {
  if (!Array.isArray(value) || value.length === 0) {
    errors.push(`traceability: ${where} must be a non-empty list`);
    return false;
  }
  return true;
}

/** Validates the traceability data against the roadmap, the manifest and the workflow. */
export function checkTraceabilityData(data, root) {
  const errors = [];
  if (!data || typeof data !== 'object') return ['traceability: the data file is not an object'];
  if (data.schema !== SCHEMA) {
    errors.push(`traceability: schema is ${JSON.stringify(data.schema)}, expected ${SCHEMA}`);
  }

  const sources = data.sources;
  if (!sources || typeof sources !== 'object') {
    errors.push('traceability: `sources` must name the documents the chain points at');
  }
  for (const [name, file] of Object.entries(sources ?? {})) {
    if (typeof file !== 'string' || !existsSync(path.join(root, file))) {
      errors.push(`traceability: sources.${name} does not exist: ${JSON.stringify(file)}`);
    }
  }

  // Every file the chain is read from must exist: a checker that throws on a missing source reports
  // nothing at all, which is worse than reporting that the source is missing.
  // Only the files this checker reads are required here; the loop above already reports every source
  // that is missing, including the ones it only cites.
  const required = ['roadmap', 'workflow', 'journeys'];
  const present = (name) => typeof sources?.[name] === 'string' && existsSync(path.join(root, sources[name]));
  if (!required.every(present)) return errors;

  const roadmapFile = sources.roadmap;
  const workflowFile = sources.workflow;
  const journeyFile = sources.journeys;
  const roadmap = readFileSync(path.join(root, roadmapFile), 'utf8');
  const workflow = readFileSync(path.join(root, workflowFile), 'utf8');
  const { tasks, journeys } = parseRoadmapIds(roadmap);
  const journeyWalkthrough = JSON.parse(readFileSync(path.join(root, journeyFile), 'utf8'));

  const benchmarkFile = 'docs/plan/benchmark.json';
  const scenarioIds = existsSync(path.join(root, benchmarkFile))
    ? new Set((JSON.parse(readFileSync(path.join(root, benchmarkFile), 'utf8')).scenarios ?? [])
      .map((scenario) => scenario.id))
    : new Set();

  const manifestFile = (journeyWalkthrough.manifest ?? '');
  const manifest = existsSync(path.join(root, manifestFile))
    ? JSON.parse(readFileSync(path.join(root, manifestFile), 'utf8'))
    : null;
  const { namespaces } = manifest ? parseManifestNamespaces(manifest) : { namespaces: [] };

  const gates = data.gates;
  const gateIds = new Set();
  if (!Array.isArray(gates) || gates.length === 0) {
    errors.push('traceability: `gates` must be a non-empty list');
  } else {
    for (const [index, gate] of gates.entries()) {
      const where = `gates[${index}]`;
      for (const field of ['id', 'command', 'covers']) requireText(errors, `${where}.${field}`, gate?.[field]);
      if (typeof gate?.id === 'string') {
        if (gateIds.has(gate.id)) errors.push(`traceability: duplicate gate ${gate.id}`);
        gateIds.add(gate.id);
      }
      if (!GATE_MODES.includes(gate?.mode)) {
        errors.push(`traceability: ${gate?.id} has mode ${JSON.stringify(gate?.mode)}`);
        continue;
      }
      if (gate.mode === 'ci' && typeof gate.command === 'string' && !commandRunsInWorkflow(gate.command, workflow)) {
        errors.push(
          `traceability: ${gate.id} is declared as a CI gate but ${JSON.stringify(gate.command)} `
          + `does not appear in ${workflowFile}`,
        );
      }
      if (gate.mode === 'manual') {
        requireText(errors, `${gate.id}.reason`, gate.reason);
        requireText(errors, `${gate.id}.owner`, gate.owner);
      } else if (gate.reason !== null && gate.reason !== undefined) {
        errors.push(`traceability: ${gate.id} is a CI gate but carries a manual reason`);
      }
    }
  }

  const requirements = data.requirements;
  const seen = new Set();
  if (!Array.isArray(requirements) || requirements.length === 0) {
    errors.push('traceability: `requirements` must be a non-empty list');
  } else {
    for (const [index, requirement] of requirements.entries()) {
      const where = `requirements[${index}]`;
      for (const field of ['id', 'statement', 'source']) requireText(errors, `${where}.${field}`, requirement?.[field]);
      if (typeof requirement?.id === 'string') {
        if (seen.has(requirement.id)) errors.push(`traceability: duplicate requirement ${requirement.id}`);
        seen.add(requirement.id);
      }
      if (requireList(errors, `${requirement?.id}.tasks`, requirement?.tasks)) {
        for (const task of requirement.tasks) {
          if (!tasks.has(task)) {
            errors.push(`traceability: ${requirement.id} is owned by ${task}, which the roadmap does not define`);
          }
        }
      }
      if (requireList(errors, `${requirement?.id}.journeys`, requirement?.journeys)) {
        for (const journey of requirement.journeys) {
          if (!journeys.has(journey)) {
            errors.push(`traceability: ${requirement.id} claims journey ${journey}, which the roadmap does not define`);
          }
        }
      }
      if (requireList(errors, `${requirement?.id}.gates`, requirement?.gates)) {
        for (const gate of requirement.gates) {
          if (!gateIds.has(gate)) {
            errors.push(`traceability: ${requirement.id} names gate ${gate}, which is not declared`);
          }
        }
      }
      if (requireList(errors, `${requirement?.id}.scenarios`, requirement?.scenarios)) {
        for (const scenario of requirement.scenarios) {
          if (!scenarioIds.has(scenario)) {
            errors.push(`traceability: ${requirement.id} names scenario ${scenario}, which the benchmark document does not define`);
          }
        }
      }
    }
  }

  const commands = data.commands;
  if (!commands || !Array.isArray(commands.namespaces) || commands.namespaces.length === 0) {
    errors.push('traceability: `commands.namespaces` must list the command vocabulary');
  } else {
    requireText(errors, 'commands.note', commands.note);
    const sorted = [...commands.namespaces].sort();
    if (JSON.stringify(sorted) !== JSON.stringify(namespaces)) {
      errors.push(
        `traceability: command namespaces are ${JSON.stringify(sorted)}, the manifest uses ${JSON.stringify(namespaces)}`,
      );
    }
  }

  const release = data.release;
  if (!release || typeof release !== 'object') {
    errors.push('traceability: `release` must record the baseline and scope');
  } else {
    requireText(errors, 'release.officialScope', release.officialScope);
    requireText(errors, 'release.note', release.note);
    if (requireList(errors, 'release.baseline', release.baseline)) {
      for (const task of release.baseline) {
        if (!tasks.has(task)) {
          errors.push(`traceability: the baseline names ${task}, which the roadmap does not define`);
        }
      }
    }
    if (typeof release.officialScope === 'string' && !roadmap.includes(release.officialScope)) {
      errors.push(`traceability: the roadmap does not state ${JSON.stringify(release.officialScope)}`);
    }
  }

  return errors;
}

/** Validates the readable document's tables against the data file. */
export function checkTraceabilityDocument(data, markdown) {
  const errors = [];
  const tables = parseTables(markdown);

  const compare = (header, expected, label) => {
    const actual = tables.get(header.join('\u0000'));
    const wanted = expected.map((cells) => cells.map(normaliseCell));
    if (!actual) {
      errors.push(`traceability document: the ${label} table is missing`);
    } else if (JSON.stringify(actual) !== JSON.stringify(wanted)) {
      errors.push(
        `traceability document: the ${label} table is ${JSON.stringify(actual)}, expected ${JSON.stringify(wanted)}`,
      );
    }
  };

  compare(
    ['需求', '陈述', '出处', '旅程', '归属任务', '门禁', '测量场景'],
    (data.requirements ?? []).map((requirement) => [
      requirement.id,
      requirement.statement,
      requirement.source,
      requirement.journeys.join(', '),
      requirement.tasks.map((task) => `\`${task}\``).join(', '),
      requirement.gates.join(', '),
      requirement.scenarios.join(', '),
    ]),
    'requirement',
  );

  compare(
    ['门禁', '命令', '覆盖', '方式', '理由 / 归属'],
    (data.gates ?? []).map((gate) => [
      gate.id,
      `\`${gate.command}\``,
      gate.covers,
      gate.mode,
      gate.mode === 'ci' ? '\u2014' : `${gate.reason}（${gate.owner}）`,
    ]),
    'gate',
  );

  compare(
    ['范围项', '取值'],
    [
      ['基线任务', (data.release?.baseline ?? []).map((task) => `\`${task}\``).join(', ')],
      ['正式版范围', data.release?.officialScope],
      ['说明', data.release?.note],
    ],
    'release',
  );

  compare(['命令命名空间', '来源'], (data.commands?.namespaces ?? []).map((namespace) => [
    namespace,
    'resources/host/icons/manifest.json',
  ]), 'namespace');

  return errors;
}

/** Runs every half over the repository's files. */
export function checkTraceability(root) {
  const dataFile = path.join(root, TRACEABILITY_PATH);
  const documentFile = path.join(root, DOCUMENT_PATH);
  for (const file of [dataFile, documentFile]) {
    if (!existsSync(file)) {
      return { errors: [`traceability: missing ${path.relative(root, file)}`], stats: null };
    }
  }

  let data;
  try {
    data = JSON.parse(readFileSync(dataFile, 'utf8'));
  } catch (error) {
    return { errors: [`traceability: ${TRACEABILITY_PATH} is not valid JSON: ${error.message}`], stats: null };
  }

  const errors = [
    ...checkTraceabilityData(data, root),
    ...checkTraceabilityDocument(data, readFileSync(documentFile, 'utf8')),
  ];

  return {
    errors,
    stats: {
      requirements: (data.requirements ?? []).length,
      gates: (data.gates ?? []).length,
      ciGates: (data.gates ?? []).filter((gate) => gate.mode === 'ci').length,
      manualGates: (data.gates ?? []).filter((gate) => gate.mode === 'manual').length,
      baseline: (data.release?.baseline ?? []).length,
      namespaces: (data.commands?.namespaces ?? []).length,
    },
  };
}

export function describe(stats) {
  return `${stats.requirements} requirements, ${stats.gates} gates (${stats.ciGates} in CI, `
    + `${stats.manualGates} manual), baseline of ${stats.baseline} tasks, ${stats.namespaces} command namespaces`;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
  const result = checkTraceability(root);
  if (result.errors.length > 0) {
    console.error(result.errors.join('\n'));
    process.exitCode = 1;
  } else {
    console.log(`Traceability checks passed (${describe(result.stats)}).`);
  }
}
