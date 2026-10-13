#!/usr/bin/env node
// SPDX-License-Identifier: MIT
//
// Host resource checks: the source tree, the icon manifest, the licence record, the branding
// masters and the packaging definitions.
//
// Scope, stated honestly: this proves the committed sources are internally consistent, that
// every command group and panel of the roadmap carries an explicit icon or no-icon decision,
// that the pinned upstream tokens are the ones referenced, that the redistributed licence copy
// is byte-identical to its recorded digest, and that packaging tokens are declared. It does not
// prove that any control renders an icon, that a glyph flattens inside the dependency's limits,
// or that a platform shows the packaged icon; those need the pinned dependency, a GUI host and
// the target platforms (see `docs/log/log-20261013-1.md`).
//
// Usage: node tools/resources/check-resources.mjs

import { createHash } from 'node:crypto';
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { checkTextFormatting, isResourceText } from './lib/format.mjs';
import { buildHostIconTable } from './lib/host-icon-table.mjs';
import {
  checkBranding, checkHostIcons, checkLicenses, checkPackaging, checkTheme, checkUpstreamTokens,
} from './lib/manifest.mjs';
import { readRoadmapLedger } from './lib/roadmap-ledger.mjs';
import { GLYPH_GRID, validateGlyphSvg, toUpstreamPathData } from './lib/svg-subset.mjs';

const ROOT_PATHS = ['resources', 'packaging'];

/// The generated Rust table the host compiles.
export const HOST_ICON_TABLE = 'src/resources/generated_host_icons.rs';

/**
 * Rebuilds `src/resources/generated_host_icons.rs` from the committed sources and compares bytes.
 *
 * This is the gate that keeps the compiled table honest: the dependency consumes
 * `&'static IconData`, so the host's own glyphs reach a control as Rust source, and a copy that no
 * longer matches the SVGs or the manifest would otherwise compile and draw something the
 * resources no longer say. Comparing the rebuilt bytes is stronger than checking that a
 * regenerate-equals-itself claim is written down, because it fails on the first divergent byte.
 */
export function checkHostIconTable(root) {
  const errors = [];
  const target = path.join(root, HOST_ICON_TABLE);
  if (!existsSync(target)) {
    errors.push(`${HOST_ICON_TABLE}: the generated host icon table is missing; `
      + 'run `node tools/resources/generate-resources.mjs`');
    return { errors, stats: null };
  }

  let built;
  try {
    const manifestFile = 'resources/host/icons/manifest.json';
    const iconManifest = JSON.parse(readFileSync(path.join(root, manifestFile), 'utf8'));
    const theme = JSON.parse(readFileSync(path.join(root, 'resources/host/icons/theme.json'), 'utf8'));
    if (!Array.isArray(iconManifest.icons) || !Array.isArray(theme.roles)) {
      return { errors, stats: null };
    }
    const glyphs = iconManifest.icons
      .filter(icon => icon.source?.kind === 'local-svg')
      .map((icon) => {
        const source = `resources/host/icons/${icon.source.path}`;
        const text = readFileSync(path.join(root, source), 'utf8');
        const { data, errors: glyphErrors } = validateGlyphSvg(text, source);
        if (glyphErrors.length > 0) throw new Error(glyphErrors.join('; '));
        return {
          id: icon.id,
          sourcePath: source,
          sourceSha256: createHash('sha256').update(text).digest('hex'),
          pathData: toUpstreamPathData(data.subpaths),
        };
      });
    built = buildHostIconTable({ iconManifest, theme, glyphs });
  } catch (error) {
    // The schema checks report the malformed file; this one only says the table could not be
    // rebuilt from it, so the two do not produce the same message twice.
    errors.push(`${HOST_ICON_TABLE}: cannot be rebuilt from the committed sources (${error.message})`);
    return { errors, stats: null };
  }

  if (readFileSync(target, 'utf8') !== built.text) {
    errors.push(`${HOST_ICON_TABLE}: is stale; the committed copy differs from the table the `
      + 'manifest and the SVG sources produce. Run `node tools/resources/generate-resources.mjs`');
  }

  return { errors, stats: built.stats };
}

function walk(root, relative) {
  const directory = path.join(root, relative);
  if (!existsSync(directory)) return [];
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const child = path.join(relative, entry.name);
    if (entry.isDirectory()) files.push(...walk(root, child));
    else files.push(child);
  }
  return files;
}

export function checkResources(root) {
  const errors = [];
  let checkedFiles = 0;

  const files = ROOT_PATHS.flatMap(prefix => walk(root, prefix)).sort();
  for (const file of files) {
    if (!isResourceText(file)) continue;
    checkedFiles += 1;
    errors.push(...checkTextFormatting(readFileSync(path.join(root, file), 'utf8'), file));
  }

  const ledger = readRoadmapLedger(root);
  errors.push(...ledger.errors);
  const upstream = checkUpstreamTokens(root);
  errors.push(...upstream.errors);

  let iconStats = null;
  let brandingStats = null;
  let packagingStats = null;
  if (ledger.errors.length === 0) {
    const icons = checkHostIcons(root, ledger, upstream);
    errors.push(...icons.errors);
    iconStats = icons.stats;
  }
  errors.push(...checkLicenses(root));
  errors.push(...checkTheme(root));
  const branding = checkBranding(root);
  errors.push(...branding.errors);
  brandingStats = branding.stats;
  const packaging = checkPackaging(root);
  errors.push(...packaging.errors);
  packagingStats = packaging.stats;
  const table = checkHostIconTable(root);
  errors.push(...table.errors);
  const tableStats = table.stats;

  return { checkedFiles, errors, iconStats, brandingStats, packagingStats, tableStats };
}

export function describe({ checkedFiles, iconStats, brandingStats, packagingStats, tableStats }) {
  const parts = [`${checkedFiles} resource files`];
  if (iconStats) {
    parts.push(
      `${iconStats.icons} icons (${iconStats.builtin} built-in, ${iconStats.custom} custom, `
      + `${iconStats.registryEntries} on the registry channel)`,
      `${iconStats.commandGroups} command groups and ${iconStats.panels} panels, `
      + `${iconStats.noIcon} explicit no-icon decisions`,
      `${iconStats.glyphBytes} custom glyph bytes`,
    );
  }
  if (brandingStats) {
    parts.push(
      `${brandingStats.masters} branding masters`,
      `platform icon frames ${brandingStats.windowsSizes}/${brandingStats.macosFrames}/`
        + `${brandingStats.linuxSizes}/${brandingStats.harmonyosFrames}`,
    );
  }
  if (packagingStats) {
    parts.push(`${packagingStats.tokens} packaging tokens`, `identity status ${packagingStats.status}`);
  }
  if (tableStats) {
    parts.push(
      `the compiled host table: ${tableStats.records} records, ${tableStats.geometry} local glyphs, `
      + `${tableStats.roles} roles`,
    );
  }
  return parts.join(', ');
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
  if (!statSync(path.join(root, 'Cargo.toml')).isFile()) {
    console.error(`resource checks: ${root} does not look like the rustage repository`);
    process.exitCode = 2;
  } else {
    const result = checkResources(root);
    if (result.errors.length > 0) {
      console.error(result.errors.join('\n'));
      process.exitCode = 1;
    } else {
      console.log(`Host resource checks passed (${describe(result)}).`);
    }
  }
}
