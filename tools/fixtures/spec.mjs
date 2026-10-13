// SPDX-License-Identifier: MIT
//
// Reading and validating `fixtures/spec.json`.
//
// Kept in its own file so the generator and the checker share one reading of the spec: two readers
// would be two chances to disagree about what a fixed size means.

import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';

export const SPEC_PATH = 'fixtures/spec.json';
export const SCHEMA = 'rustage.fixtures/v1';

/** Which measurement scenarios a fixture may claim to support; the benchmark document owns the list. */
const CHECK_MODES = ['pass', 'fail', 'not-run'];

function requireText(errors, where, value) {
  if (typeof value !== 'string' || value.trim() === '') {
    errors.push(`fixtures: ${where} must be a non-empty string`);
  }
}

/** Reads the spec. Throws when it is missing or not JSON, because a silent default would hide it. */
export function readSpec(file) {
  if (!existsSync(file)) throw new Error(`missing ${file}`);
  return JSON.parse(readFileSync(file, 'utf8'));
}

/**
 * Validates the spec alone: fixed sizes present, fixtures complete, no ambiguity about what a
 * fixture contains or what it expects. Exported so the rules can be tested without touching disk.
 */
export function checkSpec(spec, root) {
  const errors = [];
  if (!spec || typeof spec !== 'object') return ['fixtures: the spec is not an object'];
  if (spec.schema !== SCHEMA) {
    errors.push(`fixtures: schema is ${JSON.stringify(spec.schema)}, expected ${SCHEMA}`);
  }

  const sizes = spec.sizes;
  if (!sizes || typeof sizes !== 'object') {
    errors.push('fixtures: `sizes` must fix the data sizes');
  } else {
    for (const [name, size] of Object.entries(sizes)) {
      if (!Number.isInteger(size) || size <= 0) {
        errors.push(`fixtures: sizes.${name} is not a fixed positive integer: ${JSON.stringify(size)}`);
      }
    }
  }

  const list = spec.fixtures;
  if (!Array.isArray(list) || list.length === 0) {
    errors.push('fixtures: `fixtures` must be a non-empty list');
    return errors;
  }

  const ids = new Set();
  for (const [index, fixture] of list.entries()) {
    const where = `fixtures[${index}]`;
    for (const field of ['id', 'purpose']) requireText(errors, `${where}.${field}`, fixture?.[field]);
    if (typeof fixture?.id === 'string') {
      if (ids.has(fixture.id)) errors.push(`fixtures: duplicate id ${fixture.id}`);
      ids.add(fixture.id);
    }
    if (!Array.isArray(fixture?.covers) || fixture.covers.length === 0) {
      errors.push(`fixtures: ${fixture?.id} states nothing it covers`);
    }
    if (!Array.isArray(fixture?.files) || fixture.files.length === 0) {
      errors.push(`fixtures: ${fixture?.id} lists no files`);
    }
    if (!Array.isArray(fixture?.measurement)) {
      errors.push(`fixtures: ${fixture?.id}.measurement must be a list, possibly empty`);
    }
    // A fixture with neither a scenario nor a journey behind it is data nobody asked for.
    if (!Array.isArray(fixture?.journeys) || fixture.journeys.length === 0) {
      errors.push(`fixtures: ${fixture?.id} serves no journey in roadmap section 34`);
    }
    const expect = fixture?.expect;
    if (!expect || typeof expect !== 'object') {
      errors.push(`fixtures: ${fixture?.id} must state what it expects`);
      continue;
    }
    if (typeof expect.projectParses !== 'boolean') {
      errors.push(`fixtures: ${fixture?.id}.expect.projectParses must be true or false`);
    }
    if (!CHECK_MODES.includes(expect.cargoCheck)) {
      errors.push(`fixtures: ${fixture?.id}.expect.cargoCheck is ${JSON.stringify(expect.cargoCheck)}`);
    }
    requireText(errors, `${fixture?.id}.expect.notes`, expect.notes);
  }

  const determinism = spec.determinism;
  if (!determinism || typeof determinism !== 'object') {
    errors.push('fixtures: `determinism` must say how reproducibility is shown');
  } else {
    requireText(errors, 'determinism.method', determinism.method);
    requireText(errors, 'determinism.note', determinism.note);
  }

  if (root) {
    for (const file of [SPEC_PATH]) {
      if (!existsSync(path.join(root, file))) errors.push(`fixtures: missing ${file}`);
    }
  }

  return errors;
}
