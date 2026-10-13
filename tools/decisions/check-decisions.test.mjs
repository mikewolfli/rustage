// SPDX-License-Identifier: MIT
//
// Negative and positive cases for the decision-register checks. Each rule gets a case here because
// a gate that cannot refuse anything is not a gate: an unknown status, a decision without a
// rejected alternative, a decision with no compatibility impact or no affected task, a suspended
// question left unanswered, a document edited out of step with the data, and a whitepaper that no
// longer states the same conclusions must all fail.

import assert from 'node:assert/strict';
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { after, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  checkDecisions, checkDecisionsData, checkDecisionsDocument, checkWhitepaperSync, describe,
  DECISIONS_PATH, DOCUMENT_PATH, parseTables, SCHEMA, sectionText,
} from './check-decisions.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const register = JSON.parse(readFileSync(path.join(root, DECISIONS_PATH), 'utf8'));
const whitepaperPath = register.whitepaper.file;
const temporaryDirectories = [];

function fixture(mutateRegister, mutateDocument, mutateWhitepaper) {
  const directory = mkdtempSync(path.join(tmpdir(), 'rustage-decisions-'));
  temporaryDirectories.push(directory);
  for (const file of [DECISIONS_PATH, DOCUMENT_PATH, whitepaperPath]) {
    cpSync(path.join(root, file), path.join(directory, file));
  }
  const registerFile = path.join(directory, DECISIONS_PATH);
  if (mutateRegister) {
    const data = JSON.parse(readFileSync(registerFile, 'utf8'));
    mutateRegister(data);
    writeFileSync(registerFile, `${JSON.stringify(data, null, 2)}\n`);
  }
  for (const [file, mutate] of [[DOCUMENT_PATH, mutateDocument], [whitepaperPath, mutateWhitepaper]]) {
    if (!mutate) continue;
    const target = path.join(directory, file);
    writeFileSync(target, mutate(readFileSync(target, 'utf8')));
  }
  return directory;
}

function messages(directory) {
  return checkDecisions(directory).errors.join('\n');
}

/** Replaces the first occurrence of `from` with `to`, refusing to silently do nothing. */
function replace(markdown, from, to) {
  if (!markdown.includes(from)) throw new Error(`fixture text not found: ${from}`);
  return markdown.replace(from, to);
}

after(() => {
  for (const directory of temporaryDirectories) rmSync(directory, { recursive: true, force: true });
});

test('the committed register, document and whitepaper pass', () => {
  const result = checkDecisions(root);
  assert.deepEqual(result.errors, []);
  assert.equal(result.stats.decisions, 10);
  assert.equal(result.stats.open, 5);
});

test('describe names the decisions, the questions and the open items', () => {
  assert.equal(
    describe({ decisions: 2, questions: 2, open: 1, alternatives: 3, statuses: { ratified: 1, provisional: 1 } }),
    '2 decisions (1 ratified, 1 provisional), 2 suspended questions, 3 rejected alternatives, '
    + '1 open owner decisions',
  );
});

test('validating data and document separately reports the same shape of errors', () => {
  assert.deepEqual(checkDecisionsData(register), []);
  assert.deepEqual(checkDecisionsDocument(register, readFileSync(path.join(root, DOCUMENT_PATH), 'utf8')), []);
  assert.deepEqual(checkWhitepaperSync(register, readFileSync(path.join(root, whitepaperPath), 'utf8')), []);
});

test('a register with the wrong schema is refused', () => {
  const directory = fixture((data) => {
    data.schema = 'rustage.decisions/v2';
  });
  assert.match(messages(directory), new RegExp(`schema is "rustage.decisions/v2", expected ${SCHEMA}`));
});

test('an unknown status is refused', () => {
  const directory = fixture((data) => {
    data.decisions[0].status = 'probably-fine';
  });
  assert.match(messages(directory), /has status "probably-fine" outside the vocabulary/);
});

test('an open status on a settled decision is refused', () => {
  const directory = fixture((data) => {
    data.decisions[0].status = 'open';
  });
  assert.match(messages(directory), /is in `decisions` but has status open/);
});

test('a decision without a rejected alternative is refused', () => {
  const directory = fixture((data) => {
    data.decisions[0].alternatives = [];
  });
  assert.match(messages(directory), /Q1 records no rejected alternative/);
});

test('an alternative without a rejection reason is refused', () => {
  const directory = fixture((data) => {
    data.decisions[1].alternatives[0].rejected = '   ';
  });
  assert.match(messages(directory), /Q2\.alternatives\[0\]\.rejected must be a non-empty string/);
});

