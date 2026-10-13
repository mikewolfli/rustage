// SPDX-License-Identifier: MIT
//
// Schema, identity and cross-reference checks for the resource tree. Every rule here comes
// from `docs/plan/resources.md` and is stated next to the code that enforces it: closed
// schemas (unknown fields are errors), the pinned upstream token contract, the semantic-ID
// naming rules, licence provenance that matches the redistributed copy byte for byte, and the
// roadmap ledger as the single list of command groups and panels.
//
// The checks are deliberately structural. They prove that the committed sources are
// self-consistent and that no requested icon is missing; they never claim that a control
// renders an icon, which is what the integration tasks must show.

import { createHash } from 'node:crypto';
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import path from 'node:path';
import { checkJsonDocument, checkTextFormatting, isPlainObject } from './format.mjs';
import { checkBrandingSvg, checkSvgHygiene } from './safe-svg.mjs';
import { validateGlyphSvg } from './svg-subset.mjs';

// `docs/plan/resources.md` section 9.1. A unit test asserts the document lists exactly these.
export const COLOR_ROLES = [
  'icon-default',
  'icon-muted',
  'icon-accent',
  'icon-on-accent',
  'icon-disabled',
  'icon-success',
  'icon-warning',
  'icon-danger',
];

// `docs/plan/resources.md` sections 7.2 and 14.2: at most 12 entries may use the legacy string
// registration path, and the custom glyph sources stay inside these byte budgets.
export const REGISTRY_BUDGET = 12;
export const GLYPH_BYTE_BUDGET = 32 * 1024;
export const TOTAL_GLYPH_BUDGET = 256 * 1024;

const ICON_ID_PATTERN = /^host\/[a-z0-9]+(?:-[a-z0-9]+)*(?:\/[a-z0-9]+(?:-[a-z0-9]+)*)+$/;
const GROUP_PATTERN = /^[a-z][a-z0-9]*(?:\.[a-z0-9]+)*\.\*$/;
const PANEL_PATTERN = /^panel\.[a-z0-9]+(?:-[a-z0-9]+)*$/;
const REGISTRY_TOKEN_PATTERN = /^rustage\.[a-z0-9]+(?:-[a-z0-9]+)*$/;
const PROVIDER_PATTERN = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
const HEX64_PATTERN = /^[0-9a-f]{64}$/;
const HEX_COLOR_PATTERN = /^#[0-9A-Fa-f]{6}$/;
const REVERSE_DNS_PATTERN = /^[a-z][a-z0-9]*(?:\.[a-z0-9][a-z0-9-]*)+$/;

// The platforms that carry packaging definitions. Each one owns a `packaging/<platform>/app.json`
// and the templates listed there.
export const PLATFORMS = ['windows', 'macos', 'linux', 'harmonyos'];

// HarmonyOS bundle names: at least three dot separated segments over `[A-Za-z0-9_]`, the first
// segment starting with a letter, 7 to 128 bytes, reverse domain recommended and **no hyphens**
// (OpenHarmony docs, application configuration file). That is stricter than the reverse-DNS rule
// the other platforms accept, and the shared identifier has to satisfy both.
const OHOS_BUNDLE_PATTERN = /^[A-Za-z][A-Za-z0-9_]*(?:\.[A-Za-z0-9_]+){2,}$/;

// Documented HarmonyOS device types (OpenHarmony docs, module configuration file).
const OHOS_DEVICE_TYPES = ['tablet', 'tv', 'wearable', 'car', '2in1', 'default'];

function listFiles(directory, prefix = '') {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const relative = prefix === '' ? entry.name : `${prefix}/${entry.name}`;
    return entry.isDirectory() ? listFiles(path.join(directory, entry.name), relative) : [relative];
  });
}

// `docs/plan/resources.md` section 9.1: a state or theme change is a colour and cache change,
// never a second glyph. A source named after one of these would be duplicated artwork, so the
// name is refused outright rather than reviewed later.
const VARIANT_TOKENS = ['dark', 'light', 'hover', 'pressed', 'disabled', 'active', 'focus', 'selected', 'checked'];

export function readText(root, file) {
  return readFileSync(path.join(root, file), 'utf8');
}

function checkClosed(relativeFile, label, value, allowed, errors, optional = []) {
  if (!isPlainObject(value)) {
    errors.push(`${label}: expected an object`);
    return false;
  }
  for (const key of Object.keys(value)) {
    if (!allowed.includes(key)) errors.push(`${label}: unknown field "${key}"`);
  }
  for (const key of allowed) {
    if (!(key in value) && !optional.includes(key)) errors.push(`${label}: missing field "${key}"`);
  }
  return true;
}

function checkArray(value, label, errors) {
  if (!Array.isArray(value) || value.length === 0) {
    errors.push(`${label}: expected a non-empty array`);
    return false;
  }
  return true;
}

function checkString(value, label, errors) {
  if (typeof value !== 'string' || value.trim() === '') {
    errors.push(`${label}: expected a non-empty string`);
    return false;
  }
  return true;
}

function checkEnum(value, label, allowed, errors) {
  if (!allowed.includes(value)) {
    errors.push(`${label}: "${value}" is not one of ${allowed.join(', ')}`);
    return false;
  }
  return true;
}

export function safePathWithin(baseDir, relative, label, errors, { allowOutsideBase = false } = {}) {
  if (typeof relative !== 'string' || relative.trim() === '') {
    errors.push(`${label}: expected a relative path`);
    return null;
  }
  if (path.isAbsolute(relative) || /^[A-Za-z][A-Za-z\d+.-]*:/.test(relative) || relative.includes('\0')) {
    errors.push(`${label}: "${relative}" must be a relative path without a scheme, drive or NUL`);
    return null;
  }
  if (!allowOutsideBase && relative.split(/[\\/]/).includes('..')) {
    errors.push(`${label}: "${relative}" must stay inside its manifest directory`);
    return null;
  }
  return path.resolve(baseDir, relative);
}

function checkRoadmapCoverage(entries, ledger, label, pattern, errors) {
  const seen = new Set();
  for (const [index, entry] of entries.entries()) {
    const key = pattern === GROUP_PATTERN ? entry.group : entry.id;
    const entryLabel = `${label}[${index}]`;
    if (typeof key !== 'string' || !pattern.test(key)) {
      errors.push(`${entryLabel}: "${key}" is not a valid ${pattern === GROUP_PATTERN ? 'command group' : 'panel id'}`);
      continue;
    }
    if (seen.has(key)) {
      errors.push(`${entryLabel}: duplicate entry for ${key}`);
      continue;
    }
    seen.add(key);
    if (!ledger.has(key)) errors.push(`${entryLabel}: ${key} is not in the roadmap ledger`);
  }
  for (const key of ledger) {
    if (!seen.has(key)) errors.push(`${label}: the roadmap ledger entry ${key} has no icon decision`);
  }
}

