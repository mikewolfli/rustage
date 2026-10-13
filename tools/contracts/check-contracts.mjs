#!/usr/bin/env node
// SPDX-License-Identifier: MIT
//
// Contract checks for `docs/plan/contracts.json`, `docs/plan/contracts.md` and the code they describe.
//
// Scope, stated honestly. This gate checks four things and refuses to pretend it checks a fifth:
//
//   1. The data file is well formed, every referenced module exists, and every contract area is
//      named in the module it points at.
//   2. The document's tables agree with the data file row for row.
//   3. Whitepaper §12.5 still says what the data file says — same operations, same grants, in the
//      same order — because the permission table is a hand-copied security decision.
//   4. Every invariant names a test that exists in the file the invariant is enforced in, and every
//      mutating entry point in the declared files is declared (`writers`).
//
// What it cannot check: whether the declared classification is right, whether an invariant is
// actually strong, or whether a test asserts enough. A gate can refuse a contradiction; it cannot
// referee a judgement. Those stay with review, and the document says so where a reader will see it.

import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const CONTRACTS_PATH = 'docs/plan/contracts.json';
export const DOCUMENT_PATH = 'docs/plan/contracts.md';

export const SCHEMA = 'rustage.contracts/v1';

/** The kinds of mutating entry point a file may declare. */
const WRITER_KINDS = ['public', 'crate-internal'];

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

/** The same normalisation the table parser applies to a cell, so both sides can be compared. */
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

/** The text of one section, from its heading to the next heading of the same or a higher level. */
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

/**
 * The mutating entry points of a Rust file: `name(&mut self` collected with the visibility in front
 * of it. Deliberately a textual scan — the point is to notice a new write path, and a new write
 * path is spelled `pub fn` or `pub(crate) fn` at the start of a line.
 */
