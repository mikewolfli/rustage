// SPDX-License-Identifier: MIT
//
// Negative and positive cases for the journey-walkthrough checks. A gate that cannot refuse
// anything is not a gate, so each rule gets a case: a journey that drifted from the roadmap, a
// step whose text changed, an entry that was invented, a panel no mode shows, an owner that is not
// a roadmap task, a missing entry without a reason, an aspect pointing nowhere, and a document
// edited out of step with the data.

import assert from 'node:assert/strict';
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { after, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  checkJourneys, checkJourneysData, checkJourneysDocument, describe, entryExists, parseManifest,
  parseRoadmapJourneys, parseRoadmapTasks, SCHEMA, JOURNEYS_PATH,
} from './check-journeys.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const journeys = JSON.parse(readFileSync(path.join(root, JOURNEYS_PATH), 'utf8'));
const manifestPath = journeys.manifest;
const roadmapPath = journeys.roadmap.file;
const temporaryDirectories = [];

function fixture(mutateData, mutateDocument, mutateRoadmap) {
  const directory = mkdtempSync(path.join(tmpdir(), 'rustage-journeys-'));
  temporaryDirectories.push(directory);
  for (const file of [JOURNEYS_PATH, 'docs/plan/journeys.md', roadmapPath, manifestPath]) {
    mkdirSync(path.join(directory, path.dirname(file)), { recursive: true });
    cpSync(path.join(root, file), path.join(directory, file));
  }
  if (mutateData) {
    const dataFile = path.join(directory, JOURNEYS_PATH);
    const data = JSON.parse(readFileSync(dataFile, 'utf8'));
    mutateData(data);
    writeFileSync(dataFile, `${JSON.stringify(data, null, 2)}\n`);
  }
  for (const [file, mutate] of [['docs/plan/journeys.md', mutateDocument], [roadmapPath, mutateRoadmap]]) {
    if (!mutate) continue;
    const target = path.join(directory, file);
    writeFileSync(target, mutate(readFileSync(target, 'utf8')));
  }
  return directory;
}

function messages(directory) {
  return checkJourneys(directory).errors.join('\n');
}

function replace(text, from, to) {
  if (!text.includes(from)) throw new Error(`fixture text not found: ${from}`);
  return text.replace(from, to);
}

after(() => {
  for (const directory of temporaryDirectories) rmSync(directory, { recursive: true, force: true });
});

test('the committed walkthrough, document and roadmap agree', () => {
  const result = checkJourneys(root);
  assert.deepEqual(result.errors, []);
  assert.equal(result.stats.modes, 7);
  assert.equal(result.stats.journeys, 26);
  assert.equal(result.stats.future, 3);
});

test('describe names the coverage and the one owned gap', () => {
  assert.equal(
    describe({ modes: 7, journeys: 26, steps: 171, stepsWithEntry: 170, release: 23, future: 3 }),
    '7 modes, 26 journeys (23 release, 3 future), 171 steps (170 with a designated entry, '
    + '1 owned without one)',
  );
});

test('validating data and document separately reports the same shape of errors', () => {
  assert.deepEqual(checkJourneysData(journeys, root), []);
  assert.deepEqual(checkJourneysDocument(journeys, readFileSync(path.join(root, 'docs/plan/journeys.md'), 'utf8')), []);
});

test('a data file with the wrong schema is refused', () => {
  const directory = fixture((data) => {
    data.schema = 'rustage.journeys/v2';
  });
  assert.match(messages(directory), new RegExp(`schema is "rustage.journeys/v2", expected ${SCHEMA}`));
});

test('claiming an implementation is refused while no entry is wired up', () => {
  const directory = fixture((data) => {
    data.implementation = 'partial';
  });
  assert.match(messages(directory), /`implementation` must stay "none"/);
});

test('a journey whose name drifted from the roadmap is refused', () => {
  const directory = fixture((data) => {
    data.journeys[0].name = '全新旅程';
  });
  assert.match(messages(directory), /J01 is named "全新旅程"/);
});

test('a journey that dropped a step is refused', () => {
  const directory = fixture((data) => {
    data.journeys[0].steps.pop();
  });
  assert.match(messages(directory), /J01 has 8 steps, the roadmap chain has 9/);
});

test('a step whose text drifted from the roadmap chain is refused', () => {
  const directory = fixture((data) => {
    data.journeys[0].steps[1].step = '随便新建';
  });
  assert.match(messages(directory), /J01 step 2 is "随便新建"/);
});

test('an entry that was invented is refused', () => {
  const directory = fixture((data) => {
    data.journeys[0].steps[0].entry = 'magic.new.branch';
  });
  assert.match(messages(directory), /entry "magic\.new\.branch" is neither a panel nor a menu leaf/);
});

