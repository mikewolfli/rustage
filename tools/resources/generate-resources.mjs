#!/usr/bin/env node
// SPDX-License-Identifier: MIT
//
// Derives the resource artifacts that are not committed:
//
//   * normalized geometry for the custom host glyphs, on the dependency's icon grid
//   * the compiled Rust icon table the host crate links (`src/resources/generated_host_icons.rs`)
//   * the platform icon frames: Linux hicolor PNGs and the scalable source, the macOS iconset and
//     ICNS, the Windows ICO
//   * a review sheet of the host glyphs plus the branding marks
//   * a generation report with tool versions, digests, blocked stages and the unverified list
//
// Everything lands under `target/resources/` (Cargo output, not tracked) and contains no
// timestamp, so running the generator twice produces identical bytes; the report states that
// reproducibility is checked by comparing digests, not asserted.
//
// The single exception is `src/resources/generated_host_icons.rs`: the dependency consumes
// `&'static IconData`, so the host table has to exist as compiled source. It is generated here,
// committed, and rebuilt byte for byte by `check-resources.mjs`; see `lib/host-icon-table.mjs`.
//
// The generator never writes into `resources/` or `packaging/`, never updates a baseline and never
// substitutes a placeholder for a decision that has not been made: a template whose token has no
// value is reported as blocked.
//
// Usage: node tools/resources/generate-resources.mjs [--out <directory>]

import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildHostIconTable } from './lib/host-icon-table.mjs';
import { encodeIcns, encodeIco } from './lib/icon-containers.mjs';
import { encodePng } from './lib/png.mjs';
import { downscale } from './lib/raster-scale.mjs';
import { probeRasterizer, rasterizeToRgba } from './lib/rasterize.mjs';
import { GLYPH_GRID, validateGlyphSvg, toAuthoringPathData, toUpstreamPathData } from './lib/svg-subset.mjs';
import { checkResources } from './check-resources.mjs';

const UNVERIFIED = [
  'No window has displayed the icon strip: the controls, the geometry and the theme switch are '
    + 'covered by the crate\'s own tests (src/app/toolbar.rs), but a platform window needs a display '
    + 'session the resource tooling does not have.',
  'DPI handling, RTL mirroring, focus/hover rendering and assistive-technology readout are not '
    + 'verified here; the crate tests cover size, the disabled state and a forced-colour mode.',
  'Windows Explorer, shortcut, taskbar and Alt-Tab rendering, macOS Finder/Dock rendering and '
    + 'GNOME/KDE launcher rendering are not verified; no target system was available.',
  'The packaged application identity is unset, so no package metadata was written and no '
    + 'installer, notarisation or SBOM step ran.',
];


function digest(buffer) {
  return createHash('sha256').update(buffer).digest('hex');
}

function readJson(root, file) {
  return JSON.parse(readFileSync(path.join(root, file), 'utf8'));
}

function packageVersion(root, versionSource) {
  const text = readFileSync(path.join(root, versionSource.path), 'utf8');
  const section = text.match(/\[package\]([\s\S]*?)(?:\n\[|$)/);
  if (section === null) throw new Error(`${versionSource.path}: no [package] section`);
  const value = section[1].match(/^\s*version\s*=\s*"([^"]+)"/m);
  if (value === null) throw new Error(`${versionSource.path}: no ${versionSource.key} value`);
  return value[1];
}

// The HarmonyOS version code is derived from the package version with the scheme declared in
// `packaging/harmonyos/app.json`, so it cannot drift from versionName. A version the scheme
// cannot represent is an error, never a truncated value.
function versionCode(root, identity, version) {
  const scheme = readJson(root, 'packaging/harmonyos/app.json').version_code;
  const parts = version.split('.').map(Number);
  if (parts.length !== scheme.segments.length || parts.some(part => !Number.isInteger(part) || part < 0)) {
    throw new Error(`${identity.version_source.path}: version ${version} does not have `
      + `${scheme.segments.length} numeric segments, so the HarmonyOS version code scheme cannot represent it`);
  }
  if (parts.some(part => part > scheme.max_segment)) {
    throw new Error(`${identity.version_source.path}: version ${version} has a segment above `
      + `${scheme.max_segment}, which the HarmonyOS version code scheme cannot represent`);
  }
  const code = parts.reduce((total, part, index) => total + part * scheme.segments[index], 0);
  if (code < 1 || code > scheme.max_value) {
    throw new Error(`${identity.version_source.path}: version ${version} maps to version code ${code}, `
      + `outside 1..${scheme.max_value}`);
  }
  return code;
}

