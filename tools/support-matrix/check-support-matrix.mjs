#!/usr/bin/env node
// SPDX-License-Identifier: MIT
//
// Support-matrix checks: the three hosts, five profiles and fifteen cells of
// `docs/plan/support-matrix.json`, and the tables in `docs/plan/support-matrix.md` that must
// agree with it row for row.
//
// Scope, stated honestly: this proves the matrix is complete (every host x profile pair is
// present exactly once), that every cell carries a level from the fixed vocabulary, that every
// claim above `unverified` names why, that the architecturally excluded mode-1 cells for
// mini/embedded stay excluded, and that the human tables were not edited out of step with the
// data. It does **not** prove that any cell's claim is true — a `certified` cell is a promise to
// have recorded a target build and run, and only evidence review can check that (principle #24).
//
// Usage: node tools/support-matrix/check-support-matrix.mjs

import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const MATRIX_PATH = 'docs/plan/support-matrix.json';
export const DOCUMENT_PATH = 'docs/plan/support-matrix.md';

/** Profiles whose mode-1 exclusion is architectural rather than evidentiary. */
const MODE1_EXCLUDED_PROFILES = ['mini', 'embedded'];

/**
 * Splits a Markdown table row into trimmed cells, or returns null when the line is not a row.
 * Surrounding backticks are stripped so a level may be written as code or as plain text.
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

function isStatus(levels, value) {
  return typeof value === 'string' && Object.hasOwn(levels, value);
}

/** A claim above `unverified` must say why it is not merely unverified. */
function requireReason(errors, where, status, reason) {
  if (status !== 'unverified' && (typeof reason !== 'string' || reason.trim() === '')) {
    errors.push(`support matrix: ${where} claims ${status} without a reason`);
  }
}

/**
 * Validates the matrix data alone. Exported so the vocabulary and completeness rules can be
 * tested without touching the Markdown.
 */
export function checkMatrixData(matrix) {
  const errors = [];
  const levels = matrix?.levels;
  if (!levels || typeof levels !== 'object' || Object.keys(levels).length === 0) {
    return ['support matrix: `levels` must define the status vocabulary'];
  }

  const dependency = matrix.dependency ?? {};
  if (dependency.crate !== 'rust_widgets') {
    errors.push(`support matrix: dependency.crate is ${JSON.stringify(dependency.crate)}`);
  }
  if (!/^[0-9a-f]{40}$/.test(String(dependency.revision))) {
    errors.push(`support matrix: dependency.revision is not a full commit id: ${dependency.revision}`);
  }

  const hosts = Array.isArray(matrix.hosts) ? matrix.hosts : [];
  const profiles = Array.isArray(matrix.profiles) ? matrix.profiles : [];
  if (hosts.length === 0) errors.push('support matrix: no hosts');
  if (profiles.length === 0) errors.push('support matrix: no profiles');

  const hostIds = new Set();
  for (const host of hosts) {
    if (!host.id || !host.triple) {
      errors.push(`support matrix: host ${JSON.stringify(host)} needs an id and a triple`);
      continue;
    }
    if (hostIds.has(host.id)) errors.push(`support matrix: duplicate host ${host.id}`);
    hostIds.add(host.id);
    if (!isStatus(levels, host.designerHost)) {
      errors.push(
        `support matrix: host ${host.id} has unknown designerHost ${JSON.stringify(host.designerHost)}`,
      );
    }
    requireReason(errors, `host ${host.id}`, host.designerHost, host.reason);
  }

  const profileIds = new Set();
  for (const profile of profiles) {
    if (!profile.id || !profile.feature) {
      errors.push(`support matrix: profile ${JSON.stringify(profile)} needs an id and a feature`);
      continue;
    }
    if (profileIds.has(profile.id)) errors.push(`support matrix: duplicate profile ${profile.id}`);
    profileIds.add(profile.id);
    if (!isStatus(levels, profile.mode1)) {
      errors.push(`support matrix: profile ${profile.id} has unknown mode1 ${JSON.stringify(profile.mode1)}`);
    }
    if (!isStatus(levels, profile.mode2)) {
      errors.push(`support matrix: profile ${profile.id} has unknown mode2 ${JSON.stringify(profile.mode2)}`);
    }
    requireReason(errors, `profile ${profile.id} mode 1`, profile.mode1, profile.mode1Reason);
    requireReason(errors, `profile ${profile.id} mode 2`, profile.mode2, profile.mode2Reason);
  }

  if (!profileIds.has(dependency.profile)) {
    errors.push(
      `support matrix: dependency.profile ${JSON.stringify(dependency.profile)} is not one of the named profiles`,
    );
  }

  // The mode-1 exclusion is a decision, not a measurement: a profile that does not carry the  // JSON/CSS runtime must never be recorded as merely unverified, or a later reader could take it
  // for "not measured yet" and start measuring (principle #11).
  for (const id of MODE1_EXCLUDED_PROFILES) {
    const profile = profiles.find((entry) => entry.id === id);
    if (profile && profile.mode1 !== 'unsupported') {
      errors.push(
        `support matrix: profile ${id} mode 1 is ${JSON.stringify(profile.mode1)}; the JSON/CSS `
        + 'runtime is architecturally excluded there (principle #11)',
      );
    }
  }

  const seen = new Set();
  for (const cell of Array.isArray(matrix.cells) ? matrix.cells : []) {
    const key = `${cell.host}\u0000${cell.profile}`;
    if (seen.has(key)) errors.push(`support matrix: duplicate cell ${cell.host} x ${cell.profile}`);
    seen.add(key);
    if (!hostIds.has(cell.host)) errors.push(`support matrix: cell names unknown host ${cell.host}`);
    if (!profileIds.has(cell.profile)) {
      errors.push(`support matrix: cell names unknown profile ${cell.profile}`);
    }
    if (!isStatus(levels, cell.status)) {
      errors.push(
        `support matrix: ${cell.host} x ${cell.profile} has unknown status ${JSON.stringify(cell.status)}`,
      );
    }
    requireReason(errors, `${cell.host} x ${cell.profile}`, cell.status, cell.reason);
  }
  for (const host of hostIds) {
    for (const profile of profileIds) {
      if (!seen.has(`${host}\u0000${profile}`)) {
        errors.push(`support matrix: no cell for ${host} x ${profile}`);
      }
    }
  }
  const expectedCells = hostIds.size * profileIds.size;
  if (seen.size !== expectedCells) {
    errors.push(
      `support matrix: ${seen.size} cells for ${hostIds.size} hosts x ${profileIds.size} profiles`,
    );
  }

  return errors;
}

