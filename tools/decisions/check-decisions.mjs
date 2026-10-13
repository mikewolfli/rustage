#!/usr/bin/env node
// SPDX-License-Identifier: MIT
//
// Decision-register checks: `docs/plan/decisions.json` (the data), `docs/plan/decisions.md`
// (the readable copy) and whitepaper §17.7 (the place the suspended questions were asked).
//
// Scope, stated honestly: this proves the register is well formed (every entry carries a status
// from the fixed vocabulary, a decision, a rationale, at least one rejected alternative, a
// compatibility impact, a revisit trigger, an owner and at least one affected task), that every
// suspended question Q1-Q6 has an answer or an explicitly open entry, that the document's tables
// agree with the data row for row, and that whitepaper §17.7 still states the same conclusions.
// It does NOT prove the decisions are good, and it cannot notice a decision that is technically
// recorded but wrong.
//
// Why the whitepaper is part of the gate: the question lived there, and a decision recorded only
// in a new file would leave the whitepaper telling readers something else. Cross-file agreement is
// the whole point (principle #25, #59).

import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const DECISIONS_PATH = 'docs/plan/decisions.json';
export const DOCUMENT_PATH = 'docs/plan/decisions.md';

export const SCHEMA = 'rustage.decisions/v1';

/** Statuses an entry in `decisions` may use: it is a decision, so it is settled or pending confirmation. */
const DECIDED_STATUSES = ['ratified', 'provisional'];
/** Statuses an entry in `openDecisions` may use. */
const OPEN_STATUSES = ['open', 'deferred'];

const TASK_PATTERN = /^T\d{2}\.\d{2}$/;

/**
 * Splits a Markdown table row into trimmed cells, or returns null when the line is not a row.
 * Surrounding backticks are stripped so a value may be written as code or as plain text.
 */
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

/**
 * The text of one section, from its heading to the next heading of the same or a higher level.
 * The heading may be a prefix of the real heading (`### 17.7` matches `### 17.7 标题`) so a
 * section can be retitled without breaking the check.
 */
