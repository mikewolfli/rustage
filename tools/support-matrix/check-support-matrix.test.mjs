// SPDX-License-Identifier: MIT
//
// Negative and positive cases for the support-matrix checks. Each rule gets a case here because a
// gate that cannot refuse anything is not a gate: an incomplete matrix, an unknown level, a claim
// without a reason, an exclusion quietly downgraded, and a document edited out of step with the
// data must all fail.

import assert from 'node:assert/strict';
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { after, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  checkMatrixData, checkMatrixDocument, checkSupportMatrix, describe,
  DOCUMENT_PATH, MATRIX_PATH,
} from './check-support-matrix.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const temporaryDirectories = [];

function fixture(mutateMatrix, mutateDocument) {
  const directory = mkdtempSync(path.join(tmpdir(), 'rustage-support-matrix-'));
  temporaryDirectories.push(directory);
  for (const file of [MATRIX_PATH, DOCUMENT_PATH]) {
    cpSync(path.join(root, file), path.join(directory, file));
  }
  const matrixFile = path.join(directory, MATRIX_PATH);
  const documentFile = path.join(directory, DOCUMENT_PATH);
  if (mutateMatrix) {
    const matrix = JSON.parse(readFileSync(matrixFile, 'utf8'));
    mutateMatrix(matrix);
    writeFileSync(matrixFile, `${JSON.stringify(matrix, null, 2)}\n`);
  }
  if (mutateDocument) {
    writeFileSync(documentFile, mutateDocument(readFileSync(documentFile, 'utf8')));
  }
  return directory;
}

function messages(directory) {
  return checkSupportMatrix(directory).errors.join('\n');
}

after(() => {
  for (const directory of temporaryDirectories) rmSync(directory, { recursive: true, force: true });
});

test('the committed matrix and document pass', () => {
  const result = checkSupportMatrix(root);
  assert.deepEqual(result.errors, []);
  assert.equal(result.stats.cells, result.stats.hosts * result.stats.profiles);
});

test('describe names the cells and the non-empty levels', () => {
  assert.equal(
    describe({ hosts: 3, profiles: 5, cells: 15, tally: { unsupported: 0, unverified: 15 } }),
    '3 hosts x 5 profiles = 15 cells (15 unverified)',
  );
});

test('an unknown cell status is refused', () => {
  const directory = fixture((matrix) => {
    matrix.cells[0].status = 'probably-fine';
  });
  assert.match(messages(directory), /unknown status "probably-fine"/);
});

test('a missing cell is refused', () => {
  const directory = fixture((matrix) => {
    matrix.cells.pop();
  });
  assert.match(messages(directory), /no cell for linux x embedded/);
  assert.match(messages(directory), /4 hosts x 5 profiles|14 cells for/);
});

test('a duplicate cell is refused', () => {
  const directory = fixture((matrix) => {
    matrix.cells.push({ ...matrix.cells[0] });
  });
  assert.match(messages(directory), /duplicate cell macos x desktop/);
});

test('a claim above unverified must carry a reason', () => {
  const directory = fixture((matrix) => {
    matrix.hosts[1].designerHost = 'certified';
    matrix.hosts[1].reason = null;
  });
  assert.match(messages(directory), /host windows claims certified without a reason/);
});

test('the architectural mode-1 exclusion cannot be downgraded to unverified', () => {
  const directory = fixture((matrix) => {
    matrix.profiles.find((profile) => profile.id === 'mini').mode1 = 'unverified';
    matrix.profiles.find((profile) => profile.id === 'mini').mode1Reason = null;
  });
  assert.match(messages(directory), /mini mode 1 is "unverified"; the JSON\/CSS runtime/);
});

test('a truncated dependency revision is refused', () => {
  const directory = fixture((matrix) => {
    matrix.dependency.revision = 'c957fdec';
  });
  assert.match(messages(directory), /is not a full commit id/);
});

test('a document edited out of step with the data is refused', () => {
  const directory = fixture(null, (markdown) => markdown.replace(
    '| macos | unverified | unverified | unverified | unverified | unverified |',
    '| macos | certified | unverified | unverified | unverified | unverified |',
  ));
  assert.match(messages(directory), /the grid is/);
});

test('a missing document table is refused', () => {
  const directory = fixture(null, (markdown) => markdown
    .split('\n')
    .filter((line) => !line.startsWith('| profile |'))
    .filter((line) => !/^\| (desktop|tablet|mobile|mini|embedded) \| [a-z]+ \| /.test(line))
    .join('\n'));
  assert.match(messages(directory), /the profile table is missing/);
});

test('validating data and document separately reports the same shape of errors', () => {
  const matrix = JSON.parse(readFileSync(path.join(root, MATRIX_PATH), 'utf8'));
  const document = readFileSync(path.join(root, DOCUMENT_PATH), 'utf8');
  assert.deepEqual(checkMatrixData(matrix), []);
  assert.deepEqual(checkMatrixDocument(matrix, document), []);
});
