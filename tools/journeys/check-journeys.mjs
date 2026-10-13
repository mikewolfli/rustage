#!/usr/bin/env node
// SPDX-License-Identifier: MIT
//
// Journey-walkthrough checks: `docs/plan/journeys.json`, the readable copy in
// `docs/plan/journeys.md`, roadmap section 34 (where the journeys are defined) and
// `resources/host/icons/manifest.json` (where the menu groups and panels are declared).
//
// Scope, stated honestly. This gate checks:
//
//   1. The walkthrough covers exactly the journeys of roadmap 34 - same ids, names, evidence,
//      milestones and, step by step, the same operation chains.
//   2. Every step names an entry that exists: a menu leaf under a declared group glob, or a
//      declared panel. This is how "the walkthrough may refine a menu but may not invent a branch"
//      is enforced.
//   3. Every step names an owning task that exists in the roadmap, and a step with no entry says
//      why. That is the acceptance line: no step without somewhere to click, and no missing entry
//      without an owner.
//   4. Every journey records the four aspects roadmap 34 requires (normal flow, cancel/error,
//      recovery, keyboard) as a step index in range.
//   5. Every panel used as an entry is shown by at least one mode, so a journey cannot depend on a
//      panel no mode displays.
//   6. The document's tables agree with the data file, row for row.
//
// What it cannot check: whether the chosen entry is the *right* place for that step, and whether a
// journey is realistically achievable. Those are design judgements; the document says so, and the
// data file records the owning task so the judgement is revisited when that task lands.

import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const JOURNEYS_PATH = 'docs/plan/journeys.json';
export const DOCUMENT_PATH = 'docs/plan/journeys.md';

export const SCHEMA = 'rustage.journeys/v1';

/** The four aspects roadmap 34 requires of every journey. */
export const ASPECTS = ['normal', 'cancel', 'recovery', 'keyboard'];

/** Step scopes. `future` marks the journeys the roadmap keeps out of release acceptance. */
const SCOPES = ['release', 'future'];

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

/** The rows of one table in a section, as arrays of cells. */
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

/** Roadmap 34's journeys: id, name, evidence, milestone, and the operation chain split into steps. */
export function parseRoadmapJourneys(markdown, heading) {
  const tables = sectionTables(markdown, heading);
  if (!tables) return null;
  for (const [, rows] of tables) {
    const journeys = [];
    for (const row of rows) {
      if (!/^J\d+\s/.test(row[0])) continue;
      const id = row[0].match(/^J\d+/)[0];
      journeys.push({
        id,
        name: row[0].slice(id.length).trim(),
        steps: row[1].split('\u2192').map((step) => step.trim()),
        evidence: row[2],
        milestone: row[3],
      });
    }
    if (journeys.length > 0) return journeys;
  }
  return null;
}

/** Every task id defined in the roadmap, so an owner cannot be invented. */
export function parseRoadmapTasks(markdown) {
  const tasks = new Set();
  for (const match of markdown.matchAll(/^\|\s*(T\d{2}\.\d{2})\s*\|/gm)) tasks.add(match[1]);
  return tasks;
}

/** The menu groups and panels the resource manifest declares. */
export function parseManifest(manifest) {
  const groups = (manifest?.commands ?? []).map((entry) => entry.group);
  const panels = (manifest?.panels ?? []).map((entry) => entry.id);
  return { groups, panels };
}

/** Whether an entry matches a declared group glob (`file.new.*`) or is a declared panel. */
export function entryExists(entry, manifest) {
  const { groups, panels } = manifest;
  if (panels.includes(entry)) return true;
  return groups.some((group) => {
    const pattern = `^${group.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '[^.]+')}$`;
    return new RegExp(pattern).test(entry);
  });
}

function requireText(errors, where, value) {
  if (typeof value !== 'string' || value.trim() === '') {
    errors.push(`journeys: ${where} must be a non-empty string`);
  }
}