export function checkUpstreamTokens(root) {
  const file = 'resources/host/icons/upstream-tokens.json';
  const { errors, value } = checkJsonDocument(readText(root, file), file);
  const result = { errors, tokens: new Set(), revision: null, tokenCount: null };
  if (!isPlainObject(value)) return result;
  checkClosed(file, file, value, [
    'schema_version', 'provider', 'repository', 'revision', 'source', 'builtin_icon_count',
    'outline_source', 'note', 'tokens',
  ], errors);
  if (value.schema_version !== 1) errors.push(`${file}: schema_version must be 1`);
  checkString(value.provider, `${file}: provider`, errors);
  checkString(value.repository, `${file}: repository`, errors);
  if (checkString(value.revision, `${file}: revision`, errors) && !/^[0-9a-f]{40}$/.test(value.revision)) {
    errors.push(`${file}: revision must be a full 40-character commit SHA`);
  }
  result.revision = value.revision ?? null;
  if (isPlainObject(value.source)) {
    checkClosed('source', `${file}: source`, value.source, ['path', 'sha256'], errors);
    if (checkString(value.source.sha256, `${file}: source.sha256`, errors)
      && !HEX64_PATTERN.test(value.source.sha256)) {
      errors.push(`${file}: source.sha256 must be a lowercase SHA-256 digest`);
    }
  }
  if (checkArray(value.tokens, `${file}: tokens`, errors)) {
    for (const token of value.tokens) {
      if (typeof token !== 'string' || !/^[a-z][a-z0-9_]*$/.test(token)) {
        errors.push(`${file}: token "${token}" must be a lowercase snake_case upstream token`);
      }
      result.tokens.add(token);
    }
    if (result.tokens.size !== value.tokens.length) errors.push(`${file}: duplicate token entries`);
    if (value.builtin_icon_count !== value.tokens.length) {
      errors.push(
        `${file}: builtin_icon_count is ${value.builtin_icon_count} but ${value.tokens.length} tokens are listed`,
      );
    }
    result.tokenCount = value.tokens.length;
  }
  return result;
}

