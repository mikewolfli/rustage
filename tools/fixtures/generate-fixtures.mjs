#!/usr/bin/env node
// SPDX-License-Identifier: MIT
//
// Deterministic fixture generator for `fixtures/spec.json`.
//
// The sizes T00.07 has to fix are far too large to commit (the 2,000,000-line file alone is tens of
// megabytes), so the fixtures are *described* in the spec and *produced* on demand. That only helps
// if two runs produce the same bytes, so this generator is written to be reproducible: no random
// numbers, no timestamps, no iteration over object key order, no locale-dependent formatting. The
// check mode proves it by generating twice and comparing streaming SHA-256 digests.
//
// Usage:
//   node tools/fixtures/generate-fixtures.mjs --output target/fixtures          # write every fixture
//   node tools/fixtures/generate-fixtures.mjs --output target/fixtures --only normal
//   node tools/fixtures/generate-fixtures.mjs --verify                          # no disk writes, hashes only

import { createHash } from 'node:crypto';
import { mkdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { SPEC_PATH, readSpec } from './spec.mjs';

/** One generated file: its project-relative path and its exact bytes. */
function file(relativePath, content) {
  return { path: relativePath, content };
}

function json(value) {
  return `${JSON.stringify(value, null, 2)}\n`;
}

/** A page of `count` controls: deterministic ids, kinds and labels. */
function pageDocument(page, count, extraPropertyEvery = 7) {
  const nodes = [];
  for (let index = 0; index < count; index += 1) {
    const kind = ['button', 'label', 'text-field', 'checkbox'][index % 4];
    const node = {
      id: index + 1,
      kind,
      properties: {
        text: `${kind} ${index + 1}`,
        enabled: true,
      },
    };
    if (index % extraPropertyEvery === 0) node.properties.width = 120 + (index % 5) * 40;
    nodes.push(node);
  }
  return json({ formatVersion: 1, page, root: nodes.length > 0 ? nodes[0].id : null, nodes });
}

function projectDocument(pages, entry, target = 'desktop') {
  return json({
    formatVersion: 1,
    project: { name: 'fixture', target },
    entry,
    pages,
    styles: ['styles/app.css'],
    assets: ['assets'],
  });
}

function css(rules) {
  const lines = [':root {'];
  for (let index = 0; index < 12; index += 1) {
    lines.push(`  --token-${index + 1}: ${index % 2 === 0 ? '#1f6feb' : '#0d1117'};`);
  }
  lines.push('}', '');
  for (let index = 0; index < rules; index += 1) {
    lines.push(`.control-${index + 1} {`, `  padding: ${4 + (index % 8)}px;`, `  color: var(--token-${(index % 12) + 1});`, '}', '');
  }
  return lines.join('\n');
}

/** A Rust file with `lines` lines, no randomness, and a stable shape every 500 lines. */
function rustSource(lines, { longestLineBytes = 0 } = {}) {
  const chunks = ['// generated fixture: deterministic, no timestamps\n', 'pub struct Fixture;\n', '\n'];
  for (let index = 0; index < lines; index += 1) {
    if (index % 500 === 0) {
      chunks.push(`pub fn section_${String(index).padStart(7, '0')}() -> usize {\n`);
      chunks.push(`    ${index}\n`);
      chunks.push('}\n');
      chunks.push('\n');
    } else {
      chunks.push(`const VALUE_${String(index).padStart(7, '0')}: usize = ${index};\n`);
    }
  }
  if (longestLineBytes > 0) {
    chunks.push('// ');
    chunks.push('x'.repeat(Math.max(0, longestLineBytes - 4)));
    chunks.push('\n');
  }
  return chunks.join('');
}

function handwritten() {
  const lines = ['// SPDX-License-Identifier: MIT', '// Hand-written fixture logic: the generator must never rewrite this file.', ''];
  for (let index = 0; index < 80; index += 1) {
    lines.push(`pub fn handler_${index + 1}() -> usize {`, `    ${index * 3}`, '}', '');
  }
  return lines.join('\n');
}

function cargoToml(kind) {
  const lines = [
    '[package]',
    'name = "fixture-app"',
    'version = "0.1.0"',
    'edition = "2024"',
    '',
    '[dependencies]',
  ];
  if (kind === 'broken') {
    lines.push('this-line-is-not-a-dependency', '');
    lines.push('# three deliberate errors, one per line below');
    lines.push('fn not_a_dependency() {}');
    lines.push('[features');
    lines.push('missing-bracket = true');
    return `${lines.join('\n')}\n`;
  }
  lines.push('');
  return `${lines.join('\n')}\n`;
}

/**
 * The files of one fixture. Pure function of the spec: no clock, no randomness, no environment.
 */
export function fixtureFiles(spec, id) {
  const sizes = spec.sizes;
  switch (id) {
    case 'normal':
      return [
        file('Cargo.toml', cargoToml('ok')),
        file('designer-project.json', projectDocument(['ui/main.json'], 'ui/main.json')),
        file('ui/main.json', pageDocument('main', sizes.nodesNormal)),
        file('styles/app.css', css(sizes.cssRules)),
        file('src/handwritten/app.rs', handwritten()),
      ];
    case 'large-file':
      return [
        file('large/10k.rs', rustSource(sizes.linesSmall)),
        file('large/50k.rs', rustSource(sizes.linesBoundaryLow)),
        file('large/500k.rs', rustSource(sizes.linesMedium)),
        file('large/2m.rs', rustSource(sizes.linesBoundaryHigh)),
        file('large/longest-line.rs', rustSource(sizes.linesLongestLine, { longestLineBytes: sizes.longestLineBytes })),
        file('large/unicode.rs', rustSource(50).replace(
          'pub struct Fixture;',
          '// 中文标签 / emoji 🧭 / combining é / RTL: العربية\npub struct Fixture;',
        )),
      ];
    case 'multi-page': {
      const pages = [];
      for (let index = 0; index < sizes.pagesMultiPage; index += 1) pages.push(`ui/page-${String(index + 1).padStart(2, '0')}.json`);
      const files = [
        file('Cargo.toml', cargoToml('ok')),
        file('designer-project.json', projectDocument(pages, pages[0])),
        file('styles/app.css', css(sizes.cssRules)),
        file('src/handwritten/app.rs', handwritten()),
      ];
      const perPage = Math.floor(sizes.nodesMultiPage / sizes.pagesMultiPage);
      for (const [index, relative] of pages.entries()) {
        files.push(file(relative, pageDocument(`page-${index + 1}`, Math.max(1, perPage))));
      }
      return files;
    }
    case 'restricted-target':
      return [
        file('Cargo.toml', cargoToml('ok')),
        file('designer-project.json', projectDocument(['ui/main.json'], 'ui/main.json', 'embedded')),
        file('ui/main.json', pageDocument('main', 6)),
      ];
    case 'failing':
      return [
        file('Cargo.toml', cargoToml('broken')),
        file('designer-project.json', projectDocument(['ui/broken.json'], 'ui/broken.json')),
        // A truncated document: the parser must report the position, not silently drop the file.
        file('ui/broken.json', '{\n  "formatVersion": 1,\n  "page": "broken",\n  "nodes": [\n'),
        file('styles/app.css', css(4)),
        file('assets/missing.svg.reference', 'assets/missing.svg\n'),
      ];
    default:
      return null;
  }
}

/** Every fixture id in spec order. */
export function fixtureIds(spec) {
  return spec.fixtures.map((fixture) => fixture.id);
}

/** Streams one fixture's files through a SHA-256 digest, including the path names. */
export function digestFixture(spec, id) {
  const hash = createHash('sha256');
  for (const generated of fixtureFiles(spec, id)) {
    hash.update(`${generated.path}\n`);
    hash.update(generated.content);
  }
  return hash.digest('hex');
}

/** Writes one fixture under `output`, replacing what is there, and returns the file count. */
export function writeFixture(spec, id, output) {
  const files = fixtureFiles(spec, id);
  for (const generated of files) {
    const target = path.join(output, id, generated.path);
    mkdirSync(path.dirname(target), { recursive: true });
    writeFileSync(target, generated.content);
  }
  return files.length;
}

function parseArguments(argv) {
  const options = { output: null, only: null, verify: false };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === '--output') options.output = argv[++index];
    else if (argument === '--only') options.only = argv[++index];
    else if (argument === '--verify') options.verify = true;
    else if (argument === '--help' || argument === '-h') options.help = true;
    else throw new Error(`unknown argument: ${argument}`);
  }
  return options;
}