/** Validates the document's tables against the matrix data. */
export function checkMatrixDocument(matrix, markdown) {
  const errors = [];
  const tables = parseTables(markdown);
  const dash = '\u2014';

  const expectedLevels = Object.keys(matrix.levels).map((level) => [level]);
  const levelTable = tables.get(['级别', '含义'].join('\u0000'));
  if (!levelTable) {
    errors.push('support matrix document: the status-levels table is missing');
  } else {
    const actual = levelTable.map((row) => [row[0]]);
    if (JSON.stringify(actual) !== JSON.stringify(expectedLevels)) {
      errors.push(`support matrix document: the status-levels table lists ${JSON.stringify(actual)}`);
    }
  }

  const expectedHosts = matrix.hosts.map((host) => [
    host.id, host.triple, host.designerHost, host.reason ?? dash,
  ]);
  const hostTable = tables.get(['宿主', 'triple', '设计器宿主', '理由'].join('\u0000'));
  if (!hostTable) {
    errors.push('support matrix document: the host table is missing');
  } else if (JSON.stringify(hostTable) !== JSON.stringify(expectedHosts)) {
    errors.push(
      `support matrix document: the host table is ${JSON.stringify(hostTable)}, `
      + `expected ${JSON.stringify(expectedHosts)}`,
    );
  }

  const expectedProfiles = matrix.profiles.map((profile) => [
    profile.id, profile.feature, profile.mode1, profile.mode2,
  ]);
  const profileTable = tables.get(['profile', 'feature', '模式 1', '模式 2'].join('\u0000'));
  if (!profileTable) {
    errors.push('support matrix document: the profile table is missing');
  } else if (JSON.stringify(profileTable) !== JSON.stringify(expectedProfiles)) {
    errors.push(
      `support matrix document: the profile table is ${JSON.stringify(profileTable)}, `
      + `expected ${JSON.stringify(expectedProfiles)}`,
    );
  }

  const header = ['宿主 \\ profile', ...matrix.profiles.map((profile) => profile.id)];
  const expectedGrid = matrix.hosts.map((host) => [
    host.id,
    ...matrix.profiles.map((profile) => {
      const cell = matrix.cells.find((entry) => entry.host === host.id && entry.profile === profile.id);
      return cell ? cell.status : '(missing)';
    }),
  ]);
  const grid = tables.get(header.join('\u0000'));
  if (!grid) {
    errors.push('support matrix document: the host x profile grid is missing');
  } else if (JSON.stringify(grid) !== JSON.stringify(expectedGrid)) {
    errors.push(
      `support matrix document: the grid is ${JSON.stringify(grid)}, expected ${JSON.stringify(expectedGrid)}`,
    );
  }

  return errors;
}

/** Runs both halves over the repository's files. */
export function checkSupportMatrix(root) {
  const matrixFile = path.join(root, MATRIX_PATH);
  const documentFile = path.join(root, DOCUMENT_PATH);
  for (const file of [matrixFile, documentFile]) {
    if (!existsSync(file)) {
      return { errors: [`support matrix: missing ${path.relative(root, file)}`], stats: null };
    }
  }

  let matrix;
  try {
    matrix = JSON.parse(readFileSync(matrixFile, 'utf8'));
  } catch (error) {
    return { errors: [`support matrix: ${MATRIX_PATH} is not valid JSON: ${error.message}`], stats: null };
  }

  const markdown = readFileSync(documentFile, 'utf8');
  const errors = [...checkMatrixData(matrix), ...checkMatrixDocument(matrix, markdown)];

  const tally = {};
  for (const level of Object.keys(matrix.levels)) tally[level] = 0;
  for (const cell of matrix.cells) tally[cell.status] += 1;

  return {
    errors,
    stats: {
      hosts: matrix.hosts.length,
      profiles: matrix.profiles.length,
      cells: matrix.cells.length,
      tally,
    },
  };
}

export function describe(stats) {
  const levels = Object.entries(stats.tally)
    .filter(([, count]) => count > 0)
    .map(([level, count]) => `${count} ${level}`)
    .join(', ');
  return `${stats.hosts} hosts x ${stats.profiles} profiles = ${stats.cells} cells (${levels})`;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
  const result = checkSupportMatrix(root);
  if (result.errors.length > 0) {
    console.error(result.errors.join('\n'));
    process.exitCode = 1;
  } else {
    console.log(`Support matrix checks passed (${describe(result.stats)}).`);
  }
}