export function checkHostIcons(root, ledger, upstream) {
  const file = 'resources/host/icons/manifest.json';
  const manifestDir = path.dirname(path.join(root, file));
  const { errors, value } = checkJsonDocument(readText(root, file), file);
  if (!isPlainObject(value)) return { errors, stats: null };

  checkClosed(file, file, value, [
    'schema_version', 'upstream', 'note', 'icons', 'commands', 'panels',
  ], errors);
  if (value.schema_version !== 1) errors.push(`${file}: schema_version must be 1`);

  if (isPlainObject(value.upstream)) {
    checkClosed('upstream', `${file}: upstream`, value.upstream, ['name', 'revision', 'token_contract'], errors);
    if (value.upstream.revision !== upstream.revision) {
      errors.push(`${file}: upstream.revision must equal the token contract revision ${upstream.revision}`);
    }
    if (checkString(value.upstream.token_contract, `${file}: upstream.token_contract`, errors)) {
      const contract = safePathWithin(manifestDir, value.upstream.token_contract, `${file}: upstream.token_contract`, errors);
      if (contract !== null && !existsSync(contract)) {
        errors.push(`${file}: token contract ${value.upstream.token_contract} does not exist`);
      }
    }
  }

  const icons = new Map();
  const glyphBytes = { total: 0, files: new Map() };
  let registryEntries = 0;
  const ids = new Map();

  if (checkArray(value.icons, `${file}: icons`, errors)) {
    for (const [index, icon] of value.icons.entries()) {
      const label = `${file}: icons[${index}]`;
      if (!checkClosed(label, label, icon, [
        'id', 'source', 'channel', 'color_role', 'rtl_mirror', 'provenance', 'registry_token',
      ], errors, ['provenance', 'registry_token'])) continue;
      if (checkString(icon.id, `${label}: id`, errors)) {
        if (!ICON_ID_PATTERN.test(icon.id)) {
          errors.push(`${label}: id "${icon.id}" must be lowercase host/<segment>/<segment> with '-' inside segments`);
        }
        if (icons.has(icon.id)) errors.push(`${label}: duplicate id ${icon.id}`);
        icons.set(icon.id, icon);
        const lower = icon.id.toLowerCase();
        if (ids.has(lower)) errors.push(`${label}: ${icon.id} collides case-insensitively with ${ids.get(lower)}`);
        ids.set(lower, icon.id);
      }
      checkEnum(icon.color_role, `${label}: color_role`, COLOR_ROLES, errors);
      if (typeof icon.rtl_mirror !== 'boolean') {
        errors.push(`${label}: rtl_mirror must be declared as a boolean`);
      }

      const source = icon.source;
      if (!checkClosed(label, `${label}: source`, source, ['kind', 'token', 'path'], errors, ['token', 'path'])) continue;
      const kind = source.kind;
      if (checkEnum(kind, `${label}: source.kind`, ['rust-widgets-builtin', 'local-svg'], errors)) {
        const expectedKey = kind === 'rust-widgets-builtin' ? 'token' : 'path';
        const unexpectedKey = kind === 'rust-widgets-builtin' ? 'path' : 'token';
        if (!(expectedKey in source)) errors.push(`${label}: source of kind ${kind} must declare "${expectedKey}"`);
        if (unexpectedKey in source) {
          errors.push(`${label}: source of kind ${kind} must not declare "${unexpectedKey}"`);
        }
        if ('registry_token' in icon) {
          if (icon.channel !== 'string-registry') {
            errors.push(`${label}: registry_token is only allowed on the string-registry channel`);
          } else if (!REGISTRY_TOKEN_PATTERN.test(icon.registry_token)) {
            errors.push(`${label}: registry_token must look like "rustage.<name>"`);
          }
        }
        if (kind === 'rust-widgets-builtin') {
          checkEnum(icon.channel, `${label}: channel`, ['builtin'], errors);
          if (!upstream.tokens.has(source.token)) {
            errors.push(`${label}: token "${source.token}" is not in the pinned upstream token contract`);
          }
          if ('provenance' in icon) {
            errors.push(`${label}: built-in sources must not repeat upstream licence data; the contract records it`);
          }
        } else {
          checkEnum(icon.channel, `${label}: channel`, ['static-geometry', 'string-registry'], errors);
          if (icon.channel === 'string-registry') {
            registryEntries += 1;
            if (!('registry_token' in icon)) {
              errors.push(`${label}: the string-registry channel must declare registry_token`);
            }
          }
          const provenanceLabel = `${label}: provenance`;
          if (!checkClosed(provenanceLabel, provenanceLabel, icon.provenance, ['provider', 'spdx_license'], errors)) {
            continue;
          }
          checkString(icon.provenance.provider, `${provenanceLabel}: provider`, errors);
          checkString(icon.provenance.spdx_license, `${provenanceLabel}: spdx_license`, errors);
          const target = safePathWithin(manifestDir, source.path, `${label}: source.path`, errors);
          if (target === null) continue;
          for (const token of VARIANT_TOKENS) {
            if (new RegExp(`(^|[-_/])${token}([-_.]|$)`).test(source.path)) {
              errors.push(`${label}: ${source.path} is named after the "${token}" state; the theme supplies the `
                + 'colour, so one source serves every theme and state (docs/plan/resources.md section 9.1)');
            }
          }
          if (!existsSync(target)) {
            errors.push(`${label}: source file ${source.path} does not exist`);
            continue;
          }
          const size = statSync(target).size;
          if (size > GLYPH_BYTE_BUDGET) {
            errors.push(`${label}: ${source.path} is ${size} bytes, above the ${GLYPH_BYTE_BUDGET} byte glyph budget`);
          }
          glyphBytes.total += size;
          glyphBytes.files.set(source.path, size);
          const text = readFileSync(target, 'utf8');
          const display = `resources/host/icons/${source.path}`;
          const glyph = validateGlyphSvg(text, display);
          errors.push(...glyph.errors);
          errors.push(...checkSvgHygiene(text, display));
        }
      }
    }
  }
  if (registryEntries > REGISTRY_BUDGET) {
    errors.push(`${file}: ${registryEntries} icons use the string registry, above the budget of ${REGISTRY_BUDGET}`);
  }
  if (glyphBytes.total > TOTAL_GLYPH_BUDGET) {
    errors.push(`${file}: custom glyph sources total ${glyphBytes.total} bytes, above ${TOTAL_GLYPH_BUDGET}`);
  }

  const referenced = new Set();
  if (checkArray(value.commands, `${file}: commands`, errors)) {
    for (const [index, entry] of value.commands.entries()) {
      const label = `${file}: commands[${index}]`;
      if (!checkClosed(label, label, entry, ['group', 'icon', 'reason'], errors, ['reason'])) continue;
      if (entry.icon === null) {
        if (!('reason' in entry)) errors.push(`${label}: a no-icon decision must state a reason`);
        else checkString(entry.reason, `${label}: reason`, errors);
      } else if (checkString(entry.icon, `${label}: icon`, errors) && !icons.has(entry.icon)) {
        errors.push(`${label}: icon ${entry.icon} is not defined`);
      } else if (entry.icon !== null) {
        referenced.add(entry.icon);
      }
    }
    checkRoadmapCoverage(value.commands, ledger.commandGroups, `${file}: commands`, GROUP_PATTERN, errors);
  }

  if (checkArray(value.panels, `${file}: panels`, errors)) {
    for (const [index, entry] of value.panels.entries()) {
      const label = `${file}: panels[${index}]`;
      if (!checkClosed(label, label, entry, ['id', 'icon', 'reason'], errors, ['reason'])) continue;
      if (entry.icon === null) {
        if (!('reason' in entry)) errors.push(`${label}: a no-icon decision must state a reason`);
        else checkString(entry.reason, `${label}: reason`, errors);
      } else if (checkString(entry.icon, `${label}: icon`, errors) && !icons.has(entry.icon)) {
        errors.push(`${label}: icon ${entry.icon} is not defined`);
      } else if (entry.icon !== null) {
        referenced.add(entry.icon);
      }
    }
    checkRoadmapCoverage(value.panels, ledger.panels, `${file}: panels`, PANEL_PATTERN, errors);
  }

  for (const id of icons.keys()) {
    if (!referenced.has(id)) {
      errors.push(`${file}: icon ${id} is referenced by no command group and no panel; dead resources are removed`);
    }
  }

  return {
    errors,
    stats: {
      icons: icons.size,
      builtin: [...icons.values()].filter(icon => icon.source?.kind === 'rust-widgets-builtin').length,
      custom: [...icons.values()].filter(icon => icon.source?.kind === 'local-svg').length,
      registryEntries,
      glyphBytes: glyphBytes.total,
      commandGroups: value.commands?.length ?? 0,
      panels: value.panels?.length ?? 0,
      noIcon: [...(value.commands ?? []), ...(value.panels ?? [])].filter(entry => entry.icon === null).length,
    },
  };
}

export function checkTheme(root) {
  const file = 'resources/host/icons/theme.json';
  const { errors, value } = checkJsonDocument(readText(root, file), file);
  if (!isPlainObject(value)) return errors;
  checkClosed(file, file, value, ['schema_version', 'status', 'note', 'surfaces', 'roles', 'replacement'], errors);
  if (value.schema_version !== 1) errors.push(`${file}: schema_version must be 1`);
  checkEnum(value.status, `${file}: status`, ['proposed-pending-approval', 'decided'], errors);
  checkString(value.note, `${file}: note`, errors);

  const surface = (label, entry) => {
    if (!checkClosed(label, label, entry, ['light', 'dark'], errors)) return;
    for (const theme of ['light', 'dark']) {
      if (!HEX_COLOR_PATTERN.test(entry[theme] ?? '')) {
        errors.push(`${label}.${theme} must be a #RRGGBB colour`);
      }
    }
  };
  surface(`${file}: surfaces`, value.surfaces);
  if (isPlainObject(value.surfaces) && value.surfaces.light === value.surfaces.dark) {
    errors.push(`${file}: the light and dark surfaces must differ, otherwise the preview shows nothing`);
  }

  const seen = new Set();
  if (checkArray(value.roles, `${file}: roles`, errors)) {
    for (const [index, role] of value.roles.entries()) {
      const label = `${file}: roles[${index}]`;
      if (!checkClosed(label, label, role, ['id', 'light', 'dark', 'preview_surface'], errors, ['preview_surface'])) {
        continue;
      }
      checkEnum(role.id, `${label}: id`, COLOR_ROLES, errors);
      if (seen.has(role.id)) errors.push(`${label}: duplicate role ${role.id}`);
      seen.add(role.id);
      for (const theme of ['light', 'dark']) {
        if (!HEX_COLOR_PATTERN.test(role[theme] ?? '')) {
          errors.push(`${label}.${theme} must be a #RRGGBB colour`);
        }
      }
      if ('preview_surface' in role) surface(`${label}.preview_surface`, role.preview_surface);
    }
    for (const role of COLOR_ROLES) {
      if (!seen.has(role)) errors.push(`${file}: role ${role} has no swatch entry`);
    }
  }

  if (checkClosed('replacement', `${file}: replacement`, value.replacement,
    ['tasks', 'reference', 'note'], errors)) {
    if (checkArray(value.replacement.tasks, `${file}: replacement.tasks`, errors)) {
      for (const task of value.replacement.tasks) {
        if (typeof task !== 'string' || !/^T\d{2}\.\d{2}$/.test(task)) {
          errors.push(`${file}: replacement.tasks entries must be roadmap task ids`);
        }
      }
    }
    for (const field of ['reference', 'note']) {
      checkString(value.replacement[field], `${file}: replacement.${field}`, errors);
    }
  }
  return errors;
}

