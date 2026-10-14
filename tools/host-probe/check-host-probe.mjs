#!/usr/bin/env node
// SPDX-License-Identifier: MIT
//
// Host-probe record checks: the three desktop hosts, the capability groups and the cells of
// `docs/plan/host-probe.json`, the tables in `docs/plan/host-probe.md` that must agree with it, and
// the probe checks the record claims to cover.
//
// Scope, stated honestly: this proves the record is *complete and internally consistent* — every
// host x group pair is present exactly once, every cell carries a status from the fixed vocabulary,
// a `probed` cell names its outcome and its evidence and belongs to the host the record was produced
// on, a `blocked` cell names what is missing, the capability names the record groups come from the
// probe's own source and are covered exactly once, and the human tables were not edited out of step
// with the data. It does **not** prove that any cell's claim is true: only a run of the probe on that
// host can do that (principle #24), and this checker cannot run a macOS or Windows machine.
//
// Usage: node tools/host-probe/check-host-probe.mjs

import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const RECORD_PATH = 'docs/plan/host-probe.json';
export const DOCUMENT_PATH = 'docs/plan/host-probe.md';

/** The three desktop hosts T01.08 asks about, in the order the record lists them. */
export const DESKTOP_HOSTS = ['macos', 'windows', 'linux'];

/** Splits a Markdown table row into trimmed cells, or returns null when the line is not a row. */
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
 * The capability names the probe really reports.
 *
 * Read from the probe's own source rather than copied into the data file, because the whole point of
 * the record is that a check cannot be added, renamed or removed without the record following. The
 * names live in one `match self` inside `Capability::name`, so the block is taken as a whole and its
 * string arms are the vocabulary.
 */
export function probeCheckNames(source) {
  const marker = "pub const fn name(self) -> &'static str {";
  const start = source.indexOf(marker);
  if (start === -1) return null;
  const end = source.indexOf('\n    }', start);
  if (end === -1) return null;
  const block = source.slice(start, end);
  return [...block.matchAll(/=>\s*"([^"]+)"/g)].map((match) => match[1]);
}

function isKnown(vocabulary, value) {
  return typeof value === 'string' && Object.hasOwn(vocabulary, value);
}

