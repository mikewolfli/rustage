#!/usr/bin/env node
// SPDX-License-Identifier: MIT
//
// Capability-reference checks: `docs/plan/capability-reference.json`, the readable copy in
// `docs/plan/capability-reference.md`, and the roadmap's milestone table.
//
// Scope, stated honestly. This gate checks the four things T00.09 asks for:
//
//   1. Every supplementary requirement says where it comes from, what was decided about it (adopt,
//      defer or not-do), why, and which tasks, journeys and gates carry it. "Every new requirement
//      has a task, a journey and a gate" is exactly that rule.
//   2. Every phase is a milestone the roadmap defines, and every adopted item's dependencies land no
//      later than the phase that signs it off. The milestone order is read from the roadmap's own
//      milestone table, so this cannot drift from the plan.
//   3. Every protocol version names its current value and its owner, so "which version does this
//      build speak" has one answer per protocol.
//   4. The document's tables agree with the data file.
//
// What it cannot check: whether a deferral is wise, and whether an adopted requirement is really
// delivered by the tasks it names. The rationales and owners are where that argument lives.

import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const CAPABILITY_PATH = 'docs/plan/capability-reference.json';
export const DOCUMENT_PATH = 'docs/plan/capability-reference.md';

export const SCHEMA = 'rustage.capability/v1';

/** The decisions an item may carry. */
export const DECISIONS = ['adopt', 'defer', 'not-do'];

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

/** The rows of the milestone table, in roadmap order. */
export function parseMilestones(markdown, heading) {
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
  const tables = parseTables(lines.slice(start, end).join('\n'));
  for (const [, rows] of tables) {
    const milestones = rows
      .filter((row) => row.length === 4)
      .map((row) => ({ id: row[0].match(/^(MS\d+|FUTURE)/)?.[0], delivery: row[1] }))
      .filter((milestone) => milestone.id);
    if (milestones.length > 0) return milestones;
  }
  return null;
}

/**
 * Where each task belongs, from the milestone table: explicit task ranges first, then the domains
 * the row delivers (`F07` owns `T07.*`). Returns `Map<taskId, milestoneId>`.
 */
export function parseTaskPhases(milestones) {
  const phases = new Map();
  const rangePattern = /T(\d{2})\.(\d{2})[–-]T(\d{2})\.(\d{2})/g;
  const singlePattern = /T(\d{2})\.(\d{2})/g;

  for (const milestone of milestones) {
    const covered = new Set();
    for (const match of milestone.delivery.matchAll(rangePattern)) {
      const [, fromDomain, fromIndex, toDomain, toIndex] = match;
      if (fromDomain !== toDomain) continue;
      for (let index = Number(fromIndex); index <= Number(toIndex); index += 1) {
        covered.add(`T${fromDomain}.${String(index).padStart(2, '0')}`);
      }
    }
    for (const match of milestone.delivery.matchAll(singlePattern)) {
      covered.add(`T${match[1]}.${match[2]}`);
    }
    for (const task of covered) if (!phases.has(task)) phases.set(task, milestone.id);

    for (const match of milestone.delivery.matchAll(/F(\d{2})[–-]F(\d{2})/g)) {
      for (let domain = Number(match[1]); domain <= Number(match[2]); domain += 1) {
        const id = `F${String(domain).padStart(2, '0')}`;
        for (let index = 1; index <= 20; index += 1) {
          const task = `${id.replace('F', 'T')}.${String(index).padStart(2, '0')}`;
          if (!phases.has(task)) phases.set(task, milestone.id);
        }
      }
    }
    for (const match of milestone.delivery.matchAll(/F(\d{2})/g)) {
      const task = `T${match[1]}`;
      for (let index = 1; index <= 20; index += 1) {
        const candidate = `${task}.${String(index).padStart(2, '0')}`;
        if (!phases.has(candidate)) phases.set(candidate, milestone.id);
      }
    }
  }
  return phases;
}

/** The milestone order as the roadmap lists it, `FUTURE` last. */
export function milestoneOrder(milestones) {
  return new Map(milestones.map((milestone, index) => [milestone.id, index]));
}

function requireText(errors, where, value) {
  if (typeof value !== 'string' || value.trim() === '') {
    errors.push(`capability: ${where} must be a non-empty string`);
  }
}

function requireList(errors, where, value) {
  if (!Array.isArray(value) || value.length === 0) {
    errors.push(`capability: ${where} must be a non-empty list`);
    return false;
  }
  return true;
}