export function checkLicenses(root) {
  const file = 'resources/licenses/manifest.json';
  const manifestDir = path.dirname(path.join(root, file));
  const { errors, value } = checkJsonDocument(readText(root, file), file);
  if (!isPlainObject(value)) return errors;
  checkClosed(file, file, value, ['schema_version', 'note', 'entries'], errors);
  if (value.schema_version !== 1) errors.push(`${file}: schema_version must be 1`);
  checkString(value.note, `${file}: note`, errors);
  const providers = new Set();
  if (!checkArray(value.entries, `${file}: entries`, errors)) return errors;
  let ownProviderSeen = false;

  for (const [index, entry] of value.entries.entries()) {
    const label = `${file}: entries[${index}]`;
    if (!checkClosed(label, label, entry, [
      'provider', 'component', 'spdx_license', 'license_file', 'attribution_file', 'source',
      'distributed_as', 'rustage_use', 'modifications',
    ], errors)) continue;
    if (checkString(entry.provider, `${label}: provider`, errors)) {
      if (!PROVIDER_PATTERN.test(entry.provider)) {
        errors.push(`${label}: provider "${entry.provider}" must be lowercase kebab-case`);
      }
      if (providers.has(entry.provider)) errors.push(`${label}: duplicate provider ${entry.provider}`);
      providers.add(entry.provider);
      if (entry.provider === 'rustage') ownProviderSeen = true;
    }
    checkString(entry.component, `${label}: component`, errors);
    checkString(entry.spdx_license, `${label}: spdx_license`, errors);
    checkEnum(entry.distributed_as, `${label}: distributed_as`, [
      'in-repository-source', 'builtin-icon-data-inside-dependency', 'generated-derived-artifact',
    ], errors);
    checkString(entry.rustage_use, `${label}: rustage_use`, errors);
    checkString(entry.modifications, `${label}: modifications`, errors);
    if (entry.attribution_file !== null
      && checkString(entry.attribution_file, `${label}: attribution_file`, errors)) {
      const attribution = safePathWithin(manifestDir, entry.attribution_file, `${label}: attribution_file`, errors);
      if (attribution !== null && !existsSync(attribution)) {
        errors.push(`${label}: attribution file ${entry.attribution_file} does not exist`);
      }
    }

    const licenseFile = safePathWithin(manifestDir, entry.license_file, `${label}: license_file`, errors, {
      allowOutsideBase: true,
    });
    if (licenseFile !== null) {
      const fromRoot = path.relative(root, licenseFile);
      if (fromRoot.startsWith('..')) {
        errors.push(`${label}: license_file must stay inside the repository`);
      } else if (!existsSync(licenseFile)) {
        errors.push(`${label}: license file ${entry.license_file} does not exist`);
      }
    }

    const sourceLabel = `${label}: source`;
    if (!checkClosed(sourceLabel, sourceLabel, entry.source, [
      'repository', 'revision', 'path', 'sha256', 'received_through', 'received_revision',
    ], errors, ['received_through', 'received_revision'])) continue;
    checkString(entry.source.repository, `${sourceLabel}: repository`, errors);
    checkString(entry.source.revision, `${sourceLabel}: revision`, errors);
    checkString(entry.source.path, `${sourceLabel}: path`, errors);
    if (entry.source.sha256 === null) {
      if (entry.provider !== 'rustage') {
        errors.push(`${sourceLabel}: sha256 may only be null for the project's own material`);
      }
    } else if (checkString(entry.source.sha256, `${sourceLabel}: sha256`, errors)) {
      if (!HEX64_PATTERN.test(entry.source.sha256)) {
        errors.push(`${sourceLabel}: sha256 must be a lowercase SHA-256 digest`);
      } else if (licenseFile !== null && existsSync(licenseFile)) {
        const digest = createHash('sha256').update(readFileSync(licenseFile)).digest('hex');
        if (digest !== entry.source.sha256) {
          errors.push(
            `${label}: the redistributed copy ${entry.license_file} must be byte-identical to the recorded `
            + `source (expected ${entry.source.sha256}, found ${digest})`,
          );
        }
      }
    }
  }
  if (!ownProviderSeen) errors.push(`${file}: the project's own material must be recorded with provider "rustage"`);
  return errors;
}