function tokenValues(root, identity, platform, iconFile) {
  const version = packageVersion(root, identity.version_source);
  const [major = 0, minor = 0, patch = 0] = version.split('.').map(Number);
  return new Map([
    ['@@APPLICATION_ID@@', identity.application_id],
    ['@@DISPLAY_NAME@@', identity.display_name],
    ['@@BINARY_NAME@@', identity.binary_name],
    ['@@VENDOR@@', identity.vendor],
    ['@@VERSION@@', version],
    ['@@VERSION_QUAD@@', `${major},${minor},${patch},0`],
    ['@@VERSION_CODE@@', `${versionCode(root, identity, version)}`],
    ['@@COPYRIGHT@@', identity.copyright],
    ['@@ICON_FILE@@', iconFile],
    ['@@MINIMUM_OS@@', identity.minimum_os[platform]],
    ['@@INSTALL_PREFIX@@', identity.install_prefix],
  ]);
}

function substitute(template, values) {
  const missing = new Set();
  const output = template.replace(/@@[A-Z][A-Z0-9_]*@@/g, (token) => {
    const value = values.get(token);
    if (value === undefined) throw new Error(`unknown token ${token}`);
    if (value === null) {
      missing.add(token);
      return token;
    }
    return value;
  });
  return { output, missing: [...missing].sort() };
}

function write(outDir, relative, contents) {
  const target = path.join(outDir, relative);
  mkdirSync(path.dirname(target), { recursive: true });
  writeFileSync(target, contents);
  return { path: relative, bytes: Buffer.byteLength(contents), sha256: digest(Buffer.from(contents)) };
}

function colors(manifest) {
  return {
    brand: manifest.colors.brand_primary,
    mark: manifest.colors.mark_on_brand,
  };
}

function sheet(panels, width) {
  const resolved = width ?? Math.max(
    1,
    ...panels.flatMap(panel => panel.rows.map(row => row.x + row.image.width)),
  );
  const height = panels.reduce((total, panel) => total + panel.height, 0);
  const pixels = Buffer.alloc(resolved * height * 4);
  let cursorY = 0;
  for (const panel of panels) {
    for (let y = 0; y < panel.height; y += 1) {
      for (let x = 0; x < resolved; x += 1) {
        const target = ((cursorY + y) * resolved + x) * 4;
        pixels[target] = panel.background[0];
        pixels[target + 1] = panel.background[1];
        pixels[target + 2] = panel.background[2];
        pixels[target + 3] = 255;
      }
    }
    for (const row of panel.rows) {
      for (let y = 0; y < row.image.height; y += 1) {
        for (let x = 0; x < row.image.width; x += 1) {
          const source = (y * row.image.width + x) * 4;
          const targetX = row.x + x;
          const targetY = cursorY + row.y + y;
          if (targetX < 0 || targetY < 0 || targetX >= resolved || targetY >= height) continue;
          const target = (targetY * resolved + targetX) * 4;
          const alpha = row.image.rgba[source + 3] / 255;
          for (let channel = 0; channel < 3; channel += 1) {
            pixels[target + channel] = Math.round(
              row.image.rgba[source + channel] * alpha + pixels[target + channel] * (1 - alpha),
            );
          }
        }
      }
    }
    cursorY += panel.height;
  }
  return { width: resolved, height, rgba: pixels };
}

function placedPathData(subpaths, { size, x, y }) {
  return toAuthoringPathData(subpaths, { offset: [x, y], scale: size / GLYPH_GRID });
}