test('a decision without a compatibility impact is refused', () => {
  const directory = fixture((data) => {
    delete data.decisions[2].compatibilityImpact;
  });
  assert.match(messages(directory), /Q3\.compatibilityImpact must be a non-empty string/);
});

test('a decision without a revisit trigger is refused', () => {
  const directory = fixture((data) => {
    data.decisions[3].revisitTrigger = '';
  });
  assert.match(messages(directory), /Q4\.revisitTrigger must be a non-empty string/);
});

test('a decision that affects no task is refused', () => {
  const directory = fixture((data) => {
    data.decisions[4].affects = [];
  });
  assert.match(messages(directory), /Q5 names no affected task/);
});

test('a malformed affected task id is refused', () => {
  const directory = fixture((data) => {
    data.decisions[5].affects = ['T15'];
  });
  assert.match(messages(directory), /Q6\.affects\[0\] is not a task id: "T15"/);
});

test('a suspended question with neither decision nor open entry is refused', () => {
  const directory = fixture((data) => {
    data.decisions = data.decisions.filter((entry) => entry.id !== 'Q4');
  });
  assert.match(messages(directory), /suspended question Q4 has neither a decision nor an open entry/);
});

test('a duplicate id across decisions and open entries is refused', () => {
  const directory = fixture((data) => {
    data.openDecisions[0].id = 'Q1';
  });
  assert.match(messages(directory), /duplicate id Q1/);
});

test('an open entry without a reason is refused', () => {
  const directory = fixture((data) => {
    data.openDecisions[0].reason = '';
  });
  assert.match(messages(directory), /O-APP-ID\.reason must be a non-empty string/);
});

test('an open entry that pretends to be decided is refused', () => {
  const directory = fixture((data) => {
    data.openDecisions[0].status = 'ratified';
  });
  assert.match(messages(directory), /is in `openDecisions` but has status ratified/);
});

test('a document edited out of step with the data is refused', () => {
  const directory = fixture(null, (markdown) => replace(
    markdown,
    '| Q1 | 输出粒度 | ratified |',
    '| Q1 | 输出粒度 | provisional |',
  ));
  assert.match(messages(directory), /the decision table is/);
});

test('a missing document table is refused', () => {
  const directory = fixture(null, (markdown) => markdown
    .split('\n')
    .filter((line) => !line.startsWith('| 级别 |') && !line.startsWith('| ratified |')
      && !line.startsWith('| provisional |') && !line.startsWith('| deferred |')
      && !line.startsWith('| open |'))
    .join('\n'));
  assert.match(messages(directory), /the status-levels table is missing/);
});

test('a whitepaper conclusion that drifted from the register is refused', () => {
  const directory = fixture(null, null, (markdown) => replace(
    markdown,
    '| **Q2 生成物是否入库** | 设计源与生成 Rust 一并入库，生成文件标出受管理区域 | ratified |',
    '| **Q2 生成物是否入库** | 生成物不入库 | ratified |',
  ));
  assert.match(messages(directory), /Q2 row is/);
});

test('a whitepaper that drops the pointer to the register is refused', () => {
  const directory = fixture(null, null, (markdown) => {
    const section = sectionText(markdown, register.whitepaper.section);
    return replace(markdown, section, section.replace(/\[decisions\.md\]\(decisions\.md\)/g, ''));
  });
  assert.match(messages(directory), /does not point at decisions.md/);
});

test('a whitepaper that forgets a non-question decision is refused', () => {
  const directory = fixture(null, null, (markdown) => {
    const section = sectionText(markdown, register.whitepaper.section);
    return replace(markdown, section, section.replaceAll('`C1`', 'C1'));
  });
  assert.match(messages(directory), /does not mention the C1 decision/);
});

test('a whitepaper with no conclusion table is refused', () => {
  const directory = fixture(null, null, (markdown) => replace(
    markdown,
    '| 问题 | 结论 | 状态 | 详见 |',
    '| 问题 | 建议 | 状态 | 详见 |',
  ));
  assert.match(messages(directory), /has no 问题\/结论\/状态\/详见 table/);
});

test('the section lookup accepts a retitled heading and stops at the next one', () => {
  const text = sectionText(
    '# 标题\n\n### 17.7 新标题\n\n正文\n\n### 17.8 下一节\n\n其它\n',
    '### 17.7',
  );
  assert.match(text, /正文/);
  assert.doesNotMatch(text, /其它/);
});

test('the section lookup keeps a table together when it follows the heading', () => {
  const text = sectionText('### 17.7 结论\n\n| a | b |\n|---|---|\n| 1 | 2 |\n', '### 17.7');
  assert.equal(parseTables(text).get(['a', 'b'].join('\u0000')).length, 1);
});