export function checkBranding(root) {
  const file = 'resources/branding/manifest.json';
  const brandingDir = path.dirname(path.join(root, file));
  const { errors, value } = checkJsonDocument(readText(root, file), file);
  if (!isPlainObject(value)) return { errors, stats: null };
  checkClosed(file, file, value, [
    'schema_version', 'status', 'note', 'colors', 'wordmark', 'clear_space', 'masters', 'derivation',
  ], errors);
  if (value.schema_version !== 1) errors.push(`${file}: schema_version must be 1`);
  checkEnum(value.status, `${file}: status`, ['proposed-pending-approval', 'approved'], errors);
  checkString(value.note, `${file}: note`, errors);

  if (checkClosed('colors', `${file}: colors`, value.colors, [
    'brand_primary', 'gradient_start', 'gradient_end', 'mark_on_brand',
  ], errors)) {
    for (const [name, color] of Object.entries(value.colors)) {
      if (!HEX_COLOR_PATTERN.test(color)) errors.push(`${file}: colors.${name} must be a #RRGGBB value`);
    }
  }
  if (checkClosed('wordmark', `${file}: wordmark`, value.wordmark, ['embedded_in_masters', 'note'], errors)) {
    if (value.wordmark.embedded_in_masters !== false) {
      errors.push(`${file}: the wordmark must not be embedded as outlines in the masters`);
    }
    checkString(value.wordmark.note, `${file}: wordmark.note`, errors);
  }
  if (checkClosed('clear_space', `${file}: clear_space`, value.clear_space, ['master_ratio', 'note'], errors)) {
    const ratio = value.clear_space.master_ratio;
    if (typeof ratio !== 'number' || ratio <= 0 || ratio >= 0.5) {
      errors.push(`${file}: clear_space.master_ratio must be between 0 and 0.5`);
    }
  }

  const roles = new Set();
  if (checkArray(value.masters, `${file}: masters`, errors)) {
    for (const [index, master] of value.masters.entries()) {
      const label = `${file}: masters[${index}]`;
      if (!checkClosed(label, label, master, [
        'role', 'path', 'pixel_size', 'view_box', 'artwork_bounds', 'background', 'used_for',
      ], errors)) continue;
      if (checkString(master.role, `${label}: role`, errors)) {
        if (roles.has(master.role)) errors.push(`${label}: duplicate role ${master.role}`);
        roles.add(master.role);
      }
      checkEnum(master.background, `${label}: background`, ['none', 'flat', 'gradient'], errors);
      checkString(master.used_for, `${label}: used_for`, errors);
      if (master.view_box !== `0 0 ${master.pixel_size} ${master.pixel_size}`) {
        errors.push(`${label}: view_box must be "0 0 ${master.pixel_size} ${master.pixel_size}"`);
      }
      if (Array.isArray(master.artwork_bounds) && master.artwork_bounds.length === 4) {
        const [minX, minY, maxX, maxY] = master.artwork_bounds;
        const size = master.pixel_size;
        if (![minX, minY, maxX, maxY].every(value => typeof value === 'number')) {
          errors.push(`${label}: artwork_bounds must be numbers`);
        } else if (minX < 0 || minY < 0 || maxX > size || maxY > size || minX >= maxX || minY >= maxY) {
          errors.push(`${label}: artwork_bounds must sit inside the view box`);
        }
      } else {
        errors.push(`${label}: artwork_bounds must be [minX, minY, maxX, maxY]`);
      }
      const target = safePathWithin(brandingDir, master.path, `${label}: path`, errors);
      if (target === null) continue;
      if (!existsSync(target)) {
        errors.push(`${label}: master ${master.path} does not exist`);
        continue;
      }
      errors.push(...checkBrandingSvg(readFileSync(target, 'utf8'), `resources/branding/${master.path}`, {
        allowedElements: [
          'defs', 'linearGradient', 'radialGradient', 'stop', 'rect', 'circle', 'ellipse', 'path',
          'g', 'title', 'desc',
        ],
        allowedAttributes: [
          'viewBox', 'width', 'height', 'fill', 'fill-rule', 'd', 'x', 'y', 'rx', 'ry', 'cx', 'cy',
          'r', 'points', 'offset', 'stop-color', 'stop-opacity', 'id', 'x1', 'y1', 'x2', 'y2',
          'gradientUnits', 'opacity',
        ],
      }));
    }
  }
  for (const required of ['app-icon', 'app-icon-small', 'logo', 'logo-monochrome',
    'app-icon-background', 'app-icon-foreground']) {
    if (!roles.has(required)) errors.push(`${file}: the branding masters must declare the "${required}" role`);
  }

  const derivation = value.derivation;
  if (checkClosed('derivation', `${file}: derivation`, derivation, [
    'small_master_max_pixels', 'windows', 'macos', 'linux', 'harmonyos',
  ], errors)) {
    const small = value.masters?.find(master => master.role === 'app-icon-small');
    if (small !== undefined && small.pixel_size > derivation.small_master_max_pixels) {
      errors.push(`${file}: the small master must not exceed derivation.small_master_max_pixels`);
    }
    if (checkClosed('windows', `${file}: derivation.windows`, derivation.windows, [
      'format', 'sizes', 'bmp_max_size', 'record',
    ], errors)) {
      if (derivation.windows.format !== 'ico') errors.push(`${file}: derivation.windows.format must be "ico"`);
      if (!Array.isArray(derivation.windows.sizes) || derivation.windows.sizes.length === 0) {
        errors.push(`${file}: derivation.windows.sizes must list at least one pixel size`);
      } else if (!derivation.windows.sizes.includes(16) || !derivation.windows.sizes.includes(256)) {
        errors.push(`${file}: derivation.windows.sizes must include the 16 px and 256 px frames`);
      }
      if (!derivation.windows.sizes?.includes(derivation.windows.bmp_max_size)) {
        errors.push(`${file}: derivation.windows.bmp_max_size must be one of the derived sizes`);
      }
    }
    if (checkClosed('macos', `${file}: derivation.macos`, derivation.macos, [
      'format', 'max_pixels', 'iconset', 'record',
    ], errors)) {
      if (derivation.macos.format !== 'icns') errors.push(`${file}: derivation.macos.format must be "icns"`);
      if (checkArray(derivation.macos.iconset, `${file}: derivation.macos.iconset`, errors)) {
        const pixels = derivation.macos.iconset.map(entry => entry.pixels);
        if (Math.max(...pixels) !== derivation.macos.max_pixels) {
          errors.push(`${file}: the iconset must provide the ${derivation.macos.max_pixels} px frame`);
        }
        if ([16, 32, 128, 256, 512].some(size => !pixels.includes(size))) {
          errors.push(`${file}: the iconset must cover the 16, 32, 128, 256 and 512 px frames`);
        }
      }
    }
    if (checkClosed('linux', `${file}: derivation.linux`, derivation.linux, [
      'format', 'sizes', 'scalable_svg', 'record',
    ], errors)) {
      if (derivation.linux.format !== 'png') errors.push(`${file}: derivation.linux.format must be "png"`);
      if (!Array.isArray(derivation.linux.sizes)
        || [16, 24, 32, 48, 64, 128, 256, 512].some(size => !derivation.linux.sizes.includes(size))) {
        errors.push(`${file}: derivation.linux.sizes must cover the hicolor set 16 through 512`);
      }
    }
    if (checkClosed('harmonyos', `${file}: derivation.harmonyos`, derivation.harmonyos, [
      'format', 'layered', 'start_window_icon', 'record',
    ], errors)) {
      if (derivation.harmonyos.format !== 'png') errors.push(`${file}: derivation.harmonyos.format must be "png"`);
      if (checkClosed('layered', `${file}: derivation.harmonyos.layered`, derivation.harmonyos.layered, [
        'background', 'foreground', 'note',
      ], errors)) {
        for (const layer of ['background', 'foreground']) {
          const pixels = derivation.harmonyos.layered[layer];
          if (!Number.isInteger(pixels) || pixels <= derivation.small_master_max_pixels) {
            errors.push(`${file}: derivation.harmonyos.layered.${layer} must be a pixel size above the small `
              + 'master, so the layered icon is rendered from the vector master rather than enlarged');
          }
        }
        checkString(derivation.harmonyos.layered.note, `${file}: derivation.harmonyos.layered.note`, errors);
      }
      if (!Number.isInteger(derivation.harmonyos.start_window_icon) || derivation.harmonyos.start_window_icon <= 0) {
        errors.push(`${file}: derivation.harmonyos.start_window_icon must be a positive pixel size`);
      }
    }
  }

  return {
    errors,
    stats: {
      masters: roles.size,
      windowsSizes: derivation?.windows?.sizes?.length ?? 0,
      macosFrames: derivation?.macos?.iconset?.length ?? 0,
      linuxSizes: derivation?.linux?.sizes?.length ?? 0,
      harmonyosFrames: (derivation?.harmonyos?.layered === undefined ? 0 : 2)
        + (derivation?.harmonyos?.start_window_icon === undefined ? 0 : 1),
    },
  };
}