/** Validates the record data alone, against the probe's own check names. */
export function checkHostProbeData(record, checkNames) {
  const errors = [];
  if (checkNames === null) {
    return ['host probe: the probe source has no `Capability::name` block to read'];
  }
  const statuses = record?.statuses;
  const outcomes = record?.outcomes;
  if (!statuses || Object.keys(statuses).length === 0) {
    return ['host probe: `statuses` must define the status vocabulary'];
  }
  if (!outcomes || Object.keys(outcomes).length === 0) {
    return ['host probe: `outcomes` must define the outcome vocabulary'];
  }

  const groups = Array.isArray(record.groups) ? record.groups : [];
  if (groups.length === 0) errors.push('host probe: no capability groups');

  const known = new Set(checkNames);
  const covered = new Map();
  const groupIds = new Set();
  for (const group of groups) {
    if (!group.id || !group.wording) {
      errors.push(`host probe: group ${JSON.stringify(group)} needs an id and a wording`);
      continue;
    }
    if (groupIds.has(group.id)) errors.push(`host probe: duplicate group ${group.id}`);
    groupIds.add(group.id);
    const checks = Array.isArray(group.checks) ? group.checks : [];
    if (checks.length === 0) errors.push(`host probe: group ${group.id} covers no check`);
    for (const name of checks) {
      if (!known.has(name)) {
        errors.push(
          `host probe: group ${group.id} names ${JSON.stringify(name)}, which the probe does not report`,
        );
      }
      if (covered.has(name)) {
        errors.push(`host probe: ${name} is covered by ${covered.get(name)} and by ${group.id}`);
      }
      covered.set(name, group.id);
    }
  }
  for (const name of checkNames) {
    if (!covered.has(name)) {
      errors.push(`host probe: the probe reports ${name}, which no group covers`);
    }
  }

  const hosts = Array.isArray(record.hosts) ? record.hosts : [];
  if (hosts.length === 0) errors.push('host probe: no hosts');
  const hostIds = new Set();
  let probedHosts = 0;
  for (const host of hosts) {
    if (!host.id || !host.triple) {
      errors.push(`host probe: host ${JSON.stringify(host)} needs an id and a triple`);
      continue;
    }
    if (hostIds.has(host.id)) errors.push(`host probe: duplicate host ${host.id}`);
    hostIds.add(host.id);
    if (typeof host.probedOn !== 'boolean') {
      errors.push(`host probe: host ${host.id} needs a probedOn flag`);
    }
    if (host.probedOn) {
      probedHosts += 1;
      if (typeof host.environment !== 'string' || host.environment.trim() === '') {
        errors.push(`host probe: host ${host.id} was probed but names no environment`);
      }
    } else if (typeof host.missing !== 'string' || host.missing.trim() === '') {
      errors.push(`host probe: host ${host.id} was not probed and does not say what is missing`);
    }
  }
  if (probedHosts !== 1) {
    // One host is the machine the record was produced on. Two would mean the record mixes machines
    // without saying which result came from which, and zero would mean nothing was measured at all.
    errors.push(`host probe: ${probedHosts} hosts are marked as probed; exactly one may be`);
  }
  for (const id of DESKTOP_HOSTS) {
    if (!hostIds.has(id)) errors.push(`host probe: the record has no ${id} row`);
  }

  const seen = new Set();
  for (const result of Array.isArray(record.results) ? record.results : []) {
    const key = `${result.host}\u0000${result.group}`;
    if (seen.has(key)) errors.push(`host probe: duplicate cell ${result.host} x ${result.group}`);
    seen.add(key);
    if (!hostIds.has(result.host)) errors.push(`host probe: cell names unknown host ${result.host}`);
    if (!groupIds.has(result.group)) {
      errors.push(`host probe: cell names unknown group ${result.group}`);
    }
    if (!isKnown(statuses, result.status)) {
      errors.push(
        `host probe: ${result.host} x ${result.group} has unknown status ${JSON.stringify(result.status)}`,
      );
      continue;
    }
    const host = hosts.find((entry) => entry.id === result.host);
    if (result.status === 'probed') {
      if (!host?.probedOn) {
        errors.push(
          `host probe: ${result.host} x ${result.group} claims probed on a host that was not probed`,
        );
      }
      if (!isKnown(outcomes, result.outcome)) {
        errors.push(
          `host probe: ${result.host} x ${result.group} claims probed without a known outcome `
          + `(${JSON.stringify(result.outcome)})`,
        );
      }
      if (typeof result.evidence !== 'string' || result.evidence.trim() === '') {
        errors.push(`host probe: ${result.host} x ${result.group} claims probed without evidence`);
      }
    } else if (result.status === 'blocked' && typeof result.reason !== 'string' && !host?.missing) {
      errors.push(
        `host probe: ${result.host} x ${result.group} is blocked without saying what is missing`,
      );
    }
    if (result.status === 'unsupported' && typeof result.reason !== 'string') {
      errors.push(`host probe: ${result.host} x ${result.group} is unsupported without a decision`);
    }
  }
  for (const host of hostIds) {
    for (const group of groupIds) {
      if (!seen.has(`${host}\u0000${group}`)) {
        errors.push(`host probe: no cell for ${host} x ${group}`);
      }
    }
  }
  const expected = hostIds.size * groupIds.size;
  if (seen.size !== expected) {
    errors.push(`host probe: ${seen.size} cells for ${hostIds.size} hosts x ${groupIds.size} groups`);
  }

  return errors;
}

