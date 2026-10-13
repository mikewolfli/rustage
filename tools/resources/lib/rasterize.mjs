// SPDX-License-Identifier: MIT
//
// Rasteriser probe and runner.
//
// SVG rasterisation is delegated to a fixed, externally versioned tool instead of a hand-written
// renderer (`docs/plan/resources.md` section 8.2). The chosen tool, its version and the digest of
// its output are recorded in the generation report, because pixel output differs between backends
// and only a named backend can be compared against a baseline.
//
// When no tool is available the pipeline reports a blocked stage with the probe results; it never
// falls back to a decorative approximation.

import { spawnSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { decodePng } from './png.mjs';

const CANDIDATES = [
  {
    name: 'rsvg-convert',
    versionArguments: ['--version'],
    arguments: (width, height, input, output) => ['-w', `${width}`, '-h', `${height}`, '-o', output, input],
  },
  {
    name: 'resvg',
    versionArguments: ['--version'],
    arguments: (width, height, input, output) => ['--width', `${width}`, `--height`, `${height}`, input, output],
  },
  {
    name: 'sips',
    versionArguments: ['--version'],
    arguments: (width, height, input, output) => ['-s', 'format', 'png', '-z', `${height}`, `${width}`, input, '--out', output],
  },
];

export function probeRasterizer() {
  const attempts = [];
  for (const candidate of CANDIDATES) {
    const probe = spawnSync(candidate.name, candidate.versionArguments, { encoding: 'utf8' });
    if (probe.error !== undefined && probe.error !== null) {
      attempts.push(`${candidate.name}: not available`);
      continue;
    }
    const version = `${probe.stdout ?? ''}${probe.stderr ?? ''}`.trim().split('\n')[0] ?? '';
    attempts.push(`${candidate.name}: ${version || 'available'}`);
    return { name: candidate.name, version, arguments: candidate.arguments, attempts };
  }
  return { name: null, version: null, arguments: null, attempts };
}

// Pins the rasterised size and replaces `currentColor` with a concrete colour, because a
// rasteriser has no theme to resolve `currentColor` from. The substitution happens on a temporary
// copy, never on the committed source. `sips -z` takes height before width.
export function prepareSvg(svgText, width, height, color) {
  return svgText
    .replace(/(<svg\b[^>]*?)\swidth="[^"]*"/, '$1 width="' + width + '"')
    .replace(/(<svg\b[^>]*?)\sheight="[^"]*"/, '$1 height="' + height + '"')
    .replace(/currentColor/g, color);
}

export function rasterizeToRgba(rasterizer, { svgText, width, height = width, color = '#000000', workDir, name }) {
  if (rasterizer.name === null) throw new Error('rasterizeToRgba: no rasteriser is available');
  mkdirSync(workDir, { recursive: true });
  const svgPath = path.join(workDir, `${name}.svg`);
  const pngPath = path.join(workDir, `${name}.png`);
  writeFileSync(svgPath, prepareSvg(svgText, width, height, color));
  const result = spawnSync(rasterizer.name, rasterizer.arguments(width, height, svgPath, pngPath), {
    encoding: 'utf8',
  });
  if (result.status !== 0) {
    throw new Error(
      `${rasterizer.name} failed on ${name} (exit ${result.status}): ${(result.stderr ?? '').trim()}`,
    );
  }
  const decoded = decodePng(readFileSync(pngPath));
  if (decoded.width !== width || decoded.height !== height) {
    throw new Error(
      `${rasterizer.name} produced ${decoded.width}x${decoded.height} for ${name}, expected ${width}x${height}`,
    );
  }
  return decoded;
}
