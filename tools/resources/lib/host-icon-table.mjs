// SPDX-License-Identifier: MIT
//
// Renders the Rust icon table the host compiles (`src/resources/generated_host_icons.rs`).
//
// One builder, used twice on purpose: `generate-resources.mjs` writes the file and
// `check-resources.mjs` rebuilds it from the same sources and compares the bytes. A committed
// generated file therefore cannot go stale without a failure, and the table cannot drift from
// `resources/host/icons/manifest.json`, which stays the single source of the host's semantic
// icon decisions.
//
// # Why the table is a checked-in Rust file rather than `build.rs` output
//
// The dependency takes `&'static IconData`, so the geometry must exist as Rust source before
// `cargo build` runs. Producing it in a build script would mean either shelling out to Node
// (making every cargo build need a toolchain the crate otherwise does not use, and contradicting
// the no-download, no-desktop-tool rule for ordinary builds) or reimplementing the subset parser
// and the stroke expander in Rust (a second definition of the same geometry, which the one
// definition rule forbids). The file below is the third option: a reviewed artifact whose bytes
// are gated.

/** The Rust variant name for a documented role id: `icon-on-accent` -> `OnAccent`. */
export function rustRoleVariant(role) {
  const bare = role.startsWith('icon-') ? role.slice('icon-'.length) : role;
  return bare
    .split('-')
    .map(part => part.charAt(0).toUpperCase() + part.slice(1))
    .join('');
}

/** The Rust identifier for a semantic id: `host/design/canvas` -> `HOST_DESIGN_CANVAS`. */
export function rustStaticName(id) {
  return id
    .replace(/[^A-Za-z0-9]+/g, '_')
    .replace(/^_+|_+$/g, '')
    .toUpperCase();
}

function rustString(value) {
  if (typeof value !== 'string' || /["\\\r\n]/.test(value)) {
    throw new Error(`refusing to emit a Rust string literal for ${JSON.stringify(value)}`);
  }
  return `"${value}"`;
}

const HEADER = `// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT
//
// GENERATED FILE - do not edit by hand.
//
// Written by \`node tools/resources/generate-resources.mjs\` and rebuilt by
// \`node tools/resources/check-resources.mjs\`, which compares the bytes and fails when this
// copy is stale. Both read the same sources:
//
//   resources/host/icons/theme.json     - the semantic role vocabulary, in its declared order
//   resources/host/icons/manifest.json  - one record per icon: id, channel, role, mirroring
//   resources/host/icons/custom/*.svg   - the authoring sources of the local glyphs
//
// Geometry convention (dependency c957fdec): \`x\` spans \`0..grid\` and \`y\` spans \`-grid..0\`,
// so the authoring coordinates are shifted by \`y' = y - 24\`. Every contour is already a
// polygon because the source strokes were expanded during generation: the dependency flattens
// exactly the points below, so its 1024 point / 64 contour budget is judged against a count
// that is known here rather than estimated.
`;

/**
 * Builds the generated Rust table.
 *
 * `glyphs` carries the already expanded geometry of the local sources: one entry per icon whose
 * `source.kind` is `local-svg`, with `{ id, sourcePath, sourceSha256, pathData }`.
 */
export function buildHostIconTable({ iconManifest, theme, glyphs }) {
  const roles = theme.roles.map(role => role.id);
  const geometryById = new Map(glyphs.map(glyph => [glyph.id, glyph]));
  const records = [...iconManifest.icons].sort((left, right) => (left.id < right.id ? -1 : 1));

  const parts = [];
  parts.push(HEADER);
  parts.push('use rust_widgets::widget::IconData;\n');
  parts.push('use super::icons::{HostIconRecord, HostIconSource, IconRole};\n');
  parts.push('/// The semantic icon role vocabulary, in the order `resources/host/icons/theme.json`');
  parts.push('/// declares it. `IconRole::ALL` must agree with this list; the test asserts it.');
  parts.push(`pub const ICON_ROLE_NAMES: [&str; ${roles.length}] = [`);
  for (const role of roles) parts.push(`    ${rustString(role)},`);
  parts.push('];\n');

  const statics = [];
  for (const record of records) {
    if (record.source.kind !== 'local-svg') continue;
    const glyph = geometryById.get(record.id);
    if (glyph === undefined) {
      throw new Error(`${record.id}: declared as local-svg but no expanded geometry was supplied`);
    }
    statics.push({ record, glyph });
  }

  parts.push('// The local glyphs, one static per icon, named after its semantic id.\n');
  for (const { record, glyph } of statics) {
    parts.push(`/// \`${record.id}\` from \`${glyph.sourcePath}\` (sha256 \`${glyph.sourceSha256}\`).`);
    parts.push(`static ${rustStaticName(record.id)}: IconData = IconData {`);
    parts.push(`    name: ${rustString(record.id)},`);
    parts.push('    grid: 24,');
    parts.push('    paths: &[');
    parts.push(`        ${rustString(glyph.pathData)},`);
    parts.push('    ],');
    parts.push('};');
  }

  parts.push(`\n/// One record per host icon, sorted by id.\npub static HOST_ICON_RECORDS: [HostIconRecord; ${records.length}] = [`);
  for (const record of records) {
    const source = record.source.kind === 'rust-widgets-builtin'
      ? `HostIconSource::Builtin(${rustString(record.source.token)})`
      : `HostIconSource::StaticGeometry(&${rustStaticName(record.id)})`;
    parts.push(`    // ${record.id} (${record.channel})`);
    parts.push('    HostIconRecord {');
    parts.push(`        id: ${rustString(record.id)},`);
    parts.push(`        source: ${source},`);
    parts.push(`        role: IconRole::${rustRoleVariant(record.color_role)},`);
    parts.push(`        rtl_mirror: ${record.rtl_mirror === true ? 'true' : 'false'},`);
    parts.push('    },');
  }
  parts.push('];');

  const identifiers = new Set();
  for (const record of records) {
    const identifier = rustStaticName(record.id);
    if (identifiers.has(identifier)) {
      throw new Error(`${record.id}: the Rust identifier ${identifier} is not unique`);
    }
    identifiers.add(identifier);
  }

  return {
    text: `${parts.join('\n')}\n`,
    stats: {
      records: records.length,
      geometry: statics.length,
      roles: roles.length,
      bytes: Buffer.byteLength(parts.join('\n')) + 1,
    },
  };
}