/** The table rows the document must contain, derived from the record. */
export function documentTables(record) {
  const dash = '\u2014';
  const cellText = (result) => {
    if (!result) return '(missing)';
    if (result.status === 'probed') return `probed: ${result.outcome}`;
    return result.status;
  };
  const find = (host, group) =>
    record.results.find((entry) => entry.host === host && entry.group === group);
  return {
    statuses: Object.entries(record.statuses),
    outcomes: Object.entries(record.outcomes),
    groups: record.groups.map((group) => [group.id, group.wording, group.checks.join(', ')]),
    hosts: record.hosts.map((host) => [
      host.id,
      host.os,
      host.arch,
      host.triple,
      host.backend ?? dash,
      host.probedOn ? 'probed' : 'blocked',
      host.missing ?? dash,
    ]),
    grid: record.hosts.map((host) => [
      host.id,
      ...record.groups.map((group) => cellText(find(host.id, group.id))),
    ]),
    evidence: record.results
      .filter((result) => result.status === 'probed')
      .map((result) => [result.host, result.group, result.outcome, result.evidence]),
  };
}

/** Validates the document's tables against the record. */
export function checkHostProbeDocument(record, markdown) {
  const errors = [];
  const tables = parseTables(markdown);
  const expected = documentTables(record);
  const compare = (header, rows, label) => {
    const actual = tables.get(header.join('\u0000'));
    if (!actual) {
      errors.push(`host probe document: the ${label} table is missing`);
    } else if (JSON.stringify(actual) !== JSON.stringify(rows)) {
      errors.push(
        `host probe document: the ${label} table is ${JSON.stringify(actual)}, `
        + `expected ${JSON.stringify(rows)}`,
      );
    }
  };
  compare(['状态', '含义'], expected.statuses, 'status');
  compare(['结果', '含义'], expected.outcomes, 'outcome');
  compare(['组', '说明', '覆盖的检查'], expected.groups, 'group');
  compare(['宿主', 'OS', '架构', 'triple', '后端', '本机', '缺什么'], expected.hosts, 'host');
  compare(['宿主 \\ 组', ...record.groups.map((group) => group.id)], expected.grid, 'grid');
  compare(['宿主', '组', '结果', '证据'], expected.evidence, 'evidence');
  return errors;
}

/** Runs every half over the repository's files. */
export function checkHostProbe(root) {
  const recordFile = path.join(root, RECORD_PATH);
  const documentFile = path.join(root, DOCUMENT_PATH);
  for (const file of [recordFile, documentFile]) {
    if (!existsSync(file)) {
      return { errors: [`host probe: missing ${path.relative(root, file)}`], stats: null };
    }
  }

  let record;
  try {
    record = JSON.parse(readFileSync(recordFile, 'utf8'));
  } catch (error) {
    return {
      errors: [`host probe: ${RECORD_PATH} is not valid JSON: ${error.message}`],
      stats: null,
    };
  }

  const sourceFile = path.join(root, record.checkSource ?? '');
  if (!existsSync(sourceFile)) {
    return { errors: [`host probe: checkSource ${record.checkSource} does not exist`], stats: null };
  }
  const checkNames = probeCheckNames(readFileSync(sourceFile, 'utf8'));

  const errors = [
    ...checkHostProbeData(record, checkNames),
    ...checkHostProbeDocument(record, readFileSync(documentFile, 'utf8')),
  ];

  const tally = {};
  for (const status of Object.keys(record.statuses ?? {})) tally[status] = 0;
  for (const result of record.results ?? []) {
    if (Object.hasOwn(tally, result.status)) tally[result.status] += 1;
  }

  return {
    errors,
    stats: {
      hosts: (record.hosts ?? []).length,
      groups: (record.groups ?? []).length,
      checks: (record.groups ?? []).reduce(
        (total, group) => total + (group.checks?.length ?? 0),
        0,
      ),
      cells: (record.results ?? []).length,
      tally,
    },
  };
}

export function describe(stats) {
  const tally = Object.entries(stats.tally)
    .filter(([, count]) => count > 0)
    .map(([status, count]) => `${count} ${status}`)
    .join(', ');
  return `${stats.hosts} hosts x ${stats.groups} groups = ${stats.cells} cells over `
    + `${stats.checks} probe checks (${tally})`;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
  const result = checkHostProbe(root);
  if (result.errors.length > 0) {
    console.error(result.errors.join('\n'));
    process.exitCode = 1;
  } else {
    console.log(`Host probe record checks passed (${describe(result.stats)}).`);
  }
}
