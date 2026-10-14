// SPDX-License-Identifier: MIT
//
// Negative and positive cases for the host-probe record checks. Each rule gets a case here because a
// gate that cannot refuse anything is not a gate: an incomplete record, an unknown status, a claim
// of `probed` on a machine nobody ran on, a group that forgets a check the probe reports, a
// `blocked` host that does not say what is missing, and a document edited out of step with the data
// must all fail.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  checkHostProbe,
  checkHostProbeData,
  checkHostProbeDocument,
  describe,
  DOCUMENT_PATH,
  probeCheckNames,
  RECORD_PATH,
} from './check-host-probe.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');

/** The probe's own check names, read from the source exactly as the checker does. */
const checkNames = probeCheckNames(readFileSync(path.join(root, 'src/host/readiness.rs'), 'utf8'));

function committedRecord() {
  return JSON.parse(readFileSync(path.join(root, RECORD_PATH), 'utf8'));
}

test('the committed record, document and probe source pass', () => {
  const result = checkHostProbe(root);

  assert.deepEqual(result.errors, []);
  assert.equal(result.stats.hosts, 3);
  assert.equal(result.stats.cells, result.stats.hosts * result.stats.groups);
  assert.equal(result.stats.checks, checkNames.length);
  assert.ok(result.stats.tally.probed > 0, 'the record has to contain a real measurement');
  assert.ok(result.stats.tally.blocked > 0, 'the record has to say which hosts are missing');
});

test('describe names what was checked', () => {
  const line = describe({ hosts: 3, groups: 6, checks: 14, cells: 18, tally: { probed: 6, blocked: 12 } });

  assert.match(line, /3 hosts x 6 groups/);
  assert.match(line, /6 probed/);
});

test('every check the probe reports is covered by exactly one group', () => {
  const record = committedRecord();
  assert.deepEqual(checkHostProbeData(record, checkNames), []);

  // The rule the whole record rests on: a probe that gains a check cannot leave the record behind.
  const grown = checkHostProbeData(record, [...checkNames, 'a-check-nobody-recorded']);
  assert.ok(
    grown.some((error) => error.includes('which no group covers')),
    grown.join('\n'),
  );

  // And the other direction: a record naming a check the probe does not report.
  const invented = structuredClone(record);
  invented.groups[0].checks = [...invented.groups[0].checks, 'a-check-that-does-not-exist'];
  const errors = checkHostProbeData(invented, checkNames);
  assert.ok(
    errors.some((error) => error.includes('which the probe does not report')),
    errors.join('\n'),
  );

  const forgetting = structuredClone(record);
  forgetting.groups[0].checks = forgetting.groups[0].checks.slice(1);
  const dropped = checkHostProbeData(forgetting, checkNames);
  assert.ok(dropped.some((error) => error.includes('which no group covers')), dropped.join('\n'));
});

test('a check named twice is refused', () => {
  const record = committedRecord();
  record.groups[1].checks = [record.groups[0].checks[0]];

  const errors = checkHostProbeData(record, checkNames);

  assert.ok(errors.some((error) => error.includes('is covered by')), errors.join('\n'));
});

test('an unknown status or outcome is refused', () => {
  const record = committedRecord();
  const cell = record.results.find((entry) => entry.status === 'probed');
  cell.status = 'probably-fine';
  assert.ok(
    checkHostProbeData(record, checkNames).some((error) => error.includes('unknown status')),
  );

  const other = committedRecord();
  other.results.find((entry) => entry.status === 'probed').outcome = 'looked-ok';
  assert.ok(
    checkHostProbeData(other, checkNames).some((error) => error.includes('without a known outcome')),
  );
});

test('a probed cell on a host nobody ran on is refused', () => {
  const record = committedRecord();
  record.hosts.find((host) => host.id === 'windows').probedOn = false;
  const cell = record.results.find((entry) => entry.host === 'windows');
  cell.status = 'probed';
  cell.outcome = 'ready';
  cell.evidence = 'copied from somewhere else';

  const errors = checkHostProbeData(record, checkNames);

  assert.ok(errors.some((error) => error.includes('probed on a host that was not probed')), errors.join('\n'));
});

test('a host that was not probed has to say what is missing', () => {
  const record = committedRecord();
  record.hosts.find((host) => host.id === 'macos').missing = '';
  record.results
    .filter((entry) => entry.host === 'macos')
    .forEach((entry) => {
      entry.reason = null;
    });

  const errors = checkHostProbeData(record, checkNames);

  assert.ok(errors.some((error) => error.includes('does not say what is missing')), errors.join('\n'));
});

test('exactly one host may be the machine the record came from', () => {
  const none = committedRecord();
  none.hosts.forEach((host) => {
    host.probedOn = false;
    host.missing = host.missing ?? 'nothing was run';
  });
  assert.ok(
    checkHostProbeData(none, checkNames).some((error) => error.includes('exactly one may be')),
  );

  const two = committedRecord();
  two.hosts[1].probedOn = true;
  two.hosts[1].environment = 'somewhere';
  assert.ok(
    checkHostProbeData(two, checkNames).some((error) => error.includes('exactly one may be')),
  );
});

test('a missing cell is refused', () => {
  const record = committedRecord();
  record.results.pop();

  const errors = checkHostProbeData(record, checkNames);

  assert.ok(errors.some((error) => error.includes('no cell for')), errors.join('\n'));
});

test('a document out of step with the record is refused', () => {
  const record = committedRecord();
  const markdown = readFileSync(path.join(root, DOCUMENT_PATH), 'utf8');
  const edited = markdown.replace('| probed: ready |', '| probed: gap |');
  assert.notEqual(edited, markdown, 'the fixture has to change something');

  const errors = checkHostProbeDocument(record, edited);

  assert.ok(errors.some((error) => error.includes('grid table')), errors.join('\n'));
});

test('the checker reads the probe names from the source', () => {
  assert.ok(Array.isArray(checkNames));
  assert.ok(checkNames.includes('project-scope'));
  assert.equal(probeCheckNames('nothing to see'), null);
});

test('the tables the document prints are derived from the record, not written twice', () => {
  const record = committedRecord();
  const markdown = readFileSync(path.join(root, DOCUMENT_PATH), 'utf8');

  assert.deepEqual(checkHostProbeDocument(record, markdown), []);
  assert.ok(
    checkHostProbeDocument({ ...record, statuses: {} }, markdown).length > 0,
    'an empty vocabulary cannot pass',
  );
});