/** Validates the walkthrough against the roadmap and the manifest. */
export function checkJourneysData(journeys, root) {
  const errors = [];
  if (!journeys || typeof journeys !== 'object') return ['journeys: the data file is not an object'];
  if (journeys.schema !== SCHEMA) {
    errors.push(`journeys: schema is ${JSON.stringify(journeys.schema)}, expected ${SCHEMA}`);
  }
  if (journeys.implementation !== 'none') {
    errors.push(
      'journeys: `implementation` must stay "none" until a task actually wires an entry up; '
      + 'the walkthrough records designs and owners, not working menus',
    );
  }

  const roadmapFile = journeys.roadmap?.file;
  if (typeof roadmapFile !== 'string' || !existsSync(path.join(root, roadmapFile))) {
    return [...errors, `journeys: missing ${roadmapFile ?? 'roadmap'}`];
  }
  const roadmap = readFileSync(path.join(root, roadmapFile), 'utf8');

  const expected = parseRoadmapJourneys(roadmap, journeys.roadmap.section);
  if (!expected) {
    return [...errors, `journeys: the roadmap has no ${journeys.roadmap.section} journey table`];
  }
  const tasks = parseRoadmapTasks(roadmap);

  const manifestFile = journeys.manifest;
  if (typeof manifestFile !== 'string' || !existsSync(path.join(root, manifestFile))) {
    return [...errors, `journeys: missing ${manifestFile ?? 'manifest'}`];
  }
  const manifest = parseManifest(JSON.parse(readFileSync(path.join(root, manifestFile), 'utf8')));

  const modes = journeys.modes;
  const modeIds = new Set();
  const shownPanels = new Set();
  if (!Array.isArray(modes) || modes.length === 0) {
    errors.push('journeys: `modes` must be a non-empty list');
  } else {
    for (const [index, mode] of modes.entries()) {
      const where = `modes[${index}]`;
      for (const field of ['id', 'name', 'purpose']) requireText(errors, `${where}.${field}`, mode?.[field]);
      if (typeof mode?.id === 'string') {
        if (modeIds.has(mode.id)) errors.push(`journeys: duplicate mode ${mode.id}`);
        modeIds.add(mode.id);
      }
      if (!Array.isArray(mode?.panels) || mode.panels.length === 0) {
        errors.push(`journeys: ${mode?.id} shows no panel, so it is not a mode`);
      }
      for (const panel of mode?.panels ?? []) {
        if (!manifest.panels.includes(panel)) {
          errors.push(`journeys: mode ${mode.id} shows an undeclared panel ${panel}`);
        }
        shownPanels.add(panel);
      }
      if (!Array.isArray(mode?.entries) || mode.entries.length === 0) {
        errors.push(`journeys: ${mode?.id} lists no representative entry`);
      }
      for (const entry of mode?.entries ?? []) {
        if (!entryExists(entry, manifest)) {
          errors.push(`journeys: mode ${mode.id} lists an entry that does not exist: ${entry}`);
        }
      }
    }
  }

  const journeysList = journeys.journeys;
  if (!Array.isArray(journeysList) || journeysList.length === 0) {
    errors.push('journeys: `journeys` must be a non-empty list');
    return errors;
  }
  if (journeysList.length !== expected.length) {
    errors.push(
      `journeys: the walkthrough covers ${journeysList.length} journeys, the roadmap defines ${expected.length}`,
    );
  }

  const seenJourneys = new Set();
  for (const [index, journey] of journeysList.entries()) {
    const where = `journeys[${index}]`;
    const reference = expected.find((candidate) => candidate.id === journey?.id);
    if (!reference) {
      errors.push(`journeys: ${journey?.id} is not a journey in the roadmap`);
      continue;
    }
    if (seenJourneys.has(journey.id)) errors.push(`journeys: duplicate journey ${journey.id}`);
    seenJourneys.add(journey.id);
    if (journey.name !== reference.name) {
      errors.push(`journeys: ${journey.id} is named ${JSON.stringify(journey.name)}, roadmap says ${JSON.stringify(reference.name)}`);
    }
    if (journey.evidence !== reference.evidence) {
      errors.push(`journeys: ${journey.id} evidence drifted from the roadmap`);
    }
    if (journey.milestone !== reference.milestone) {
      errors.push(`journeys: ${journey.id} milestone is ${JSON.stringify(journey.milestone)}, roadmap says ${JSON.stringify(reference.milestone)}`);
    }
    if (!SCOPES.includes(journey.scope)) {
      errors.push(`journeys: ${journey.id} has scope ${JSON.stringify(journey.scope)}`);
    }
    const roadmapFuture = reference.milestone.startsWith('FUTURE');
    if (roadmapFuture !== (journey.scope === 'future')) {
      errors.push(
        `journeys: ${journey.id} is marked ${journey.scope} while the roadmap says ${reference.milestone}`,
      );
    }

    const steps = journey.steps;
    if (!Array.isArray(steps) || steps.length !== reference.steps.length) {
      errors.push(
        `journeys: ${journey.id} has ${steps?.length ?? 0} steps, the roadmap chain has ${reference.steps.length}`,
      );
      continue;
    }
    for (const [stepIndex, step] of steps.entries()) {
      const label = `${journey.id} step ${stepIndex + 1}`;
      if (step.step !== reference.steps[stepIndex]) {
        errors.push(
          `journeys: ${label} is ${JSON.stringify(step.step)}, roadmap says ${JSON.stringify(reference.steps[stepIndex])}`,
        );
      }
      if (step.entry === null) {
        requireText(errors, `${label}.reason`, step.reason);
      } else if (typeof step.entry !== 'string' || !entryExists(step.entry, manifest)) {
        errors.push(`journeys: ${label} entry ${JSON.stringify(step.entry)} is neither a panel nor a menu leaf`);
      } else if (manifest.panels.includes(step.entry) && !shownPanels.has(step.entry)) {
        errors.push(
          `journeys: ${label} uses panel ${step.entry}, which no mode shows, so it would be undiscoverable`,
        );
      }
      if (typeof step.owner !== 'string' || (step.owner !== 'FUTURE' && !tasks.has(step.owner))) {
        errors.push(`journeys: ${label} owner ${JSON.stringify(step.owner)} is not a roadmap task`);
      }
    }

    for (const aspect of ASPECTS) {
      const value = journey.aspects?.[aspect];
      if (!Number.isInteger(value) || value < 0 || value >= reference.steps.length) {
        errors.push(`journeys: ${journey.id} aspect ${aspect} does not point at one of its steps`);
      }
    }
    if (!Array.isArray(journey.checks) || journey.checks.length !== ASPECTS.length) {
      errors.push(`journeys: ${journey.id} must record one check per aspect`);
    }
  }

  for (const reference of expected) {
    if (!seenJourneys.has(reference.id)) errors.push(`journeys: ${reference.id} is missing from the walkthrough`);
  }

  return errors;
}