export function sectionText(markdown, heading) {
  const lines = markdown.split('\n');
  const wanted = heading.trim();
  const start = lines.findIndex((line) => {
    const text = line.trim();
    return text === wanted || text.startsWith(`${wanted} `);
  });
  if (start === -1) return null;
  const level = wanted.match(/^#+/)[0].length;
  for (let index = start + 1; index < lines.length; index += 1) {
    const match = lines[index].match(/^(#+)\s/);
    if (match && match[1].length <= level) return lines.slice(start, index).join('\n');
  }
  return lines.slice(start).join('\n');
}

function requireText(errors, where, value) {
  if (typeof value !== 'string' || value.trim() === '') {
    errors.push(`decisions: ${where} must be a non-empty string`);
    return false;
  }
  return true;
}

function requireTexts(errors, where, values) {
  if (!Array.isArray(values) || values.length === 0) {
    errors.push(`decisions: ${where} must be a non-empty list`);
    return false;
  }
  return values.every((value, index) => requireText(errors, `${where}[${index}]`, value));
}

/** Validates the register data alone, so vocabulary and completeness rules are testable without the Markdown. */
export function checkDecisionsData(register) {
  const errors = [];
  if (!register || typeof register !== 'object') return ['decisions: the register is not an object'];
  if (register.schema !== SCHEMA) {
    errors.push(`decisions: schema is ${JSON.stringify(register.schema)}, expected ${SCHEMA}`);
  }

  const statuses = register.statuses;
  if (!statuses || typeof statuses !== 'object' || Object.keys(statuses).length === 0) {
    errors.push('decisions: `statuses` must define the status vocabulary');
  } else {
    for (const [status, meaning] of Object.entries(statuses)) {
      requireText(errors, `statuses.${status}`, meaning);
    }
  }

  const whitepaper = register.whitepaper;
  if (!whitepaper || typeof whitepaper !== 'object') {
    errors.push('decisions: `whitepaper` must name the file, section and questions');
  } else {
    requireText(errors, 'whitepaper.file', whitepaper.file);
    requireText(errors, 'whitepaper.section', whitepaper.section);
    requireTexts(errors, 'whitepaper.questions', whitepaper.questions);
  }

  const decisions = Array.isArray(register.decisions) ? register.decisions : null;
  if (!decisions || decisions.length === 0) {
    errors.push('decisions: `decisions` must be a non-empty list');
  }

  const seen = new Set();
  const answered = new Set();
  for (const [index, entry] of (decisions ?? []).entries()) {
    const where = `decisions[${index}]`;
    if (!requireText(errors, `${where}.id`, entry?.id)) continue;
    if (seen.has(entry.id)) errors.push(`decisions: duplicate id ${entry.id}`);
    seen.add(entry.id);
    answered.add(entry.id);
    if (!Object.hasOwn(statuses ?? {}, entry.status)) {
      errors.push(`decisions: ${entry.id} has status ${JSON.stringify(entry.status)} outside the vocabulary`);
    } else if (!DECIDED_STATUSES.includes(entry.status)) {
      errors.push(`decisions: ${entry.id} is in \`decisions\` but has status ${entry.status}`);
    }
    requireText(errors, `${entry.id}.topic`, entry.topic);
    requireText(errors, `${entry.id}.summary`, entry.summary);
    requireText(errors, `${entry.id}.decision`, entry.decision);
    requireText(errors, `${entry.id}.rationale`, entry.rationale);
    requireText(errors, `${entry.id}.compatibilityImpact`, entry.compatibilityImpact);
    requireText(errors, `${entry.id}.revisitTrigger`, entry.revisitTrigger);
    requireText(errors, `${entry.id}.owner`, entry.owner);

    const alternatives = entry.alternatives;
    if (!Array.isArray(alternatives) || alternatives.length === 0) {
      errors.push(`decisions: ${entry.id} records no rejected alternative`);
    } else {
      for (const [optionIndex, alternative] of alternatives.entries()) {
        requireText(errors, `${entry.id}.alternatives[${optionIndex}].option`, alternative?.option);
        requireText(errors, `${entry.id}.alternatives[${optionIndex}].rejected`, alternative?.rejected);
      }
    }

    const affects = entry.affects;
    if (!Array.isArray(affects) || affects.length === 0) {
      errors.push(`decisions: ${entry.id} names no affected task`);
    } else {
      for (const [taskIndex, task] of affects.entries()) {
        if (typeof task !== 'string' || !TASK_PATTERN.test(task)) {
          errors.push(`decisions: ${entry.id}.affects[${taskIndex}] is not a task id: ${JSON.stringify(task)}`);
        }
      }
    }
  }

  const open = Array.isArray(register.openDecisions) ? register.openDecisions : null;
  if (!open) {
    errors.push('decisions: `openDecisions` must be a list, possibly empty');
  }
  for (const [index, entry] of (open ?? []).entries()) {
    const where = `openDecisions[${index}]`;
    if (!requireText(errors, `${where}.id`, entry?.id)) continue;
    if (seen.has(entry.id)) errors.push(`decisions: duplicate id ${entry.id}`);
    seen.add(entry.id);
    answered.add(entry.id);
    if (!Object.hasOwn(statuses ?? {}, entry.status)) {
      errors.push(`decisions: ${entry.id} has status ${JSON.stringify(entry.status)} outside the vocabulary`);
    } else if (!OPEN_STATUSES.includes(entry.status)) {
      errors.push(`decisions: ${entry.id} is in \`openDecisions\` but has status ${entry.status}`);
    }
    requireText(errors, `${entry.id}.decision`, entry.decision);
    requireText(errors, `${entry.id}.owner`, entry.owner);
    requireText(errors, `${entry.id}.owningTask`, entry.owningTask);
    requireText(errors, `${entry.id}.reason`, entry.reason);
  }

  for (const question of whitepaper?.questions ?? []) {
    if (!answered.has(question)) {
      errors.push(`decisions: suspended question ${question} has neither a decision nor an open entry`);
    }
  }

  return errors;
}

/** Validates the readable document's tables against the register. */
export function checkDecisionsDocument(register, markdown) {
  const errors = [];
  const tables = parseTables(markdown);
  const dash = '\u2014';

  const compare = (header, expected, label) => {
    const actual = tables.get(header.join('\u0000'));
    if (!actual) {
      errors.push(`decisions document: the ${label} table is missing`);
    } else if (JSON.stringify(actual) !== JSON.stringify(expected)) {
      errors.push(
        `decisions document: the ${label} table is ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`,
      );
    }
  };

  compare(['级别', '含义'], Object.entries(register.statuses ?? {}), 'status-levels');
  compare(
    ['编号', '主题', '状态', '决策'],
    (register.decisions ?? []).map((entry) => [entry.id, entry.topic, entry.status, entry.decision]),
    'decision',
  );
  compare(
    ['编号', '备选', '拒绝理由'],
    (register.decisions ?? []).flatMap((entry) => (entry.alternatives ?? []).map(
      (alternative) => [entry.id, alternative.option, alternative.rejected],
    )),
    'alternative',
  );
  compare(
    ['编号', '兼容影响', '复核触发'],
    (register.decisions ?? []).map((entry) => [entry.id, entry.compatibilityImpact, entry.revisitTrigger]),
    'compatibility',
  );
  compare(
    ['编号', '影响任务'],
    (register.decisions ?? []).map((entry) => [entry.id, (entry.affects ?? []).join(', ')]),
    'task',
  );
  compare(
    ['编号', '待定决策', '状态', '归属', '归属任务', '为什么不能代填'],
    (register.openDecisions ?? []).map((entry) => [
      entry.id, entry.decision, entry.status, entry.owner, entry.owningTask, entry.reason ?? dash,
    ]),
    'open-decision',
  );

  return errors;
}

/** Validates that whitepaper §17.7 still carries the same conclusions as the register. */
export function checkWhitepaperSync(register, markdown) {
  const errors = [];
  const section = register?.whitepaper?.section;
  if (typeof section !== 'string') return ['decisions: whitepaper.section is not set'];

  const text = sectionText(markdown, section);
  if (text === null) return [`decisions: whitepaper has no ${section} section`];

  if (!text.includes('[decisions.md](decisions.md)')) {
    errors.push(`decisions: whitepaper ${section} does not point at decisions.md`);
  }

  const questions = register?.whitepaper?.questions ?? [];
  const table = parseTables(text).get(['问题', '结论', '状态', '详见'].join('\u0000'));
  if (!table) {
    errors.push(`decisions: whitepaper ${section} has no 问题/结论/状态/详见 table`);
  } else {
    for (const id of questions) {
      const entry = (register.decisions ?? []).find((candidate) => candidate.id === id);
      const row = table.find((cells) => cells[0] === `**${id} ${entry?.topic ?? ''}**`);
      if (!row) {
        errors.push(`decisions: whitepaper ${section} has no row for ${id}`);
        continue;
      }
      if (!entry) {
        errors.push(`decisions: ${id} is a whitepaper question with no decision`);
        continue;
      }
      const wanted = [entry.summary, entry.status, '[decisions.md](decisions.md)'];
      if (JSON.stringify(row.slice(1)) !== JSON.stringify(wanted)) {
        errors.push(
          `decisions: whitepaper ${section} ${id} row is ${JSON.stringify(row.slice(1))}, `
          + `expected ${JSON.stringify(wanted)}`,
        );
      }
    }
  }

  for (const entry of (register.decisions ?? [])) {
    if (questions.includes(entry.id)) continue;
    if (!text.includes(`\`${entry.id}\``)) {
      errors.push(`decisions: whitepaper ${section} does not mention the ${entry.id} decision`);
    }
  }

  return errors;
}

/** Runs every half over the repository's files. */
export function checkDecisions(root) {
  const registerFile = path.join(root, DECISIONS_PATH);
  const documentFile = path.join(root, DOCUMENT_PATH);
  for (const file of [registerFile, documentFile]) {
    if (!existsSync(file)) {
      return { errors: [`decisions: missing ${path.relative(root, file)}`], stats: null };
    }
  }

  let register;
  try {
    register = JSON.parse(readFileSync(registerFile, 'utf8'));
  } catch (error) {
    return { errors: [`decisions: ${DECISIONS_PATH} is not valid JSON: ${error.message}`], stats: null };
  }

  const errors = [
    ...checkDecisionsData(register),
    ...checkDecisionsDocument(register, readFileSync(documentFile, 'utf8')),
  ];

  const whitepaperFile = path.join(root, register?.whitepaper?.file ?? '');
  if (typeof register?.whitepaper?.file === 'string' && existsSync(whitepaperFile)) {
    errors.push(...checkWhitepaperSync(register, readFileSync(whitepaperFile, 'utf8')));
  } else {
    errors.push(`decisions: missing ${register?.whitepaper?.file ?? 'whitepaper'}`);
  }

  return {
    errors,
    stats: {
      decisions: (register.decisions ?? []).length,
      questions: (register.whitepaper?.questions ?? []).length,
      open: (register.openDecisions ?? []).length,
      alternatives: (register.decisions ?? []).reduce(
        (total, entry) => total + (entry.alternatives ?? []).length,
        0,
      ),
      statuses: (register.decisions ?? []).reduce((tally, entry) => {
        tally[entry.status] = (tally[entry.status] ?? 0) + 1;
        return tally;
      }, {}),
    },
  };
}

export function describe(stats) {
  const statuses = Object.entries(stats.statuses)
    .map(([status, count]) => `${count} ${status}`)
    .join(', ');
  return `${stats.decisions} decisions (${statuses}), ${stats.questions} suspended questions, `
    + `${stats.alternatives} rejected alternatives, ${stats.open} open owner decisions`;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
  const result = checkDecisions(root);
  if (result.errors.length > 0) {
    console.error(result.errors.join('\n'));
    process.exitCode = 1;
  } else {
    console.log(`Decision register checks passed (${describe(result.stats)}).`);
  }
}