// HarmonyOS owns a directory of manifests rather than a single template, so its checks are the
// cross file ones the other platforms do not need: the declared template list must match the
// files on disk, the device types must be documented values, the icon sizes must agree with the
// branding derivation, a dark resource override must cover exactly the names of the light one,
// and the distribution form must agree with the identity record.
function checkHarmonyos(root, definition, identity, branding, errors) {
  const file = definition.config;
  const { value: config } = checkJsonDocument(readText(root, file), file);
  if (!isPlainObject(config)) return;
  const directory = path.dirname(path.join(root, file));

  if (checkArray(config.device_types, `${file}: device_types`, errors)) {
    for (const type of config.device_types) {
      if (!OHOS_DEVICE_TYPES.includes(type)) {
        errors.push(`${file}: device type "${type}" is not a documented HarmonyOS device type`);
      }
    }
    if (!config.device_types.includes('2in1')) {
      errors.push(`${file}: the desktop packaging definition must declare the "2in1" device type`);
    }
  }
  for (const field of ['app_scope_dir', 'module_dir']) checkString(config[field], `${file}: ${field}`, errors);

  const declared = new Set();
  if (checkArray(config.templates, `${file}: templates`, errors)) {
    for (const [index, entry] of config.templates.entries()) {
      const label = `${file}: templates[${index}]`;
      if (!checkClosed(label, label, entry, ['path', 'role', 'substituted'], errors)) continue;
      checkString(entry.role, `${label}: role`, errors);
      if (typeof entry.substituted !== 'boolean') errors.push(`${label}: substituted must be a boolean`);
      if (!checkString(entry.path, `${label}: path`, errors)) continue;
      declared.add(entry.path);
      const target = path.join(directory, entry.path);
      if (!existsSync(target)) {
        errors.push(`${label}: ${path.posix.join(path.dirname(file), entry.path)} does not exist`);
        continue;
      }
      if (entry.substituted === true && !/@@[A-Z][A-Z0-9_]*@@/.test(readFileSync(target, 'utf8'))) {
        errors.push(`${label}: ${entry.path} declares substitution but carries no token placeholder`);
      }
    }
  }
  for (const present of listFiles(directory).filter(candidate => candidate !== path.basename(file))) {
    if (!declared.has(present)) {
      errors.push(`${file}: ${path.posix.join(path.dirname(file), present)} exists but is not declared in templates`);
    }
  }

  if (checkClosed('version_code', `${file}: version_code`, config.version_code,
    ['segments', 'max_segment', 'max_value', 'record'], errors)) {
    const { segments, max_segment: maxSegment, max_value: maxValue } = config.version_code;
    if (!Array.isArray(segments) || segments.length !== 3
      || !segments.every(segment => Number.isInteger(segment) && segment > 0)
      || !(segments[0] > segments[1] && segments[1] > segments[2])) {
      errors.push(`${file}: version_code.segments must be three strictly descending integer multipliers`);
    } else if (!Number.isInteger(maxSegment) || maxSegment < 1
      || maxSegment * segments.reduce((total, segment) => total + segment, 0) > maxValue) {
      errors.push(`${file}: version_code.max_segment is too large for the declared multipliers and max_value`);
    }
    if (maxValue !== 2147483647) {
      errors.push(`${file}: version_code.max_value must be the documented platform range 2147483647`);
    }
    checkString(config.version_code.record, `${file}: version_code.record`, errors);
  }

  const harmonyos = branding?.derivation?.harmonyos;
  if (checkClosed('icons', `${file}: icons`, config.icons, ['layered', 'start_window_icon', 'install'], errors)) {
    if (checkClosed('layered', `${file}: icons.layered`, config.icons.layered,
      ['background', 'foreground', 'pixels'], errors)) {
      for (const layer of ['background', 'foreground']) {
        const expected = `package/harmonyos/${config.app_scope_dir}/resources/base/media/${layer}.png`;
        if (checkString(config.icons.layered[layer], `${file}: icons.layered.${layer}`, errors)
          && config.icons.layered[layer] !== expected) {
          errors.push(`${file}: icons.layered.${layer} must be "${expected}", which is where the generator writes it`);
        }
        if (harmonyos !== undefined && config.icons.layered.pixels !== harmonyos.layered[layer]) {
          errors.push(`${file}: icons.layered.pixels must match derivation.harmonyos.layered.${layer}`);
        }
      }
    }
    if (checkClosed('start_window_icon', `${file}: icons.start_window_icon`, config.icons.start_window_icon,
      ['path', 'pixels', 'note'], errors)) {
      const expected = `package/harmonyos/${config.module_dir}/resources/base/media/app_icon.png`;
      if (config.icons.start_window_icon.path !== expected) {
        errors.push(`${file}: icons.start_window_icon.path must be "${expected}"`);
      }
      if (harmonyos !== undefined && config.icons.start_window_icon.pixels !== harmonyos.start_window_icon) {
        errors.push(`${file}: icons.start_window_icon.pixels must match derivation.harmonyos.start_window_icon`);
      }
      checkString(config.icons.start_window_icon.note, `${file}: icons.start_window_icon.note`, errors);
    }
    if (checkClosed('install', `${file}: icons.install`, config.icons.install,
      ['app_icon', 'start_window_icon', 'note'], errors)) {
      checkString(config.icons.install.app_icon, `${file}: icons.install.app_icon`, errors);
      checkString(config.icons.install.start_window_icon, `${file}: icons.install.start_window_icon`, errors);
      checkString(config.icons.install.note, `${file}: icons.install.note`, errors);
    }
  }

  if (checkClosed('theme', `${file}: theme`, config.theme, ['dark_override', 'note'], errors)) {
    const override = config.theme.dark_override;
    if (checkString(override, `${file}: theme.dark_override`, errors)) {
      const base = path.posix.join(path.dirname(file), `${config.module_dir}/resources/base/element/color.json`);
      const names = document => new Set((document.color ?? []).map(entry => entry.name));
      const overridePath = path.join(root, path.dirname(file), override);
      const basePath = path.join(root, base);
      if (existsSync(overridePath) && existsSync(basePath)) {
        const overrideNames = names(JSON.parse(readFileSync(overridePath, 'utf8')));
        const baseNames = names(JSON.parse(readFileSync(basePath, 'utf8')));
        for (const name of baseNames) {
          if (!overrideNames.has(name)) {
            errors.push(`${file}: ${override} does not override "${name}", which ${base} defines, `
              + 'so the dark resource would silently fall back to the light value');
          }
        }
        for (const name of overrideNames) {
          if (!baseNames.has(name)) {
            errors.push(`${file}: ${override} defines "${name}", which is not a colour name the module uses`);
          }
        }
      }
    }
    checkString(config.theme.note, `${file}: theme.note`, errors);
  }

  if (checkClosed('host_requirement', `${file}: host_requirement`, config.host_requirement,
    ['arkts_shell', 'native_candidate', 'note'], errors)) {
    for (const field of ['arkts_shell', 'native_candidate', 'note']) {
      checkString(config.host_requirement[field], `${file}: host_requirement.${field}`, errors);
    }
    const shell = config.host_requirement.arkts_shell?.split(' ')[0];
    if (shell !== undefined && shell.includes('/') && existsSync(path.join(root, `packaging/harmonyos/${shell}`))) {
      errors.push(`${file}: host_requirement.arkts_shell names ${shell}, which should be provided by the host `
        + 'application rather than this resource tree');
    }
  }

  if (checkClosed('distribution', `${file}: distribution`, config.distribution, ['form', 'note'], errors)) {
    const form = config.distribution.form;
    if (form !== null && (typeof form !== 'string' || form.trim() === '')) {
      errors.push(`${file}: distribution.form must be null or a non-empty string`);
    }
    if ((form === null) !== (identity.distribution?.harmonyos === null)) {
      errors.push(`${file}: distribution.form and ${'packaging/identity.json'} distribution.harmonyos disagree `
        + 'about whether the distribution form is decided');
    }
    checkString(config.distribution.note, `${file}: distribution.note`, errors);
  }

  if (checkArray(config.sources, `${file}: sources`, errors)) {
    for (const source of config.sources) checkString(source, `${file}: sources entry`, errors);
  } else {
    errors.push(`${file}: sources must list the platform references the definition was written from`);
  }
}