function sheetSvg({ width, height, panels }) {
  const parts = [
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}">`,
  ];
  for (const panel of panels) {
    parts.push(`<rect x="${panel.x}" y="${panel.y}" width="${panel.width}" height="${panel.height}" `
      + `fill="${panel.background}"/>`);
    for (const cell of panel.cells) {
      parts.push(`<path fill="${cell.color}" fill-rule="nonzero" d="${cell.d}"/>`);
    }
  }
  parts.push('</svg>');
  return parts.join('');
}

function hexToRgb(hex) {
  return [
    Number.parseInt(hex.slice(1, 3), 16),
    Number.parseInt(hex.slice(3, 5), 16),
    Number.parseInt(hex.slice(5, 7), 16),
  ];
}

function rasterMaster(rasterizer, svgText, pixelSize, color, workDir, name) {
  return rasterizeToRgba(rasterizer, { svgText, width: pixelSize, height: pixelSize, color, workDir, name });
}

function deriveAt(rasterizer, sources, size, color, workDir, name) {
  const small = sources.small;
  const large = sources.large;
  const useSmall = size <= small.maxPixels;
  const source = useSmall ? small : large;
  const image = source.raster === null || source.rasterColor !== color
    ? rasterMaster(rasterizer, source.svg, source.rasterPixels, color, workDir, `${name}-${source.role}`)
    : source.raster;
  source.raster = image;
  source.rasterColor = color;
  return downscale(image, size, size);
}