/** Validates the capability data against the roadmap's milestones and the other registers. */
export function checkCapabilityData(data, root) {
  const errors = [];
  if (!data || typeof data !== 'object') return ['capability: the data file is not an object'];
  if (data.schema !== SCHEMA) {
    errors.push(`capability: schema is ${JSON.stringify(data.schema)}, expected ${SCHEMA}`);
  }

  const roadmapFile = data.roadmap?.file;
  if (typeof roadmapFile !== 'string' || !existsSync(path.join(root, roadmapFile))) {
    return [...errors, `capability: missing ${roadmapFile ?? 'roadmap'}`];
  }
  const roadmap = readFileSync(path.join(root, roadmapFile), 'utf8');
  const milestones = parseMilestones(roadmap, data.roadmap.section);
  if (!milestones) return [...errors, `capability: the roadmap has no ${data.roadmap.section} table`];
  const order = milestoneOrder(milestones);
  const phases = parseTaskPhases(milestones);

  const taskIds = new Set();
  for (const match of roadmap.matchAll(/^\|\s*(T\d{2}\.\d{2})\s*\|/gm)) taskIds.add(match[1]);
  const journeyIds = new Set();
  for (const match of roadmap.matchAll(/^\|\s*(J\d+)\s/gm)) journeyIds.add(match[1]);

  const gatesFile = 'docs/plan/traceability.json';
  const gateIds = existsSync(path.join(root, gatesFile))
    ? new Set((JSON.parse(readFileSync(path.join(root, gatesFile), 'utf8')).gates ?? []).map((gate) => gate.id))
    : new Set();

  const decisionVocabulary = data.decisions;
  if (!decisionVocabulary || typeof decisionVocabulary !== 'object') {
    errors.push('capability: `decisions` must define the vocabulary');
  } else {
    for (const decision of DECISIONS) {
      requireText(errors, `decisions.${decision}`, decisionVocabulary[decision]);
    }
  }

  const protocols = data.protocols;
  const protocolIds = new Set();
  if (!Array.isArray(protocols) || protocols.length === 0) {
    errors.push('capability: `protocols` must be a non-empty list');
  } else {
    for (const [index, protocol] of protocols.entries()) {
      const where = `protocols[${index}]`;
      for (const field of ['id', 'name', 'current', 'owner', 'meaning']) {
        requireText(errors, `${where}.${field}`, protocol?.[field]);
      }
      if (typeof protocol?.id === 'string') {
        if (protocolIds.has(protocol.id)) errors.push(`capability: duplicate protocol ${protocol.id}`);
        protocolIds.add(protocol.id);
      }
      if (typeof protocol?.owner === 'string' && protocol.owner !== 'FUTURE' && !taskIds.has(protocol.owner)) {
        errors.push(`capability: ${protocol?.id} is owned by ${protocol.owner}, which the roadmap does not define`);
      }
    }
  }

  const items = data.items;
  if (!Array.isArray(items) || items.length === 0) {
    errors.push('capability: `items` must be a non-empty list');
    return errors;
  }

  const seen = new Set();
  for (const [index, item] of items.entries()) {
    const where = `items[${index}]`;
    for (const field of ['id', 'statement', 'source', 'decision', 'rationale', 'phase']) {
      requireText(errors, `${where}.${field}`, item?.[field]);
    }
    if (typeof item?.id === 'string') {
      if (seen.has(item.id)) errors.push(`capability: duplicate item ${item.id}`);
      seen.add(item.id);
    }
    if (!DECISIONS.includes(item?.decision)) {
      errors.push(`capability: ${item?.id} has decision ${JSON.stringify(item?.decision)}`);
    }
    if (!order.has(item?.phase)) {
      errors.push(`capability: ${item?.id} is signed off in ${JSON.stringify(item?.phase)}, which is not a roadmap milestone`);
      continue;
    }
    if (requireList(errors, `${item?.id}.tasks`, item?.tasks)) {
      for (const task of item.tasks) {
        if (!taskIds.has(task)) {
          errors.push(`capability: ${item.id} names task ${task}, which the roadmap does not define`);
        }
      }
    }
    if (requireList(errors, `${item?.id}.journeys`, item?.journeys)) {
      for (const journey of item.journeys) {
        if (!journeyIds.has(journey)) {
          errors.push(`capability: ${item.id} names journey ${journey}, which the roadmap does not define`);
        }
      }
    }
    if (requireList(errors, `${item?.id}.gates`, item?.gates)) {
      for (const gate of item.gates) {
        if (!gateIds.has(gate)) {
          errors.push(`capability: ${item.id} names gate ${gate}, which the traceability document does not declare`);
        }
      }
    }
    if (!Array.isArray(item?.dependsOn) || item.dependsOn.length === 0) {
      errors.push(`capability: ${item?.id}.dependsOn must name at least one task it waits for`);
      continue;
    }
    const phaseIndex = order.get(item.phase);
    for (const task of item.dependsOn) {
      if (!taskIds.has(task)) {
        errors.push(`capability: ${item.id} depends on ${task}, which the roadmap does not define`);
        continue;
      }
      const dependencyPhase = phases.get(task);
      if (!dependencyPhase) continue;
      const dependencyIndex = order.get(dependencyPhase);
      if (dependencyIndex > phaseIndex) {
        errors.push(
          `capability: ${item.id} is signed off in ${item.phase} but depends on ${task} (${dependencyPhase})`,
        );
      }
    }
  }

  return errors;
}