/** Validates the readable document's tables against the data file. */
export function checkJourneysDocument(journeys, markdown) {
  const errors = [];
  const tables = parseTables(markdown);
  const dash = '\u2014';

  const compare = (header, expected, label) => {
    const actual = tables.get(header.join('\u0000'));
    const wanted = expected.map((cells) => cells.map(normaliseCell));
    if (!actual) {
      errors.push(`journeys document: the ${label} table is missing`);
    } else if (JSON.stringify(actual) !== JSON.stringify(wanted)) {
      errors.push(
        `journeys document: the ${label} table is ${JSON.stringify(actual)}, expected ${JSON.stringify(wanted)}`,
      );
    }
  };

  const list = (values) => values.map((value) => `\`${value}\``).join(', ');
  compare(
    ['模式', '名称', '作用', '默认面板', '代表入口'],
    (journeys.modes ?? []).map((mode) => [
      mode.id, mode.name, mode.purpose, list(mode.panels), list(mode.entries),
    ]),
    'mode',
  );

  const stepText = (journey, aspect) => {
    const index = journey.aspects?.[aspect];
    const step = journey.steps?.[index];
    return step ? `${index + 1} ${step.step}` : '';
  };
  compare(
    ['旅程', '名称', '范围', '必需证据', '首验/收敛', '正常流程', '取消/错误', '恢复', '键盘'],
    (journeys.journeys ?? []).map((journey) => [
      journey.id,
      journey.name,
      journey.scope,
      journey.evidence,
      journey.milestone,
      ...ASPECTS.map((aspect) => stepText(journey, aspect)),
    ]),
    'journey',
  );

  compare(
    ['旅程', '步骤', '入口', '归属任务', '说明'],
    (journeys.journeys ?? []).flatMap((journey) => journey.steps.map((step, index) => [
      journey.id,
      `${index + 1}. ${step.step}`,
      step.entry === null ? '' : `\`${step.entry}\``,
      `\`${step.owner}\``,
      step.reason ?? '',
    ])),
    'step',
  );

  return errors;
}

/** Runs every half over the repository's files. */
export function checkJourneys(root) {
  const dataFile = path.join(root, JOURNEYS_PATH);
  const documentFile = path.join(root, DOCUMENT_PATH);
  for (const file of [dataFile, documentFile]) {
    if (!existsSync(file)) {
      return { errors: [`journeys: missing ${path.relative(root, file)}`], stats: null };
    }
  }

  let journeys;
  try {
    journeys = JSON.parse(readFileSync(dataFile, 'utf8'));
  } catch (error) {
    return { errors: [`journeys: ${JOURNEYS_PATH} is not valid JSON: ${error.message}`], stats: null };
  }

  const errors = [
    ...checkJourneysData(journeys, root),
    ...checkJourneysDocument(journeys, readFileSync(documentFile, 'utf8')),
  ];

  const steps = (journeys.journeys ?? []).flatMap((journey) => journey.steps ?? []);
  return {
    errors,
    stats: {
      modes: (journeys.modes ?? []).length,
      journeys: (journeys.journeys ?? []).length,
      steps: steps.length,
      stepsWithEntry: steps.filter((step) => step.entry !== null).length,
      release: (journeys.journeys ?? []).filter((journey) => journey.scope === 'release').length,
      future: (journeys.journeys ?? []).filter((journey) => journey.scope === 'future').length,
    },
  };
}

export function describe(stats) {
  return `${stats.modes} modes, ${stats.journeys} journeys (${stats.release} release, ${stats.future} future), `
    + `${stats.steps} steps (${stats.stepsWithEntry} with a designated entry, `
    + `${stats.steps - stats.stepsWithEntry} owned without one)`;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
  const result = checkJourneys(root);
  if (result.errors.length > 0) {
    console.error(result.errors.join('\n'));
    process.exitCode = 1;
  } else {
    console.log(`Journey walkthrough checks passed (${describe(result.stats)}).`);
  }
}
