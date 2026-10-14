// SPDX-License-Identifier: MIT
//
// Negative and positive cases for the contract checks. Each rule gets a case here because a gate
// that cannot refuse anything is not a gate. The ones that matter most are the two that guard the
// acceptance line — "no second writable model" — by refusing an undeclared mutating entry point and
// refusing a public field, and the one that refuses an invariant pointing at a test that does not
// exist.

import assert from 'node:assert/strict';
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { after, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  checkContracts, checkContractsData, checkContractsDocument, checkWhitepaperSync, describe,
  mutatingEntryPoints, parseTables, publicFields, SCHEMA,
} from './check-contracts.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const contracts = JSON.parse(readFileSync(path.join(root, 'docs/plan/contracts.json'), 'utf8'));
const temporaryDirectories = [];

function fixture(mutateData, mutateDocument, mutateWhitepaper, mutateSource) {
  const directory = mkdtempSync(path.join(tmpdir(), 'rustage-contracts-'));
  temporaryDirectories.push(directory);
  for (const file of ['docs/plan/contracts.json', 'docs/plan/contracts.md', 'docs/plan/whitepaper.md']) {
    mkdirSync(path.join(directory, path.dirname(file)), { recursive: true });
    cpSync(path.join(root, file), path.join(directory, file));
  }
  cpSync(path.join(root, 'src/contracts'), path.join(directory, 'src/contracts'), { recursive: true });

  if (mutateData) {
    const dataFile = path.join(directory, 'docs/plan/contracts.json');
    const data = JSON.parse(readFileSync(dataFile, 'utf8'));
    mutateData(data);
    writeFileSync(dataFile, `${JSON.stringify(data, null, 2)}\n`);
  }
  for (const [file, mutate] of [
    ['docs/plan/contracts.md', mutateDocument],
    ['docs/plan/whitepaper.md', mutateWhitepaper],
  ]) {
    if (!mutate) continue;
    const target = path.join(directory, file);
    writeFileSync(target, mutate(readFileSync(target, 'utf8')));
  }
  if (mutateSource) {
    for (const [file, mutate] of Object.entries(mutateSource)) {
      const target = path.join(directory, file);
      writeFileSync(target, mutate(readFileSync(target, 'utf8')));
    }
  }
  return directory;
}

function messages(directory) {
  return checkContracts(directory).errors.join('\n');
}

function replace(text, from, to) {
  if (!text.includes(from)) throw new Error(`fixture text not found: ${from}`);
  return text.replace(from, to);
}

after(() => {
  for (const directory of temporaryDirectories) rmSync(directory, { recursive: true, force: true });
});

test('the committed data, document and whitepaper pass', () => {
  const result = checkContracts(root);
  assert.deepEqual(result.errors, []);
  assert.equal(result.stats.areas, 8);
  assert.equal(result.stats.permissions, 7);
  assert.equal(result.stats.invariants, 11);
});

test('describe names what was checked', () => {
  assert.equal(
    describe({ areas: 8, permissions: 7, invariants: 10, writers: 7, denials: 6 }),
    '8 contract areas, 7 permission rows, 10 invariants, 7 declared write entry points, 6 denial reasons',
  );
});

test('validating data and document separately reports the same shape of errors', () => {
  assert.deepEqual(checkContractsData(contracts, root), []);
  assert.deepEqual(checkContractsDocument(contracts, readFileSync(path.join(root, 'docs/plan/contracts.md'), 'utf8')), []);
  assert.deepEqual(
    checkWhitepaperSync(contracts, readFileSync(path.join(root, 'docs/plan/whitepaper.md'), 'utf8')),
    [],
  );
});

test('a data file with the wrong schema is refused', () => {
  const directory = fixture((data) => {
    data.schema = 'rustage.contracts/v2';
  });
  assert.match(messages(directory), new RegExp(`schema is "rustage.contracts/v2", expected ${SCHEMA}`));
});

test('an area whose module does not name the contract is refused', () => {
  const directory = fixture((data) => {
    data.areas[0].id = 'C-RENAMED';
  });
  assert.match(messages(directory), /never names its contract C-RENAMED/);
});

test('an area pointing at a missing module is refused', () => {
  const directory = fixture((data) => {
    data.areas[1].module = 'src/contracts/nowhere.rs';
  });
  assert.match(messages(directory), /areas\[1\]\.module does not exist: src\/contracts\/nowhere\.rs/);
});

test('a duplicated operation is refused', () => {
  const directory = fixture((data) => {
    data.permissions[1].operation = data.permissions[0].operation;
  });
  assert.match(messages(directory), /duplicate operation edit-widget-property/);
});

