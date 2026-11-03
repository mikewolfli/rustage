// SPDX-License-Identifier: MIT
//
// The HarmonyOS packaging output is driven by the declared template list, not by a second list in
// the generator (issue-20261013 §2.1 item 2). These cases pin that: every declared file is written,
// static files are written even while the identity is undecided, and a static file that carries a
// token is refused rather than shipped with a placeholder.

import assert from 'node:assert/strict';
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { after, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { writeHarmonyPackage } from './generate-resources.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const temporaryDirectories = [];

function fixture() {
  const directory = mkdtempSync(path.join(tmpdir(), 'rustage-harmony-'));
  temporaryDirectories.push(directory);
  cpSync(path.join(root, 'packaging'), path.join(directory, 'packaging'), { recursive: true });
  cpSync(path.join(root, 'Cargo.toml'), path.join(directory, 'Cargo.toml'));
  return directory;
}

after(() => {
  for (const directory of temporaryDirectories) {
    rmSync(directory, { recursive: true, force: true });
  }
});

function identity({ decided }) {
  return {
    version_source: { path: 'Cargo.toml', key: 'package.version' },
    application_id: decided ? 'com.example.rustagetest' : null,
    display_name: 'rustage',
    binary_name: 'rustage',
    vendor: 'rustage-test',
    copyright: 'Copyright (c) 2026 mikewolfli',
    minimum_os: { harmonyos: decided ? '6.0.0(20)' : null },
  };
}

test('every declared HarmonyOS template is written when the identity is decided', () => {
  const directory = fixture();
  const outDir = path.join(directory, 'out');
  const { outputs, blocked } = writeHarmonyPackage(directory, outDir, identity({ decided: true }));
  assert.deepEqual(blocked, []);

  const declared = JSON.parse(readFileSync(path.join(directory, 'packaging/harmonyos/app.json'), 'utf8')).templates;
  const written = new Set(outputs.map(entry => entry.path));
  for (const entry of declared) {
    assert.ok(
      written.has(`package/harmonyos/${entry.path}`),
      `${entry.path} is declared and must be written`,
    );
    const text = readFileSync(path.join(outDir, `package/harmonyos/${entry.path}`), 'utf8');
    assert.ok(!text.includes('@@'), `${entry.path} still carries a placeholder`);
  }

  const app = readFileSync(path.join(outDir, 'package/harmonyos/AppScope/app.json5'), 'utf8');
  assert.match(app, /"versionCode": \d+,/, 'versionCode must be a JSON number after substitution');
  assert.match(app, /"bundleName": "com\.example\.rustagetest"/);
});

test('static files are written even while the identity is undecided', () => {
  const directory = fixture();
  const outDir = path.join(directory, 'out');
  const { outputs, blocked } = writeHarmonyPackage(directory, outDir, identity({ decided: false }));

  const module = path.join(outDir, 'package/harmonyos/entry/src/main/module.json5');
  assert.equal(readFileSync(module, 'utf8'), readFileSync(path.join(directory, 'packaging/harmonyos/entry/src/main/module.json5'), 'utf8'));
  assert.ok(outputs.some(entry => entry.path.endsWith('entry/src/main/ets/pages/Index.ets')));
  assert.ok(blocked.some(entry => entry.template.endsWith('AppScope/app.json5')));
  assert.ok(blocked.some(entry => entry.template.endsWith('build-profile.json5')));
});

test('a static template that carries a token is refused', () => {
  const directory = fixture();
  const moduleFile = path.join(directory, 'packaging/harmonyos/entry/src/main/module.json5');
  writeFileSync(moduleFile, `${readFileSync(moduleFile, 'utf8')}\n// @@VENDOR@@\n`);
  const { blocked } = writeHarmonyPackage(directory, path.join(directory, 'out'), identity({ decided: true }));
  assert.ok(
    blocked.some(entry => entry.template.endsWith('module.json5')
      && entry.unresolved_tokens.includes('declared substituted:false but carries a token')),
    JSON.stringify(blocked),
  );
});

test('a declared path that escapes the package tree is refused', () => {
  const directory = fixture();
  const manifest = path.join(directory, 'packaging/harmonyos/app.json');
  const config = JSON.parse(readFileSync(manifest, 'utf8'));
  config.templates.push({ path: '../../outside.txt', role: 'escaped file', substituted: false });
  writeFileSync(manifest, `${JSON.stringify(config, null, 2)}\n`);
  const { blocked } = writeHarmonyPackage(directory, path.join(directory, 'out'), identity({ decided: true }));
  assert.ok(
    blocked.some(entry => entry.unresolved_tokens.includes('path escapes packaging/harmonyos')),
    JSON.stringify(blocked),
  );
});