export function checkPackaging(root) {
  const identityFile = 'packaging/identity.json';
  const { errors, value } = checkJsonDocument(readText(root, identityFile), identityFile);
  if (!isPlainObject(value)) return { errors, stats: null };
  checkClosed(identityFile, identityFile, value, [
    'schema_version', 'status', 'binary_name', 'display_name', 'vendor', 'version_source',
    'copyright', 'application_id', 'minimum_os', 'distribution', 'install_prefix', 'tokens', 'decision',
  ], errors);
  if (value.schema_version !== 1) errors.push(`${identityFile}: schema_version must be 1`);
  checkEnum(value.status, `${identityFile}: status`, ['blocked-pending-decision', 'decided'], errors);
  for (const field of ['binary_name', 'display_name', 'vendor', 'copyright']) {
    checkString(value[field], `${identityFile}: ${field}`, errors);
  }
  if (checkClosed('version_source', `${identityFile}: version_source`, value.version_source, ['path', 'key'], errors)) {
    const target = safePathWithin(root, value.version_source.path, `${identityFile}: version_source.path`, errors, {
      allowOutsideBase: true,
    });
    if (target !== null && !existsSync(target)) {
      errors.push(`${identityFile}: version_source.path ${value.version_source.path} does not exist`);
    }
  }
  for (const field of ['minimum_os', 'distribution']) {
    if (checkClosed(field, `${identityFile}: ${field}`, value[field], PLATFORMS, errors)) {
      for (const platform of PLATFORMS) {
        const platformValue = value[field][platform];
        if (platformValue !== null && (typeof platformValue !== 'string' || platformValue.trim() === '')) {
          errors.push(`${identityFile}: ${field}.${platform} must be null or a non-empty string`);
        }
      }
    }
  }
  const undecided = ['application_id', 'install_prefix'];
  for (const field of undecided) {
    const fieldValue = value[field];
    if (fieldValue !== null && (typeof fieldValue !== 'string' || fieldValue.trim() === '')) {
      errors.push(`${identityFile}: ${field} must be null or a non-empty string`);
    }
  }
  if (value.application_id !== null) {
    if (!REVERSE_DNS_PATTERN.test(value.application_id)) {
      errors.push(`${identityFile}: application_id must be a reverse-DNS name the project controls`);
    }
    if (!OHOS_BUNDLE_PATTERN.test(value.application_id) || value.application_id.length < 7
      || value.application_id.length > 128) {
      errors.push(`${identityFile}: application_id must also be a valid HarmonyOS bundle name `
        + '(three or more dot separated segments of letters, digits and underscores, 7 to 128 bytes, no hyphens)');
    }
  }
  const platformUndecided = ['minimum_os', 'distribution']
    .flatMap(field => PLATFORMS.map(platform => value[field]?.[platform] ?? null));
  const hasUndecided = undecided.some(field => value[field] === null)
    || platformUndecided.some(entry => entry === null);
  if (value.status === 'decided' && hasUndecided) {
    errors.push(`${identityFile}: status "decided" requires every identity and distribution value to be filled in`);
  }
  if (value.status === 'blocked-pending-decision' && !hasUndecided) {
    errors.push(`${identityFile}: status "blocked-pending-decision" requires at least one open value`);
  }
  if (checkClosed('decision', `${identityFile}: decision`, value.decision, [
    'tasks', 'reference', 'blocked_reason',
  ], errors)) {
    if (checkArray(value.decision.tasks, `${identityFile}: decision.tasks`, errors)) {
      for (const task of value.decision.tasks) {
        if (typeof task !== 'string' || !/^T\d{2}\.\d{2}$/.test(task)) {
          errors.push(`${identityFile}: decision.tasks entries must be roadmap task ids`);
        }
      }
    }
    if (checkString(value.decision.reference, `${identityFile}: decision.reference`, errors)) {
      const reference = value.decision.reference.split(' ')[0];
      const target = safePathWithin(path.dirname(path.join(root, identityFile)), reference,
        `${identityFile}: decision.reference`, errors, { allowOutsideBase: true });
      if (target !== null && !existsSync(target)) {
        errors.push(`${identityFile}: decision.reference ${reference} does not exist`);
      }
    }
    checkString(value.decision.blocked_reason, `${identityFile}: decision.blocked_reason`, errors);
  }

  const declaredTokens = new Set();
  if (checkArray(value.tokens, `${identityFile}: tokens`, errors)) {
    for (const token of value.tokens) {
      if (typeof token !== 'string' || !/^@@[A-Z][A-Z0-9_]*@@$/.test(token)) {
        errors.push(`${identityFile}: token "${token}" must look like @@NAME@@`);
      }
      if (declaredTokens.has(token)) errors.push(`${identityFile}: duplicate token ${token}`);
      declaredTokens.add(token);
    }
  }

  const platforms = {
    windows: {
      config: 'packaging/windows/app.json',
      fields: ['schema_version', 'platform', 'resource_script', 'icon_file', 'icon_resource_id',
        'app_user_model_id', 'install', 'verification_required', 'verification_environment', 'record'],
      templates: [
        { path: 'packaging/windows/rustage.rc', requiredKeys: ['ICON', 'VERSIONINFO', 'FILEVERSION', 'ProductName'] },
      ],
    },
    macos: {
      config: 'packaging/macos/app.json',
      fields: ['schema_version', 'platform', 'info_plist', 'icon_file', 'iconset_dir', 'install_path',
        'notarization', 'verification_required', 'verification_environment', 'record'],
      templates: [
        {
          path: 'packaging/macos/Info.plist',
          requiredKeys: ['CFBundleIdentifier', 'CFBundleIconFile', 'CFBundleExecutable',
            'CFBundleShortVersionString', 'LSMinimumSystemVersion'],
        },
      ],
    },
    linux: {
      config: 'packaging/linux/app.json',
      fields: ['schema_version', 'platform', 'desktop_entry', 'icon_theme', 'install_layout',
        'cache_refresh', 'verification_required', 'verification_environment', 'record'],
      templates: [
        {
          path: 'packaging/linux/rustage.desktop',
          requiredKeys: ['[Desktop Entry]', 'Type=Application', 'Name=', 'Exec=', 'Icon='],
        },
      ],
    },
    // HarmonyOS is the one platform whose definition is spread over several manifests and
    // resource files, because the platform requires exact paths rather than one template.
    harmonyos: {
      config: 'packaging/harmonyos/app.json',
      fields: ['schema_version', 'platform', 'device_types', 'app_scope_dir', 'module_dir', 'templates',
        'version_code', 'icons', 'theme', 'host_requirement', 'distribution', 'verification_required',
        'verification_environment', 'sources', 'record'],
      templates: [
        {
          path: 'packaging/harmonyos/AppScope/app.json5',
          requiredKeys: ['"bundleName"', '"versionCode"', '"versionName"', '$media:layered_image', '$string:app_name'],
        },
        {
          path: 'packaging/harmonyos/build-profile.json5',
          requiredKeys: ['"compatibleSdkVersion"', '"runtimeOS"', '"signingConfigs"'],
        },
        {
          path: 'packaging/harmonyos/entry/src/main/module.json5',
          requiredKeys: ['"type": "entry"', '"2in1"', '"mainElement"', '$profile:main_pages',
            'startWindowIcon', 'startWindowBackground', 'entity.system.home', 'ohos.want.action.home'],
        },
        {
          path: 'packaging/harmonyos/AppScope/resources/base/element/string.json',
          requiredKeys: ['app_name'],
          json: true,
        },
        {
          path: 'packaging/harmonyos/AppScope/resources/base/media/layered_image.json',
          requiredKeys: ['layered-image', '$media:background', '$media:foreground'],
          json: true,
        },
        {
          path: 'packaging/harmonyos/entry/src/main/resources/base/element/string.json',
          requiredKeys: ['EntryAbility_label'],
          json: true,
        },
        {
          path: 'packaging/harmonyos/entry/src/main/resources/base/element/color.json',
          requiredKeys: ['start_window_background'],
          json: true,
        },
        {
          path: 'packaging/harmonyos/entry/src/main/resources/dark/element/color.json',
          requiredKeys: ['start_window_background'],
          json: true,
        },
        {
          path: 'packaging/harmonyos/entry/src/main/resources/base/profile/main_pages.json',
          requiredKeys: ['"src"'],
          json: true,
        },
      ],
    },
  };

  const usedTokens = new Map();
  for (const [platform, definition] of Object.entries(platforms)) {
    const { errors: configErrors, value: config } = checkJsonDocument(readText(root, definition.config), definition.config);
    errors.push(...configErrors);
    if (isPlainObject(config)) {
      checkClosed(definition.config, definition.config, config, definition.fields, errors);
      if (config.schema_version !== 1) errors.push(`${definition.config}: schema_version must be 1`);
      if (config.platform !== platform) errors.push(`${definition.config}: platform must be "${platform}"`);
      if (checkArray(config.verification_required, `${definition.config}: verification_required`, errors)) {
        for (const check of config.verification_required) {
          checkString(check, `${definition.config}: verification_required entry`, errors);
        }
      }
      checkString(config.record, `${definition.config}: record`, errors);
    }

    for (const template of definition.templates) {
      const text = readText(root, template.path);
      errors.push(...checkTextFormatting(text, template.path));
      for (const key of template.requiredKeys) {
        if (!text.includes(key)) errors.push(`${template.path}: missing required key ${key}`);
      }
      if (template.json === true) {
        errors.push(...checkJsonDocument(text, template.path).errors);
      }
      for (const token of text.match(/@@[A-Z][A-Z0-9_]*@@/g) ?? []) {
        usedTokens.set(token, template.path);
      }
    }
  }

  if (isPlainObject(value)) {
    const brandFile = 'resources/branding/manifest.json';
    if (existsSync(path.join(root, brandFile))) {
      const branding = JSON.parse(readText(root, brandFile));
      checkHarmonyos(root, platforms.harmonyos, value, branding, errors);
    }
  }

  for (const [token, where] of usedTokens) {
    if (!declaredTokens.has(token)) errors.push(`${where}: token ${token} is not declared in ${identityFile}`);
  }
  for (const token of declaredTokens) {
    if (!usedTokens.has(token)) {
      const used = Object.values(platforms).some(definition => readText(root, definition.config).includes(token));
      if (!used) errors.push(`${identityFile}: token ${token} is used by no packaging template or config`);
    }
  }

  return {
    errors,
    stats: {
      status: value.status,
      openDecisions: [
        ...undecided.filter(field => value[field] === null),
        ...PLATFORMS.filter(platform => value.minimum_os?.[platform] === null).map(platform => `minimum_os.${platform}`),
        ...PLATFORMS.filter(platform => value.distribution?.[platform] === null).map(platform => `distribution.${platform}`),
      ],
      tokens: declaredTokens.size,
    },
  };
}