test('an unknown grant is refused', () => {
  const directory = fixture((data) => {
    data.permissions[0].grant = 'probably-fine';
  });
  assert.match(messages(directory), /uses an unknown grant "probably-fine"/);
});

test('an unknown denial is refused', () => {
  const directory = fixture((data) => {
    data.permissions[1].deniedAs = 'because';
  });
  assert.match(messages(directory), /uses an unknown denial "because"/);
});

test('a new mutating entry point that was not declared is refused', () => {
  const directory = fixture(null, null, null, {
    'src/contracts/store.rs': (source) => `${source}\n\nimpl DocumentStore {
    /// A second write path, added without touching the data file.
    pub fn set_kind_directly(&mut self) {}
}
`,
  });
  assert.match(messages(directory), /has public mutating entry points .*set_kind_directly/);
});

test('a declared writer that no longer exists is refused', () => {
  const directory = fixture((data) => {
    data.writers[0].names.push('write_anything');
  });
  assert.match(messages(directory), /declared .*write_anything/);
});

test('a public field is refused because it bypasses every entry point', () => {
  const directory = fixture(null, null, null, {
    'src/contracts/model.rs': (source) => replace(source, '    id: NodeId,\n', '    pub id: NodeId,\n'),
  });
  assert.match(messages(directory), /exposes public fields/);
});

test('an invariant whose test does not exist is refused', () => {
  const directory = fixture((data) => {
    data.invariants[0].test = 'a_test_that_was_never_written';
  });
  assert.match(messages(directory), /has no test named a_test_that_was_never_written/);
});

test('an invariant pointing at a missing area is refused', () => {
  const directory = fixture((data) => {
    data.invariants[0].area = 'C-NOWHERE';
  });
  assert.match(messages(directory), /names an unknown area C-NOWHERE/);
});

test('an invariant without a note is refused', () => {
  const directory = fixture((data) => {
    data.invariants[2].note = '';
  });
  assert.match(messages(directory), /invariants\[2\]\.note must be a non-empty string/);
});

test('a document edited out of step with the data is refused', () => {
  const directory = fixture(null, (markdown) => replace(
    markdown,
    '| C-STORE |',
    '| C-STORE-RENAMED |',
  ));
  assert.match(messages(directory), /the area table is/);
});

test('a missing document table is refused', () => {
  const directory = fixture(null, (markdown) => markdown
    .split('\n')
    .filter((line) => !line.startsWith('| 拒绝原因 |') && !/^\| (untrusted-project|needs-review|needs-confirmation|human-only|generated-code-read-only|scope-not-authorized) \|/.test(line))
    .join('\n'));
  assert.match(messages(directory), /the denial table is missing/);
});

test('a whitepaper grant that drifted from the data is refused', () => {
  const directory = fixture(null, null, (markdown) => replace(
    markdown,
    '| 增删控件节点 | **需评审通过** |',
    '| 增删控件节点 | **agent 可自主** |',
  ));
  assert.match(messages(directory), /whitepaper ### 12\.5 .* is \[/);
});

test('a whitepaper table removed from the section is refused', () => {
  const directory = fixture(null, null, (markdown) => replace(
    markdown,
    '| 操作类别 | 默认权限 | 理由 |',
    '| 操作类别 | 建议权限 | 理由 |',
  ));
  assert.match(messages(directory), /has no 操作类别\/默认权限\/理由 table/);
});

test('the mutating entry point scan separates public from crate-internal writes', () => {
  const source = `
    pub fn apply(&mut self) {}
    pub(crate) fn set_property(&mut self) {}
    pub fn view(&self) {}
    fn helper(&mut self) {}
`;
  assert.deepEqual(mutatingEntryPoints(source), {
    public: ['apply'],
    'crate-internal': ['set_property'],
  });
});

test('the public field scan ignores methods, structs and modules', () => {
  const source = `
pub struct Node {
    id: NodeId,
    pub(crate) page: PageId,
}
pub fn view(&self) {}
pub(crate) mod inner {}
`;
  assert.deepEqual(publicFields(source), []);
  assert.deepEqual(publicFields(`${source}    pub kind: String,\n`), ['pub kind:']);
});

test('tables are keyed by their header so two tables cannot be confused', () => {
  const tables = parseTables('| a | b |\n|---|---|\n| 1 | 2 |\n\n| a | c |\n|---|---|\n| 3 | 4 |\n');
  assert.equal(tables.size, 2);
  assert.deepEqual(tables.get(['a', 'b'].join('\u0000')), [['1', '2']]);
});