/** Validates the readable document's tables against the data file. */
export function checkCapabilityDocument(data, markdown) {
  const errors = [];
  const tables = parseTables(markdown);
  const dash = '\u2014';

  const compare = (header, expected, label) => {
    const actual = tables.get(header.join('\u0000'));
    const wanted = expected.map((cells) => cells.map(normaliseCell));
    if (!actual) {
      errors.push(`capability document: the ${label} table is missing`);
    } else if (JSON.stringify(actual) !== JSON.stringify(wanted)) {
      errors.push(
        `capability document: the ${label} table is ${JSON.stringify(actual)}, expected ${JSON.stringify(wanted)}`,
      );
    }
  };

  compare(
    ['编号', '补充需求', '出处', '决定', '理由', '任务', '旅程', '门禁', '签收阶段', '依赖'],
    (data.items ?? []).map((item) => [
      item.id,
      item.statement,
      item.source,
      item.decision,
      item.rationale,
      item.tasks.map((task) => `\`${task}\``).join(', '),
      item.journeys.join(', '),
      item.gates.join(', '),
      item.phase,
      item.dependsOn.map((task) => `\`${task}\``).join(', '),
    ]),
    'item',
  );

  compare(
    ['编号', '协议', '当前版本', '归属任务', '含义'],
    (data.protocols ?? []).map((protocol) => [
      protocol.id, protocol.name, protocol.current, `\`${protocol.owner}\``, protocol.meaning,
    ]),
    'protocol',
  );

  compare(
    ['决定', '含义'],
    DECISIONS.map((decision) => [decision, data.decisions?.[decision] ?? dash]),
    'decision',
  );

  return errors;
}

/** Runs every half over the repository's files. */
export function checkCapability(root) {
  const dataFile = path.join(root, CAPABILITY_PATH);
  const documentFile = path.join(root, DOCUMENT_PATH);
  for (const file of [dataFile, documentFile]) {
    if (!existsSync(file)) {
      return { errors: [`capability: missing ${path.relative(root, file)}`], stats: null };
    }
  }

  let data;
  try {
    data = JSON.parse(readFileSync(dataFile, 'utf8'));
  } catch (error) {
    return { errors: [`capability: ${CAPABILITY_PATH} is not valid JSON: ${error.message}`], stats: null };
  }

  const errors = [
    ...checkCapabilityData(data, root),
    ...checkCapabilityDocument(data, readFileSync(documentFile, 'utf8')),
  ];

  const tally = {};
  for (const item of data.items ?? []) tally[item.decision] = (tally[item.decision] ?? 0) + 1;

  return {
    errors,
    stats: {
      items: (data.items ?? []).length,
      protocols: (data.protocols ?? []).length,
      tally,
    },
  };
}

export function describe(stats) {
  const decisions = Object.entries(stats.tally).map(([decision, count]) => `${count} ${decision}`).join(', ');
  return `${stats.items} supplementary requirements (${decisions}), ${stats.protocols} protocol versions`;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
  const result = checkCapability(root);
  if (result.errors.length > 0) {
    console.error(result.errors.join('\n'));
    process.exitCode = 1;
  } else {
    console.log(`Capability reference checks passed (${describe(result.stats)}).`);
  }
}