test('a panel no mode shows is refused because the step would be undiscoverable', () => {
  const directory = fixture((data) => {
    for (const mode of data.modes) mode.panels = mode.panels.filter((panel) => panel !== 'panel.problems');
    data.journeys[2].steps[7].entry = 'panel.problems';
  });
  assert.match(messages(directory), /panel panel\.problems, which no mode shows/);
});

test('a mode showing an undeclared panel is refused', () => {
  const directory = fixture((data) => {
    data.modes[0].panels.push('panel.imaginary');
  });
  assert.match(messages(directory), /mode welcome shows an undeclared panel panel\.imaginary/);
});

test('a mode listing an entry that does not exist is refused', () => {
  const directory = fixture((data) => {
    data.modes[0].entries.push('file.explode.everything');
  });
  assert.match(messages(directory), /mode welcome lists an entry that does not exist/);
});

test('an owner that is not a roadmap task is refused', () => {
  const directory = fixture((data) => {
    data.journeys[0].steps[0].owner = 'T99.99';
  });
  assert.match(messages(directory), /owner "T99\.99" is not a roadmap task/);
});

test('a step with no entry and no reason is refused', () => {
  const directory = fixture((data) => {
    const step = data.journeys[20].steps[0];
    step.entry = null;
    delete step.reason;
  });
  assert.match(messages(directory), /J21 step 1\.reason must be a non-empty string/);
});

test('an aspect pointing past the steps is refused', () => {
  const directory = fixture((data) => {
    data.journeys[0].aspects.keyboard = 42;
  });
  assert.match(messages(directory), /J01 aspect keyboard does not point at one of its steps/);
});

test('a journey missing from the walkthrough is refused', () => {
  const directory = fixture((data) => {
    data.journeys = data.journeys.filter((journey) => journey.id !== 'J13');
  });
  assert.match(messages(directory), /covers 25 journeys, the roadmap defines 26/);
  assert.match(messages(directory), /J13 is missing from the walkthrough/);
});

test('a journey that is not in the roadmap is refused', () => {
  const directory = fixture((data) => {
    data.journeys.push({ ...data.journeys[0], id: 'J99', name: '额外旅程' });
  });
  assert.match(messages(directory), /J99 is not a journey in the roadmap/);
});

test('a scope that contradicts the roadmap milestone is refused', () => {
  const directory = fixture((data) => {
    data.journeys[20].scope = 'release';
  });
  assert.match(messages(directory), /J21 is marked release while the roadmap says FUTURE/);
});

test('a duplicated journey is refused', () => {
  const directory = fixture((data) => {
    data.journeys.push({ ...data.journeys[0] });
  });
  assert.match(messages(directory), /duplicate journey J01/);
});

test('a document edited out of step with the data is refused', () => {
  const directory = fixture(null, (markdown) => replace(
    markdown,
    '| J01 | 新用户首个应用 | release |',
    '| J01 | 新用户首个应用 | future |',
  ));
  assert.match(messages(directory), /the journey table is/);
});

test('a missing document table is refused', () => {
  const directory = fixture(null, (markdown) => markdown
    .split('\n')
    .filter((line) => !line.startsWith('| 模式 |') && !/^\| (welcome|project|design|code|run-debug|test|release) \|/.test(line))
    .join('\n'));
  assert.match(messages(directory), /the mode table is missing/);
});

test('an entry must match a declared group glob, not a similar-looking name', () => {
  const manifest = parseManifest(JSON.parse(readFileSync(path.join(root, manifestPath), 'utf8')));

  assert.ok(entryExists('file.new.project', manifest));
  assert.ok(entryExists('panel.problems', manifest));
  assert.ok(!entryExists('file.new', manifest), 'a group is not itself a leaf');
  assert.ok(!entryExists('file.new.project.extra', manifest), 'a leaf is one segment deep');
  assert.ok(!entryExists('file.explode.project', manifest));
});

test('journey names keep their spaces and task ids are collected', () => {
  const roadmap = readFileSync(path.join(root, roadmapPath), 'utf8');
  const parsed = parseRoadmapJourneys(roadmap, journeys.roadmap.section);

  assert.equal(parsed.length, 26);
  assert.equal(parsed.find((journey) => journey.id === 'J20').name, '任务与 CI 复现');
  assert.equal(parsed.find((journey) => journey.id === 'J01').steps.length, 9);
  assert.ok(parseRoadmapTasks(roadmap).has('T00.05'));
  assert.ok(!parseRoadmapTasks(roadmap).has('T99.99'));
});