export function mutatingEntryPoints(source) {
  const found = { public: [], 'crate-internal': [] };
  // `\s*` after the opening parenthesis on purpose: rustfmt wraps long signatures, and a checker
  // that stops seeing a write path because it was reformatted is not a checker.
  for (const match of source.matchAll(/^\s*(pub\(crate\)|pub) fn (\w+)\(\s*&mut self/gm)) {
    const kind = match[1] === 'pub' ? 'public' : 'crate-internal';
    found[kind].push(match[2]);
  }
  for (const kind of WRITER_KINDS) found[kind].sort();
  return found;
}

/** Public fields would be a way around every declared entry point, so their presence is refused. */
export function publicFields(source) {
  return [...source.matchAll(/^\s+pub [a-z_][a-z0-9_]*: /gm)].map((match) => match[0].trim());
}

function requireText(errors, where, value) {
  if (typeof value !== 'string' || value.trim() === '') {
    errors.push(`contracts: ${where} must be a non-empty string`);
  }
}

/** Validates the data file, the modules it names and the code it describes. */
export function checkContractsData(contracts, root) {
  const errors = [];
  if (!contracts || typeof contracts !== 'object') return ['contracts: the data file is not an object'];
  if (contracts.schema !== SCHEMA) {
    errors.push(`contracts: schema is ${JSON.stringify(contracts.schema)}, expected ${SCHEMA}`);
  }

  const whitepaper = contracts.whitepaper;
  if (!whitepaper || typeof whitepaper !== 'object') {
    errors.push('contracts: `whitepaper` must name the file, the section and the table header');
  } else {
    requireText(errors, 'whitepaper.file', whitepaper.file);
    requireText(errors, 'whitepaper.section', whitepaper.section);
    if (!Array.isArray(whitepaper.table) || whitepaper.table.length !== 3) {
      errors.push('contracts: whitepaper.table must be the three header cells of the §12.5 table');
    }
  }

  for (const [field, vocabulary] of [['grants', contracts.grants], ['denials', contracts.denials]]) {
    if (!vocabulary || typeof vocabulary !== 'object' || Object.keys(vocabulary).length === 0) {
      errors.push(`contracts: \`${field}\` must define the vocabulary`);
      continue;
    }
    for (const [name, meaning] of Object.entries(vocabulary)) {
      requireText(errors, `${field}.${name}`, meaning);
    }
  }

  const areas = contracts.areas;
  const areaIds = new Set();
  if (!Array.isArray(areas) || areas.length === 0) {
    errors.push('contracts: `areas` must be a non-empty list');
  } else {
    for (const [index, area] of areas.entries()) {
      const where = `areas[${index}]`;
      for (const field of ['id', 'concern', 'module', 'principle']) {
        requireText(errors, `${where}.${field}`, area?.[field]);
      }
      if (typeof area?.id === 'string') {
        if (areaIds.has(area.id)) errors.push(`contracts: duplicate area ${area.id}`);
        areaIds.add(area.id);
      }
      if (typeof area?.module !== 'string') continue;
      const modulePath = path.join(root, area.module);
      if (!existsSync(modulePath)) {
        errors.push(`contracts: ${where}.module does not exist: ${area.module}`);
        continue;
      }
      const source = readFileSync(modulePath, 'utf8');
      if (!source.includes(area.id)) {
        errors.push(`contracts: ${area.module} never names its contract ${area.id}`);
      }
    }
  }

  const permissions = contracts.permissions;
  const seen = new Set();
  if (!Array.isArray(permissions) || permissions.length === 0) {
    errors.push('contracts: `permissions` must be a non-empty list');
  } else {
    for (const [index, permission] of permissions.entries()) {
      const where = `permissions[${index}]`;
      for (const field of ['operation', 'operationWording', 'grant', 'grantWording', 'reason', 'test']) {
        requireText(errors, `${where}.${field}`, permission?.[field]);
      }
      if (typeof permission?.operation === 'string') {
        if (!/^[a-z][a-z-]*$/.test(permission.operation)) {
          errors.push(`contracts: ${where}.operation is not a machine name: ${permission.operation}`);
        }
        if (seen.has(permission.operation)) errors.push(`contracts: duplicate operation ${permission.operation}`);
        seen.add(permission.operation);
      }
      if (!Object.hasOwn(contracts.grants ?? {}, permission?.grant)) {
        errors.push(`contracts: ${permission?.operation} uses an unknown grant ${JSON.stringify(permission?.grant)}`);
      }
      if (permission?.deniedAs !== null && !Object.hasOwn(contracts.denials ?? {}, permission?.deniedAs)) {
        errors.push(
          `contracts: ${permission?.operation} uses an unknown denial ${JSON.stringify(permission?.deniedAs)}`,
        );
      }
    }
  }

  const writers = contracts.writers;
  const declaredWriters = new Map();
  if (!Array.isArray(writers) || writers.length === 0) {
    errors.push('contracts: `writers` must declare every mutating entry point');
  } else {
    for (const [index, writer] of writers.entries()) {
      const where = `writers[${index}]`;
      requireText(errors, `${where}.file`, writer?.file);
      requireText(errors, `${where}.kind`, writer?.kind);
      requireText(errors, `${where}.reason`, writer?.reason);
      if (!WRITER_KINDS.includes(writer?.kind)) {
        errors.push(`contracts: ${where}.kind is ${JSON.stringify(writer?.kind)}, not one of ${WRITER_KINDS}`);
      }
      if (!Array.isArray(writer?.names) || writer.names.length === 0) {
        errors.push(`contracts: ${where}.names must list the methods`);
        continue;
      }
      if (typeof writer?.file !== 'string') continue;
      const filePath = path.join(root, writer.file);
      if (!existsSync(filePath)) {
        errors.push(`contracts: ${where}.file does not exist: ${writer.file}`);
        continue;
      }
      const source = readFileSync(filePath, 'utf8');
      const found = mutatingEntryPoints(source)[writer.kind].slice().sort();
      const declared = [...writer.names].sort();
      if (JSON.stringify(found) !== JSON.stringify(declared)) {
        errors.push(
          `contracts: ${writer.file} has ${writer.kind} mutating entry points ${JSON.stringify(found)}, `
          + `declared ${JSON.stringify(declared)}`,
        );
      }
      const fields = publicFields(source);
      if (fields.length > 0) {
        errors.push(
          `contracts: ${writer.file} exposes public fields (${fields.join(', ')}); `
          + 'a public field bypasses every declared entry point',
        );
      }
    }
  }

  const invariants = contracts.invariants;
  const seenInvariants = new Set();
  if (!Array.isArray(invariants) || invariants.length === 0) {
    errors.push('contracts: `invariants` must be a non-empty list');
  } else {
    for (const [index, invariant] of invariants.entries()) {
      const where = `invariants[${index}]`;
      for (const field of ['id', 'statement', 'area', 'enforcedBy', 'test', 'evidence', 'note']) {
        requireText(errors, `${where}.${field}`, invariant?.[field]);
      }
      if (typeof invariant?.id === 'string') {
        if (seenInvariants.has(invariant.id)) errors.push(`contracts: duplicate invariant ${invariant.id}`);
        seenInvariants.add(invariant.id);
      }
      if (typeof invariant?.area === 'string' && !areaIds.has(invariant.area)) {
        errors.push(`contracts: ${invariant.id} names an unknown area ${invariant.area}`);
      }
      if (typeof invariant?.enforcedBy !== 'string') continue;
      const filePath = path.join(root, invariant.enforcedBy);
      if (!existsSync(filePath)) {
        errors.push(`contracts: ${invariant.id} is enforced in a file that does not exist: ${invariant.enforcedBy}`);
        continue;
      }
      const source = readFileSync(filePath, 'utf8');
      if (!source.includes(`fn ${invariant.test}(`)) {
        errors.push(`contracts: ${invariant.enforcedBy} has no test named ${invariant.test}`);
      }
    }
  }

  return errors;
}

/** Validates that whitepaper §12.5 still states the same permission table. */
export function checkWhitepaperSync(contracts, markdown) {
  const section = contracts?.whitepaper?.section;
  if (typeof section !== 'string') return ['contracts: whitepaper.section is not set'];

  const text = sectionText(markdown, section);
  if (text === null) return [`contracts: the whitepaper has no ${section} section`];

  const errors = [];
  const header = contracts.whitepaper.table.join('\u0000');
  const table = parseTables(text).get(header);
  if (!table) {
    errors.push(`contracts: whitepaper ${section} has no ${contracts.whitepaper.table.join('/')} table`);
    return errors;
  }

  // Normalised the same way the table parser normalises its cells: a cell that ends with a code
  // span would otherwise compare unequal to itself.
  const expected = (contracts.permissions ?? []).map((permission) => [
    permission.operationWording, permission.grantWording, permission.reason,
  ].map(normaliseCell));
  if (JSON.stringify(table) !== JSON.stringify(expected)) {
    errors.push(
      `contracts: whitepaper ${section} is ${JSON.stringify(table)}, expected ${JSON.stringify(expected)}`,
    );
  }
  return errors;
}

/** Validates the readable document's tables against the data file. */
export function checkContractsDocument(contracts, markdown) {
  const errors = [];
  const tables = parseTables(markdown);

  const compare = (header, expected, label) => {
    const actual = tables.get(header.join('\u0000'));
    // Both sides go through the same normalisation: the table parser strips surrounding backticks,
    // so the expected cells have to be stripped the same way or every code span reads as a mismatch.
    const wanted = expected.map((cells) => cells.map(normaliseCell));
    if (!actual) {
      errors.push(`contracts document: the ${label} table is missing`);
    } else if (JSON.stringify(actual) !== JSON.stringify(wanted)) {
      errors.push(
        `contracts document: the ${label} table is ${JSON.stringify(actual)}, expected ${JSON.stringify(wanted)}`,
      );
    }
  };

  compare(
    ['契约', '关注点', '模块', '依据'],
    (contracts.areas ?? []).map((area) => [
      area.id, area.concern, `[\`${area.module}\`](../../${area.module})`, area.principle,
    ]),
    'area',
  );
  compare(['授权', '含义'], Object.entries(contracts.grants ?? {}), 'grant');
  compare(['拒绝原因', '含义'], Object.entries(contracts.denials ?? {}), 'denial');
  compare(
    ['操作类别', '机器名', '默认权限', '原文写法', '理由', '拒绝时', '用例'],
    (contracts.permissions ?? []).map((permission) => [
      permission.operationWording,
      `\`${permission.operation}\``,
      `\`${permission.grant}\``,
      permission.grantWording,
      permission.reason,
      permission.deniedAs === null ? '\u2014' : `\`${permission.deniedAs}\``,
      `\`${permission.test}\``,
    ]),
    'permission',
  );
  compare(
    ['文件', '种类', '写入方法', '理由'],
    (contracts.writers ?? []).map((writer) => [
      `[\`${writer.file}\`](../../${writer.file})`,
      writer.kind,
      writer.names.map((name) => `\`${name}\``).join(', '),
      writer.reason,
    ]),
    'writer',
  );
  compare(
    ['编号', '不变量', '契约', '强制位置', '用例', '证据'],
    (contracts.invariants ?? []).map((invariant) => [
      invariant.id,
      invariant.statement,
      `\`${invariant.area}\``,
      `[\`${invariant.enforcedBy}\`](../../${invariant.enforcedBy})`,
      `\`${invariant.test}\``,
      invariant.evidence,
    ]),
    'invariant',
  );
  compare(
    ['编号', '说明'],
    (contracts.invariants ?? []).map((invariant) => [invariant.id, invariant.note]),
    'invariant note',
  );

  return errors;
}

/** Runs every half over the repository's files. */
export function checkContracts(root) {
  const dataFile = path.join(root, CONTRACTS_PATH);
  const documentFile = path.join(root, DOCUMENT_PATH);
  for (const file of [dataFile, documentFile]) {
    if (!existsSync(file)) {
      return { errors: [`contracts: missing ${path.relative(root, file)}`], stats: null };
    }
  }

  let contracts;
  try {
    contracts = JSON.parse(readFileSync(dataFile, 'utf8'));
  } catch (error) {
    return { errors: [`contracts: ${CONTRACTS_PATH} is not valid JSON: ${error.message}`], stats: null };
  }

  const errors = [
    ...checkContractsData(contracts, root),
    ...checkContractsDocument(contracts, readFileSync(documentFile, 'utf8')),
  ];

  const whitepaperFile = path.join(root, contracts?.whitepaper?.file ?? '');
  if (typeof contracts?.whitepaper?.file === 'string' && existsSync(whitepaperFile)) {
    errors.push(...checkWhitepaperSync(contracts, readFileSync(whitepaperFile, 'utf8')));
  } else {
    errors.push(`contracts: missing ${contracts?.whitepaper?.file ?? 'whitepaper'}`);
  }

  return {
    errors,
    stats: {
      areas: (contracts.areas ?? []).length,
      permissions: (contracts.permissions ?? []).length,
      invariants: (contracts.invariants ?? []).length,
      writers: (contracts.writers ?? []).reduce((total, writer) => total + writer.names.length, 0),
      denials: Object.keys(contracts.denials ?? {}).length,
    },
  };
}

export function describe(stats) {
  return `${stats.areas} contract areas, ${stats.permissions} permission rows, `
    + `${stats.invariants} invariants, ${stats.writers} declared write entry points, `
    + `${stats.denials} denial reasons`;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
  const result = checkContracts(root);
  if (result.errors.length > 0) {
    console.error(result.errors.join('\n'));
    process.exitCode = 1;
  } else {
    console.log(`Contract checks passed (${describe(result.stats)}).`);
  }
}
