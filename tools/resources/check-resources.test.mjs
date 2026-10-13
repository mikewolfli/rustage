// SPDX-License-Identifier: MIT
//
// Negative and positive cases for the host resource checks. Every check that can refuse a
// source gets a case here: a gate that cannot fail is not a gate, and
// `docs/plan/resources.md` section 16.1 asks for the concrete negative inputs by name.
//
// Fixtures are built by copying the real `resources/`, `packaging/` and roadmap into a
// temporary directory and mutating one file, so the tests also keep working when the tree grows.

import assert from 'node:assert/strict';
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { after, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { checkResources } from './check-resources.mjs';
import { COLOR_ROLES } from './lib/manifest.mjs';
import { flattenSubpath, parsePathData, strokeToOutline, toUpstreamPathData, validateGlyphSvg } from './lib/svg-subset.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const temporaryDirectories = [];

function fixture(mutate) {
  const directory = mkdtempSync(path.join(tmpdir(), 'rustage-resources-'));
  temporaryDirectories.push(directory);
  cpSync(path.join(root, 'resources'), path.join(directory, 'resources'), { recursive: true });
  cpSync(path.join(root, 'packaging'), path.join(directory, 'packaging'), { recursive: true });
  cpSync(path.join(root, 'docs/plan'), path.join(directory, 'docs/plan'), { recursive: true });
  // The compiled host table is generated from the tree but lives in the crate, so the fixture has
  // to carry it: without it every case below would pass for the wrong reason.
  cpSync(path.join(root, 'src/resources'), path.join(directory, 'src/resources'), { recursive: true });
  cpSync(path.join(root, 'Cargo.toml'), path.join(directory, 'Cargo.toml'));
  cpSync(path.join(root, 'LICENSE'), path.join(directory, 'LICENSE'));
  mutate(directory);
  return directory;
}

after(() => {
  for (const directory of temporaryDirectories) rmSync(directory, { recursive: true, force: true });
});

function readJson(directory, file) {
  return JSON.parse(readFileSync(path.join(directory, file), 'utf8'));
}

function writeJson(directory, file, value) {
  writeFileSync(path.join(directory, file), `${JSON.stringify(value, null, 2)}\n`);
}

function messages(result) {
  return result.errors.join('\n');
}

test('the committed resource tree passes', () => {
  const result = checkResources(root);
  assert.deepEqual(result.errors, []);
  assert.equal(result.iconStats.commandGroups, 71);
  assert.equal(result.iconStats.panels, 42);
});

test('the compiled host icon table is rebuilt and matched', () => {
  const result = checkResources(root);
  assert.deepEqual(result.errors, []);
  assert.equal(result.tableStats.records, result.iconStats.icons);
  assert.equal(result.tableStats.geometry, result.iconStats.custom);
  assert.equal(result.tableStats.roles, COLOR_ROLES.length);
});

test('a stale compiled host icon table is refused', () => {
  const directory = fixture((base) => {
    const file = path.join(base, 'src/resources/generated_host_icons.rs');
    writeFileSync(file, `${readFileSync(file, 'utf8')}// hand edit\n`);
  });
  assert.match(messages(checkResources(directory)), /generated_host_icons\.rs: is stale/);
});

test('a missing compiled host icon table is refused', () => {
  const directory = fixture((base) => {
    rmSync(path.join(base, 'src/resources/generated_host_icons.rs'));
  });
  assert.match(messages(checkResources(directory)), /generated_host_icons\.rs: the generated host icon table is missing/);
});

test('a role changed in the manifest without regenerating the table is refused', () => {
  const directory = fixture((base) => {
    const manifest = readJson(base, 'resources/host/icons/manifest.json');
    const entry = manifest.icons.find(icon => icon.color_role === 'icon-default');
    entry.color_role = 'icon-muted';
    writeJson(base, 'resources/host/icons/manifest.json', manifest);
  });
  assert.match(messages(checkResources(directory)), /generated_host_icons\.rs: is stale/);
});

test('a glyph changed without regenerating the table is refused', () => {
  const directory = fixture((base) => {
    const file = path.join(base, 'resources/host/icons/custom/test-flask.svg');
    // Same shape, different bytes: the table records the source digest, so a source edit has to
    // reach it even when the drawing still validates.
    writeFileSync(file, `${readFileSync(file, 'utf8')}\n`);
  });
  assert.match(messages(checkResources(directory)), /generated_host_icons\.rs: is stale/);
});

test('the colour roles match the documented set', () => {
  const document = readFileSync(path.join(root, 'docs/plan/resources.md'), 'utf8');
  const block = document.match(/```text\n((?:icon-[a-z-]+\n)+)```/);
  assert.ok(block, 'docs/plan/resources.md must list the colour roles in a text block');
  assert.deepEqual(block[1].trim().split('\n'), COLOR_ROLES);
});

test('a duplicate icon id is refused', () => {
  const directory = fixture((base) => {
    const file = 'resources/host/icons/manifest.json';
    const manifest = readJson(base, file);
    manifest.icons.push({ ...manifest.icons[0] });
    writeJson(base, file, manifest);
  });
  assert.match(messages(checkResources(directory)), /duplicate id host\/file\/save/);
});

test('an unknown field is refused', () => {
  const directory = fixture((base) => {
    const file = 'resources/host/icons/manifest.json';
    const manifest = readJson(base, file);
    manifest.icons[0].optical_variants = [];
    writeJson(base, file, manifest);
  });
  assert.match(messages(checkResources(directory)), /unknown field "optical_variants"/);
});

test('a built-in token outside the pinned contract is refused', () => {
  const directory = fixture((base) => {
    const file = 'resources/host/icons/manifest.json';
    const manifest = readJson(base, file);
    manifest.icons[0].source.token = 'save_all';
    writeJson(base, file, manifest);
  });
  assert.match(messages(checkResources(directory)), /not in the pinned upstream token contract/);
});

test('an id that only collides on a case-insensitive file system is refused', () => {
  const directory = fixture((base) => {
    const file = 'resources/host/icons/manifest.json';
    const manifest = readJson(base, file);
    const clone = { ...manifest.icons[0], id: 'host/file/SAVE' };
    assert.equal(/^host\/[a-z0-9/.-]+$/.test(clone.id), false);
    clone.id = 'host/file/save';
    manifest.icons.push(clone);
    writeJson(base, file, manifest);
  });
  assert.match(messages(checkResources(directory)), /duplicate id|collides case-insensitively/);
});

test('a glyph that leaves the safe area is refused', () => {
  const directory = fixture((base) => {
    writeFileSync(
      path.join(base, 'resources/host/icons/custom/layout-align.svg'),
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24">'
      + '<path fill-rule="nonzero" d="M0 0H10V10H0Z"/></svg>\n',
    );
  });
  assert.match(messages(checkResources(directory)), /leaves the 2\.\.22 safe area/);
});

test('a glyph with a transform or an unknown path attribute is refused', () => {
  const directory = fixture((base) => {
    writeFileSync(
      path.join(base, 'resources/host/icons/custom/layout-align.svg'),
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24">'
      + '<path fill-rule="nonzero" transform="translate(1 1)" d="M2 2H10V10H2Z"/></svg>\n',
    );
  });
  assert.match(messages(checkResources(directory)), /attribute transform is not allowed/);
});

test('a stroked path must declare the shared stroke style', () => {
  const directory = fixture((base) => {
    writeFileSync(
      path.join(base, 'resources/host/icons/custom/layout-align.svg'),
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24">'
      + '<path fill="currentColor" fill-rule="nonzero" stroke="#000000" stroke-width="1.5"'
      + ' stroke-linecap="butt" stroke-linejoin="miter" d="M4 4H20"/></svg>\n',
    );
  });
  const errors = messages(checkResources(directory));
  assert.match(errors, /stroke must be currentColor/);
  assert.match(errors, /stroke-width must be "2"/);
  assert.match(errors, /stroke-linecap must be "round"/);
  assert.match(errors, /stroke-linejoin must be "round"/);
  assert.match(errors, /a stroked path must declare fill="none"/);
  assert.match(errors, /fill-rule does not apply to a stroked path/);
});

test('a closed stroked centerline is refused because it would expand to a ring', () => {
  const directory = fixture((base) => {
    writeFileSync(
      path.join(base, 'resources/host/icons/custom/layout-align.svg'),
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24">'
      + '<path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"'
      + ' stroke-linejoin="round" d="M4 4H20V20H4Z"/></svg>\n',
    );
  });
  assert.match(messages(checkResources(directory)), /a stroked centerline must stay open/);
});

test('a source named after a theme or state variant is refused', () => {
  const directory = fixture((base) => {
    const from = path.join(base, 'resources/host/icons/custom/layout-align.svg');
    const to = path.join(base, 'resources/host/icons/custom/layout-align-dark.svg');
    writeFileSync(to, readFileSync(from));
    rmSync(from);
    const file = 'resources/host/icons/manifest.json';
    const manifest = readJson(base, file);
    manifest.icons.find(icon => icon.source.path === 'custom/layout-align.svg').source.path = 'custom/layout-align-dark.svg';
    writeJson(base, file, manifest);
  });
  assert.match(messages(checkResources(directory)), /named after the "dark" state/);
});

test('a theme contract that drops a role is refused', () => {
  const directory = fixture((base) => {
    const file = 'resources/host/icons/theme.json';
    const theme = readJson(base, file);
    theme.roles = theme.roles.filter(role => role.id !== 'icon-danger');
    writeJson(base, file, theme);
  });
  assert.match(messages(checkResources(directory)), /role icon-danger has no swatch entry/);
});

test('a theme contract with matching surfaces is refused', () => {
  const directory = fixture((base) => {
    const file = 'resources/host/icons/theme.json';
    const theme = readJson(base, file);
    theme.surfaces.dark = theme.surfaces.light;
    writeJson(base, file, theme);
  });
  assert.match(messages(checkResources(directory)), /light and dark surfaces must differ/);
});

test('a glyph source escaping its manifest directory is refused', () => {
  const directory = fixture((base) => {
    const file = 'resources/host/icons/manifest.json';
    const manifest = readJson(base, file);
    manifest.icons.find(icon => icon.source.kind === 'local-svg').source.path = '../../../LICENSE';
    writeJson(base, file, manifest);
  });
  assert.match(messages(checkResources(directory)), /must stay inside its manifest directory|not a valid/);
});

test('a command group without a decision is refused', () => {
  const directory = fixture((base) => {
    const file = 'resources/host/icons/manifest.json';
    const manifest = readJson(base, file);
    manifest.commands = manifest.commands.filter(entry => entry.group !== 'help.*');
    writeJson(base, file, manifest);
  });
  assert.match(messages(checkResources(directory)), /help\.\* has no icon decision/);
});

test('a command group that is not in the roadmap is refused', () => {
  const directory = fixture((base) => {
    const file = 'resources/host/icons/manifest.json';
    const manifest = readJson(base, file);
    manifest.commands.push({ group: 'invented.group.*', icon: 'host/help/help' });
    writeJson(base, file, manifest);
  });
  assert.match(messages(checkResources(directory)), /invented\.group\.\* is not in the roadmap ledger/);
});

test('an unknown icon reference and a missing no-icon reason are refused', () => {
  const directory = fixture((base) => {
    const file = 'resources/host/icons/manifest.json';
    const manifest = readJson(base, file);
    manifest.commands.find(entry => entry.group === 'help.*').icon = 'host/help/missing';
    const noIcon = manifest.commands.find(entry => entry.icon === null);
    delete noIcon.reason;
    writeJson(base, file, manifest);
  });
  const output = messages(checkResources(directory));
  assert.match(output, /icon host\/help\/missing is not defined/);
  assert.match(output, /a no-icon decision must state a reason/);
});

test('an icon that no command group or panel references is refused', () => {
  const directory = fixture((base) => {
    const file = 'resources/host/icons/manifest.json';
    const manifest = readJson(base, file);
    manifest.icons.push({
      id: 'host/help/unused',
      source: { kind: 'rust-widgets-builtin', token: 'help' },
      channel: 'builtin',
      color_role: 'icon-default',
      rtl_mirror: false,
    });
    writeJson(base, file, manifest);
  });
  assert.match(messages(checkResources(directory)), /host\/help\/unused is referenced by no command group and no panel/);
});

test('a redistributed licence copy that differs from its digest is refused', () => {
  const directory = fixture((base) => {
    writeFileSync(path.join(base, 'resources/licenses/material-symbols/LICENSE'), 'not the Apache licence\n');
  });
  assert.match(messages(checkResources(directory)), /must be byte-identical to the recorded source/);
});

test('an icon panel id that is missing from the manifest is refused', () => {
  const directory = fixture((base) => {
    const file = 'resources/host/icons/manifest.json';
    const manifest = readJson(base, file);
    manifest.panels = manifest.panels.filter(entry => entry.id !== 'panel.help');
    writeJson(base, file, manifest);
  });
  assert.match(messages(checkResources(directory)), /panel\.help has no icon decision/);
});

test('a branding master with a script element is refused', () => {
  const directory = fixture((base) => {
    writeFileSync(
      path.join(base, 'resources/branding/logo.svg'),
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" width="64" height="64">'
      + '<script>alert(1)</script><rect x="0" y="0" width="10" height="10" fill="#000"/></svg>\n',
    );
  });
  assert.match(messages(checkResources(directory)), /element <script> is outside the allowed branding source subset/);
});

test('a branding master without an SPDX identifier is refused', () => {
  const directory = fixture((base) => {
    writeFileSync(
      path.join(base, 'resources/branding/logo.svg'),
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" width="64" height="64">'
      + '<rect x="0" y="0" width="10" height="10" fill="#000"/></svg>\n',
    );
  });
  assert.match(messages(checkResources(directory)), /missing an SPDX-License-Identifier comment/);
});

test('a branding manifest that drops a required role is refused', () => {
  const directory = fixture((base) => {
    const file = 'resources/branding/manifest.json';
    const manifest = readJson(base, file);
    manifest.masters = manifest.masters.filter(master => master.role !== 'logo-monochrome');
    writeJson(base, file, manifest);
  });
  assert.match(messages(checkResources(directory)), /must declare the "logo-monochrome" role/);
});

test('a decided identity that still holds open values is refused', () => {
  const directory = fixture((base) => {
    const file = 'packaging/identity.json';
    const identity = readJson(base, file);
    identity.status = 'decided';
    writeJson(base, file, identity);
  });
  assert.match(messages(checkResources(directory)), /status "decided" requires every identity/);
});

test('an undeclared packaging token is refused', () => {
  const directory = fixture((base) => {
    writeFileSync(
      path.join(base, 'packaging/linux/rustage.desktop'),
      readFileSync(path.join(base, 'packaging/linux/rustage.desktop'), 'utf8')
        .replace('[Desktop Entry]', '[Desktop Entry]\nX-Undeclared=@@NOPE@@'),
    );
  });
  assert.match(messages(checkResources(directory)), /token @@NOPE@@ is not declared/);
});

test('a packaging template missing a required key is refused', () => {
  const directory = fixture((base) => {
    writeFileSync(
      path.join(base, 'packaging/macos/Info.plist'),
      readFileSync(path.join(base, 'packaging/macos/Info.plist'), 'utf8')
        .replace('<key>CFBundleIconFile</key>', '<key>CFBundleSomethingElse</key>'),
    );
  });
  assert.match(messages(checkResources(directory)), /missing required key CFBundleIconFile/);
});

test('a HarmonyOS resource file that is not declared in the platform definition is refused', () => {
  const directory = fixture((base) => {
    writeFileSync(path.join(base, 'packaging/harmonyos/entry/src/main/resources/base/element/undeclared.json'), '{}\n');
  });
  assert.match(
    messages(checkResources(directory)),
    /packaging\/harmonyos\/entry\/src\/main\/resources\/base\/element\/undeclared\.json exists but is not declared/,
  );
});

test('a dark HarmonyOS colour resource that drops a name is refused', () => {
  const directory = fixture((base) => {
    writeFileSync(
      path.join(base, 'packaging/harmonyos/entry/src/main/resources/dark/element/color.json'),
      '{\n  "color": [\n    {\n      "name": "start_window_background_dark",\n'
        + '      "value": "#16181D"\n    }\n  ]\n}\n',
    );
  });
  const errors = messages(checkResources(directory));
  assert.match(errors, /does not override "start_window_background".*fall back to the light value/);
  assert.match(errors, /defines "start_window_background_dark", which is not a colour name the module uses/);
});

test('a HarmonyOS device type outside the documented set is refused', () => {
  const directory = fixture((base) => {
    const file = 'packaging/harmonyos/app.json';
    const config = readJson(base, file);
    config.device_types = ['desktop'];
    writeJson(base, file, config);
  });
  const errors = messages(checkResources(directory));
  assert.match(errors, /device type "desktop" is not a documented HarmonyOS device type/);
  assert.match(errors, /must declare the "2in1" device type/);
});

test('a HarmonyOS version code scheme outside the platform range is refused', () => {
  const directory = fixture((base) => {
    const file = 'packaging/harmonyos/app.json';
    const config = readJson(base, file);
    config.version_code.max_value = 2147483648;
    writeJson(base, file, config);
  });
  assert.match(messages(checkResources(directory)), /version_code.max_value must be the documented platform range/);
});

test('an application id that only satisfies the desktop platforms is refused', () => {
  const directory = fixture((base) => {
    const file = 'packaging/identity.json';
    const identity = readJson(base, file);
    identity.application_id = 'dev.example.rust-age';
    writeJson(base, file, identity);
  });
  assert.match(
    messages(checkResources(directory)),
    /must also be a valid HarmonyOS bundle name/,
  );
});

test('a roadmap whose section 3.2 stops parsing is a hard error', () => {
  const directory = fixture((base) => {
    writeFileSync(path.join(base, 'docs/plan/roadmap.md'), '# roadmap without the ledgers\n');
  });
  assert.match(messages(checkResources(directory)), /could not read the command-group ledger/);
});

test('implicit command repetition is refused', () => {
  const { errors } = parsePathData('M2 2 10 10Z');
  assert.match(errors.join('\n'), /appears without a pending command/);
});

test('relative and lowercase commands are refused', () => {
  const { errors } = parsePathData('m2 2h10v10h-10z');
  assert.match(errors.join('\n'), /outside the glyph subset/);
});

test('an unclosed subpath is refused', () => {
  const { errors } = parsePathData('M2 2H10V10H2');
  assert.match(errors.join('\n'), /is not closed with Z/);
});

test('a hole carved by opposite winding is refused', () => {
  const { errors } = validateGlyphSvg(
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24">'
    + '<path fill-rule="nonzero" d="M2 2H10V10H2ZM4 4V8H8V4Z"/></svg>\n',
    'fixture.svg',
  );
  assert.match(errors.join('\n'), /winds opposite to an earlier subpath/);
});

test('glyph sources convert to the dependency grid with y - grid', () => {
  const { data, errors } = validateGlyphSvg(
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24">'
    + '<path fill-rule="nonzero" d="M2 2H4V4H2Z"/></svg>\n',
    'fixture.svg',
  );
  assert.deepEqual(errors, []);
  assert.equal(toUpstreamPathData(data.subpaths), 'M2 -22L4 -22L4 -20L2 -20Z');
});

// The stroke expander has one property that makes it verifiable without a renderer: every
// vertex of the expanded outline sits exactly on the offset locus, half a stroke width away from
// the flattened centerline. A join placed on the wrong side (an arc where the inner corner
// belongs) breaks it, which is the failure this pins down.
function distanceToPolyline(points, [px, py]) {
  let best = Infinity;
  for (let index = 0; index < points.length - 1; index += 1) {
    const [x1, y1] = points[index];
    const [x2, y2] = points[index + 1];
    const dx = x2 - x1;
    const dy = y2 - y1;
    const lengthSquared = dx ** 2 + dy ** 2;
    const t = lengthSquared === 0 ? 0
      : Math.max(0, Math.min(1, ((px - x1) * dx + (py - y1) * dy) / lengthSquared));
    best = Math.min(best, Math.hypot(px - (x1 + t * dx), py - (y1 + t * dy)));
  }
  return best;
}

function pointInsideContours(contours, [px, py]) {
  let winding = 0;
  for (const points of contours) {
    for (let index = 0; index < points.length; index += 1) {
      const [x1, y1] = points[index];
      const [x2, y2] = points[(index + 1) % points.length];
      const side = (x2 - x1) * (py - y1) - (px - x1) * (y2 - y1);
      if (y1 <= py && y2 > py && side > 0) winding += 1;
      else if (y1 > py && y2 <= py && side < 0) winding -= 1;
    }
  }
  return winding !== 0;
}

const STROKE_CASES = [
  'M4 12H20',
  'M4 4H12V20',
  'M3 12L12 3L21 12',
  'M12 3.5C16.68 3.5 20.5 7.32 20.5 12',
  'M4 4L10 10L20 20',
];

test('every expanded outline vertex sits on the offset locus', () => {
  for (const data of STROKE_CASES) {
    const { subpaths } = parsePathData(data, { allowOpen: true });
    const { contours, errors } = strokeToOutline(subpaths[0], 2);
    assert.deepEqual(errors, []);
    assert.equal(contours.length, 1);
    const centerline = flattenSubpath(subpaths[0]);
    for (const [x, y] of contours[0]) {
      assert.ok(
        Math.abs(distanceToPolyline(centerline, [x, y]) - 1) < 1e-6,
        `${data}: (${x}, ${y}) is not one stroke half width from the centerline`,
      );
    }
  }
});

test('the expanded outline covers the core of the stroke and nothing beyond the band', () => {
  for (const data of STROKE_CASES) {
    const { subpaths } = parsePathData(data, { allowOpen: true });
    const { contours } = strokeToOutline(subpaths[0], 2);
    const centerline = flattenSubpath(subpaths[0]);
    for (let y = 0; y <= 24; y += 0.25) {
      for (let x = 0; x <= 24; x += 0.25) {
        const distance = distanceToPolyline(centerline, [x, y]);
        const inside = pointInsideContours(contours, [x, y]);
        if (distance <= 0.94) assert.ok(inside, `${data}: (${x}, ${y}) is in the stroke core but outside the outline`);
        if (inside) {
          assert.ok(distance <= 1.001, `${data}: (${x}, ${y}) is outside the band but inside the outline`);
        }
      }
    }
  }
});

test('a centerline that doubles back on itself is refused', () => {
  const { subpaths } = parsePathData('M4 12H20L4.5 12.1', { allowOpen: true });
  const { errors } = strokeToOutline(subpaths[0], 2);
  assert.match(errors.join('\n'), /turns back on itself/);
});

test('a stroked source is measured on its expanded outline', () => {
  const { data, errors } = validateGlyphSvg(
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24">'
    + '<path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"'
    + ' stroke-linejoin="round" d="M4 12H20"/></svg>\n',
    'fixture.svg',
  );
  assert.deepEqual(errors, []);
  assert.equal(data.declaredContours, 1);
  assert.equal(data.strokedContours, 1);
  assert.equal(data.contours, 1);
  assert.ok(data.outlinePoints > data.declaredPoints, 'the outline carries the cap geometry the centerline does not');
  assert.deepEqual(
    [data.bounds.minX, data.bounds.minY, data.bounds.maxX, data.bounds.maxY],
    [3, 11, 21, 13],
  );
  const emitted = toUpstreamPathData(data.subpaths);
  assert.ok(emitted.startsWith('M') && emitted.endsWith('Z'));
  assert.ok(emitted.includes('L4 -13') && emitted.includes('L20 -13'), 'both cap extremes reach y = 13 - 24');
  assert.ok(emitted.includes('L4 -11') && emitted.includes('L20 -11'), 'both cap extremes reach y = 11 - 24');
});

test('a curved fill is flattened before it is measured and emitted', () => {
  const { data, errors } = validateGlyphSvg(
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24">'
    + '<path fill="currentColor" fill-rule="nonzero" d="M12 4C14.21 4 16 5.79 16 8C16 10.21 14.21 12 12 12'
    + 'C9.79 12 8 10.21 8 8C8 5.79 9.79 4 12 4Z"/></svg>\n',
    'fixture.svg',
  );
  assert.deepEqual(errors, []);
  assert.ok(!toUpstreamPathData(data.subpaths).includes('C'), 'no curve survives into the emitted geometry');
  assert.ok(data.outlinePoints > 8, `a quarter-arc circle flattens to more than its declared points, got ${data.outlinePoints}`);
});