export function generate(root, outDir) {
  const verification = checkResources(root);
  if (verification.errors.length > 0) {
    throw new Error(`resource checks failed; refusing to derive from an invalid tree:\n${verification.errors.join('\n')}`);
  }

  const iconManifest = readJson(root, 'resources/host/icons/manifest.json');
  const branding = readJson(root, 'resources/branding/manifest.json');
  const theme = readJson(root, 'resources/host/icons/theme.json');
  const identity = readJson(root, 'packaging/identity.json');
  const palette = colors(branding);
  const report = { schema_version: 1, stages: [], rasterizer: null, sources: [], unverified: UNVERIFIED };

  const glyphs = iconManifest.icons
    .filter(icon => icon.source.kind === 'local-svg')
    .map(icon => ({
      id: icon.id,
      path: `resources/host/icons/${icon.source.path}`,
      text: readFileSync(path.join(root, `resources/host/icons/${icon.source.path}`), 'utf8'),
    }))
    .sort((left, right) => (left.id < right.id ? -1 : 1));

  for (const source of [...glyphs, ...branding.masters.map(master => ({
    id: `branding/${master.role}`,
    path: `resources/branding/${master.path}`,
    text: readFileSync(path.join(root, `resources/branding/${master.path}`), 'utf8'),
  }))]) {
    report.sources.push({ id: source.id, path: source.path, sha256: digest(source.text) });
  }

  // Stage 1: normalized geometry. Pure JavaScript, so it never blocks.
  const validated = glyphs.map((glyph) => {
    const { data, errors } = validateGlyphSvg(glyph.text, glyph.path);
    if (errors.length > 0) throw new Error(errors.join('\n'));
    return { id: glyph.id, data };
  });
  const geometry = validated.map(({ id, data }) => {
    const glyph = glyphs.find(candidate => candidate.id === id);
    return {
      id,
      source: glyph.path,
      source_sha256: digest(glyph.text),
      grid: GLYPH_GRID,
      y_transform: "y' = y - 24",
      path_data: toUpstreamPathData(data.subpaths),
      declared_contours: data.declaredContours,
      declared_points: data.declaredPoints,
      stroked_centerlines: data.strokedContours,
      outline_contours: data.contours,
      outline_points: data.outlinePoints,
      bounds: [data.bounds.minX, data.bounds.minY, data.bounds.maxX, data.bounds.maxY],
      capacity_note: 'the emitted geometry is polygonal, so the dependency flattens exactly these points',
    };
  });
  const geometryOutput = write(
    outDir,
    'generated/host-icons.geometry.json',
    `${JSON.stringify({ schema_version: 1, note: geometry[0]?.capacity_note, icons: geometry }, null, 2)}\n`,
  );
  report.stages.push({ name: 'geometry', status: 'ok', detail: `${geometry.length} glyphs`, outputs: [geometryOutput] });

  // Stage 1b: the Rust table the host compiles.
  //
  // This is the one output that is written into the source tree rather than `target/`, because
  // the dependency consumes `&'static IconData` and therefore needs it as compiled source. See
  // `lib/host-icon-table.mjs` for why it is a gated artifact instead of `build.rs` output.
  const table = buildHostIconTable({
    iconManifest,
    theme,
    glyphs: geometry.map(entry => ({
      id: entry.id,
      sourcePath: entry.source,
      sourceSha256: entry.source_sha256,
      pathData: entry.path_data,
    })),
  });
  const tablePath = 'src/resources/generated_host_icons.rs';
  const tableFile = path.join(root, tablePath);
  const previous = existsSync(tableFile) ? readFileSync(tableFile, 'utf8') : null;
  writeFileSync(tableFile, table.text);
  report.stages.push({
    name: 'host-icon-table',
    status: 'ok',
    detail: `${table.stats.records} records, ${table.stats.geometry} local glyphs, `
      + `${table.stats.roles} roles; ${previous === table.text ? 'unchanged' : 'written'}`,
    outputs: [{ path: tablePath, bytes: table.stats.bytes, sha256: digest(Buffer.from(table.text)) }],
  });

  // Stage 2: platform icon frames and the review sheets.
  const rasterizer = probeRasterizer();
  report.rasterizer = { name: rasterizer.name, version: rasterizer.version, attempts: rasterizer.attempts };
  const outputs = [];
  if (rasterizer.name === null) {
    const detail = `no SVG rasteriser found; probed ${rasterizer.attempts.join('; ')}`;
    report.stages.push({ name: 'platform-icons', status: 'blocked', detail, outputs: [] });
    report.stages.push({ name: 'review-sheet', status: 'blocked', detail: 'needs the rasteriser', outputs: [] });
    report.stages.push({ name: 'theme-matrix', status: 'blocked', detail: 'needs the rasteriser', outputs: [] });
  } else {
    const workDir = path.join(outDir, 'work');
    rmSync(workDir, { recursive: true, force: true });
    const smallMaster = branding.masters.find(master => master.role === 'app-icon-small');
    const largeMaster = branding.masters.find(master => master.role === 'app-icon');
    const sources = {
      small: {
        role: 'app-icon-small',
        svg: readFileSync(path.join(root, `resources/branding/${smallMaster.path}`), 'utf8'),
        rasterPixels: smallMaster.pixel_size,
        maxPixels: branding.derivation.small_master_max_pixels,
        raster: null,
        rasterColor: null,
      },
      large: {
        role: 'app-icon',
        svg: readFileSync(path.join(root, `resources/branding/${largeMaster.path}`), 'utf8'),
        rasterPixels: branding.derivation.macos.max_pixels,
        maxPixels: Infinity,
        raster: null,
        rasterColor: null,
      },
    };

    const windowsSizes = branding.derivation.windows.sizes;
    const linuxSizes = branding.derivation.linux.sizes;
    const macosFrames = branding.derivation.macos.iconset;
    const needed = new Set([...windowsSizes, ...linuxSizes, ...macosFrames.map(frame => frame.pixels)]);

    const frames = new Map();
    for (const size of [...needed].sort((left, right) => left - right)) {
      frames.set(size, deriveAt(rasterizer, sources, size, palette.brand, workDir, `frame-${size}`));
    }

    const base = identity.binary_name;
    for (const size of linuxSizes) {
      const frame = frames.get(size);
      outputs.push(write(outDir, `package/linux/hicolor/${size}x${size}/apps/${base}.png`, encodePng(frame)));
    }
    const scalableSource = readFileSync(path.join(root, `resources/branding/${largeMaster.path}`));
    mkdirSync(path.join(outDir, `package/linux/hicolor/scalable/apps`), { recursive: true });
    copyFileSync(
      path.join(root, `resources/branding/${largeMaster.path}`),
      path.join(outDir, `package/linux/hicolor/scalable/apps/${base}.svg`),
    );
    outputs.push({
      path: `package/linux/hicolor/scalable/apps/${base}.svg`,
      bytes: scalableSource.length,
      sha256: digest(scalableSource),
    });

    for (const frame of macosFrames) {
      outputs.push(write(outDir, `package/macos/${base}.iconset/${frame.name}`, encodePng(frames.get(frame.pixels))));
    }
    outputs.push(write(
      outDir,
      `package/macos/${base}.icns`,
      encodeIcns([...new Set(macosFrames.map(frame => frame.pixels))].sort((left, right) => left - right)
        .map(size => ({ size, ...frames.get(size) }))),
    ));
    outputs.push(write(
      outDir,
      `package/windows/${base}.ico`,
      encodeIco(
        windowsSizes.map(size => ({ size, ...frames.get(size) })),
        { bmpMaxSize: branding.derivation.windows.bmp_max_size },
      ),
    ));
    // HarmonyOS layered icon: both layers are rendered from the vector masters at the layer
    // size, so neither is an enlarged copy of a smaller frame.
    const harmonyos = branding.derivation.harmonyos;
    const layerMaster = role => branding.masters.find(master => master.role === role);
    const layerSvg = role => readFileSync(path.join(root, `resources/branding/${layerMaster(role).path}`), 'utf8');
    const layeredLayers = [
      ['background', 'app-icon-background', palette.brand],
      ['foreground', 'app-icon-foreground', palette.mark],
    ];
    for (const [layer, role, color] of layeredLayers) {
      const pixels = harmonyos.layered[layer];
      outputs.push(write(
        outDir,
        `package/harmonyos/AppScope/resources/base/media/${layer}.png`,
        encodePng(rasterMaster(rasterizer, layerSvg(role), pixels, color, workDir, `harmonyos-${layer}`)),
      ));
    }
    outputs.push(write(
      outDir,
      'package/harmonyos/entry/src/main/resources/base/media/app_icon.png',
      encodePng(deriveAt(rasterizer, sources, harmonyos.start_window_icon, palette.brand, workDir,
        'harmonyos-start-window')),
    ));
    report.stages.push({
      name: 'platform-icons',
      status: 'ok',
      detail: `${windowsSizes.length} ICO frames, ${macosFrames.length} iconset frames, `
        + `${linuxSizes.length} hicolor sizes, ${harmonyos.layered.background} px layered pair `
        + `and a ${harmonyos.start_window_icon} px start window icon`,
      outputs,
    });

    // Review sheet: every glyph at each logical size on both surfaces, in the default role, so
    // the question "does it still read at 16 px" has a picture. What is drawn here is the
    // expanded outline, which is the geometry the renderer receives, not the authoring source.
    const sizes = [12, 16, 20, 24, 32];
    const cell = 40;
    const defaultRole = theme.roles.find(role => role.id === 'icon-default');
    const contactPanels = [];
    const contactSections = [];
    let contactCursor = 0;
    for (const name of ['light', 'dark']) {
      contactPanels.push({
        x: contactCursor,
        y: 0,
        width: sizes.length * cell,
        height: validated.length * cell,
        background: theme.surfaces[name],
        cells: validated.flatMap((glyph, row) => sizes.map((size, column) => ({
          color: defaultRole[name],
          d: placedPathData(glyph.data.subpaths, {
            size,
            x: contactCursor + column * cell + (cell - size) / 2,
            y: row * cell + (cell - size) / 2,
          }),
        }))),
      });
      contactSections.push({
        name: `glyphs-on-${name}`,
        surface: theme.surfaces[name],
        ink: defaultRole[name],
        rows: validated.map(glyph => glyph.id),
        sizes,
        cell,
        origin: [contactCursor, 0],
      });
      contactCursor += sizes.length * cell + 8;
    }
    const contactWidth = contactCursor - 8;
    const contactHeight = validated.length * cell;
    const contactImage = rasterizeToRgba(rasterizer, {
      svgText: sheetSvg({ width: contactWidth, height: contactHeight, panels: contactPanels }),
      width: contactWidth,
      height: contactHeight,
      color: palette.brand,
      workDir,
      name: 'contact-sheet-glyphs',
    });

    const brandingRows = [];
    let brandingCursor = 8;
    const brandingSection = { name: 'branding', surface: theme.surfaces.light, rows: [], cell: 0, origin: [4, 8] };
    for (const master of branding.masters) {
      const size = master.role === 'app-icon' ? 128 : 64;
      const raster = rasterMaster(
        rasterizer,
        readFileSync(path.join(root, `resources/branding/${master.path}`), 'utf8'),
        Math.max(size, master.pixel_size),
        palette.brand,
        workDir,
        `brand-${master.role}`,
      );
      brandingRows.push({ x: 8, y: brandingCursor, image: downscale(raster, size, size) });
      brandingSection.rows.push({ role: master.role, size, origin: [8, brandingCursor] });
      brandingCursor += size + 16;
    }

    const sheetImage = sheet([
      { background: hexToRgb(theme.surfaces.light), height: contactHeight, rows: [{ x: 0, y: 0, image: contactImage }] },
      { background: hexToRgb(theme.surfaces.light), height: brandingCursor, rows: brandingRows },
    ]);
    outputs.push(write(outDir, 'reports/icon-contact-sheet.png', encodePng(sheetImage)));
    outputs.push(write(outDir, 'reports/icon-contact-sheet.json', `${JSON.stringify({
      schema_version: 1,
      note: 'Cell mapping for icon-contact-sheet.png. Every glyph is drawn from the expanded outline, '
        + 'so the sheet shows the geometry the dependency receives. The two glyph panels sit side by side, '
        + 'then the branding rows.',
      cell,
      sizes,
      sections: [...contactSections, brandingSection],
    }, null, 2)}\n`));
    report.stages.push({
      name: 'review-sheet',
      status: 'ok',
      detail: `${validated.length} glyphs at ${sizes.join('/')} px on both surfaces, plus the branding marks`,
      outputs: outputs.slice(-2),
    });

    // Theme matrix: the same single source in every semantic role on every surface. This is the
    // artefact that shows a theme change is a colour change, not a second file.
    const matrixCell = 32;
    const matrixRows = theme.roles.flatMap(role => ['light', 'dark'].map(name => ({
      role: role.id,
      theme: name,
      surface: role.preview_surface?.[name] ?? theme.surfaces[name],
      ink: role[name],
    })));
    const matrixPanels = matrixRows.map((row, index) => ({
      x: 0,
      y: index * matrixCell,
      width: validated.length * matrixCell,
      height: matrixCell,
      background: row.surface,
      cells: validated.map((glyph, column) => ({
        color: row.ink,
        d: placedPathData(glyph.data.subpaths, {
          size: 24,
          x: column * matrixCell + (matrixCell - 24) / 2,
          y: index * matrixCell + (matrixCell - 24) / 2,
        }),
      })),
    }));
    const matrixWidth = validated.length * matrixCell;
    const matrixHeight = matrixRows.length * matrixCell;
    const matrixImage = rasterizeToRgba(rasterizer, {
      svgText: sheetSvg({ width: matrixWidth, height: matrixHeight, panels: matrixPanels }),
      width: matrixWidth,
      height: matrixHeight,
      color: palette.brand,
      workDir,
      name: 'theme-matrix',
    });
    outputs.push(write(outDir, 'reports/icon-theme-matrix.png', encodePng(matrixImage)));
    outputs.push(write(outDir, 'reports/icon-theme-matrix.json', `${JSON.stringify({
      schema_version: 1,
      note: 'Cell mapping for icon-theme-matrix.png: one row per semantic role and theme, one column per '
        + 'glyph, drawn at 24 px from the same single source file. The swatches are review scaffolding '
        + 'from resources/host/icons/theme.json and are replaced by the host theme at T05.02/T24.02.',
      cell: matrixCell,
      glyph: 24,
      columns: validated.map(glyph => glyph.id),
      rows: matrixRows,
    }, null, 2)}\n`));
    report.stages.push({
      name: 'theme-matrix',
      status: 'ok',
      detail: `${matrixRows.length} role and theme rows across ${validated.length} glyphs`,
      outputs: outputs.slice(-2),
    });
    rmSync(workDir, { recursive: true, force: true });
  }

  // Stage 3: package metadata. Substitution is refused while the identity is undecided.
  const templates = [
    { platform: 'windows', file: 'packaging/windows/rustage.rc', iconFile: `${identity.binary_name}.ico` },
    { platform: 'macos', file: 'packaging/macos/Info.plist', iconFile: `${identity.binary_name}.icns` },
    { platform: 'linux', file: 'packaging/linux/rustage.desktop', iconFile: null },
    { platform: 'windows', file: 'packaging/windows/app.json', iconFile: null },
    { platform: 'linux', file: 'packaging/linux/app.json', iconFile: null },
    { platform: 'harmonyos', file: 'packaging/harmonyos/AppScope/app.json5', iconFile: null },
    { platform: 'harmonyos', file: 'packaging/harmonyos/AppScope/resources/base/element/string.json', iconFile: null },
    { platform: 'harmonyos', file: 'packaging/harmonyos/build-profile.json5', iconFile: null },
    {
      platform: 'harmonyos',
      file: 'packaging/harmonyos/entry/src/main/resources/base/element/string.json',
      iconFile: null,
    },
  ];
  const blocked = [];
  const metadataOutputs = [];
  for (const template of templates) {
    const text = readFileSync(path.join(root, template.file), 'utf8');
    const values = tokenValues(root, identity, template.platform, template.iconFile);
    const { output, missing } = substitute(text, values);
    if (missing.length > 0) {
      blocked.push({ template: template.file, unresolved_tokens: missing });
      continue;
    }
    const relative = template.file.slice(`packaging/${template.platform}/`.length);
    metadataOutputs.push(write(outDir, `package/${template.platform}/${relative}`, output));
  }
  report.stages.push({
    name: 'package-metadata',
    status: blocked.length === 0 ? 'ok' : 'blocked',
    detail: blocked.length === 0
      ? `${metadataOutputs.length} files`
      : `identity status ${identity.status}: ${blocked.map(entry => `${entry.template} needs ${entry.unresolved_tokens.join(', ')}`).join('; ')}`,
    blocked,
    outputs: metadataOutputs,
  });
  if (identity.application_id === null) {
    report.stages.push({
      name: 'installed-icon-name',
      status: 'blocked',
      detail: 'the installed icon basename must equal the application id; the icons are written under '
        + `the binary name "${identity.binary_name}" until T00.04 records the id`,
      outputs: [],
    });
  }

  const reportOutput = write(outDir, 'reports/generation-report.json', `${JSON.stringify(report, null, 2)}\n`);
  report.stages.push({ name: 'report', status: 'ok', detail: '', outputs: [reportOutput] });
  return report;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
  const outIndex = process.argv.indexOf('--out');
  const outDir = path.resolve(root, outIndex >= 0 ? process.argv[outIndex + 1] : 'target/resources');
  try {
    const report = generate(root, outDir);
    for (const stage of report.stages) {
      console.log(`${stage.status.padEnd(7)} ${stage.name}${stage.detail ? `: ${stage.detail}` : ''}`);
    }
    console.log(`artifacts under ${path.relative(root, outDir)}/`);
  } catch (error) {
    console.error(`resource generation failed: ${error.message}`);
    process.exitCode = 1;
  }
}