function main() {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
  const spec = readSpec(path.join(root, SPEC_PATH));

  let options;
  try {
    options = parseArguments(process.argv.slice(2));
  } catch (error) {
    console.error(`rustage fixtures: ${error.message}`);
    process.exitCode = 2;
    return;
  }
  if (options.help) {
    console.log('usage: generate-fixtures.mjs [--output <dir>] [--only <id>] [--verify]');
    return;
  }

  const ids = options.only ? [options.only] : fixtureIds(spec);
  for (const id of ids) {
    if (fixtureFiles(spec, id) === null) {
      console.error(`rustage fixtures: unknown fixture ${id}`);
      process.exitCode = 2;
      return;
    }
  }

  let failures = 0;
  for (const id of ids) {
    const first = digestFixture(spec, id);
    const second = digestFixture(spec, id);
    const deterministic = first === second;
    if (!deterministic) failures += 1;

    const files = fixtureFiles(spec, id);
    if (options.verify) {
      console.log(`${deterministic ? 'stable ' : 'UNSTABLE'} ${id} ${files.length} files sha256=${first.slice(0, 16)}`);
      continue;
    }
    if (!options.output) {
      console.error('rustage fixtures: --output <dir> is required unless --verify is used');
      process.exitCode = 2;
      return;
    }
    const written = writeFixture(spec, id, path.resolve(options.output));
    console.log(`${deterministic ? 'stable ' : 'UNSTABLE'} ${id} ${written} files sha256=${first.slice(0, 16)}`);
  }

  if (failures > 0) {
    console.error(`rustage fixtures: ${failures} fixture(s) are not reproducible`);
    process.exitCode = 1;
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
