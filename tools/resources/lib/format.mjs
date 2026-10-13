// SPDX-License-Identifier: MIT
//
// Text hygiene for the resource tree. `tools/check-docs.mjs` already covers Markdown, `.mjs`
// and workflow YAML; this covers the resource formats it does not reach (JSON, SVG, Windows
// resource scripts, plists, desktop entries) so the whole tree keeps the same line-ending,
// indentation and trailing-whitespace rules.

const TEXT_EXTENSIONS = ['.json', '.svg', '.rc', '.plist', '.desktop'];

export function isResourceText(file) {
  return TEXT_EXTENSIONS.some(extension => file.endsWith(extension));
}

export function checkTextFormatting(text, file) {
  const errors = [];
  text.split('\n').forEach((line, index) => {
    if (/[ \t]+$/.test(line)) errors.push(`${file}:${index + 1}: trailing whitespace`);
    if (/^\t| \t/.test(line)) errors.push(`${file}:${index + 1}: tab character; the tree indents with spaces`);
  });
  if (text.includes('\r')) errors.push(`${file}: uses CRLF; the tree uses LF`);
  if (!text.endsWith('\n')) errors.push(`${file}: missing final newline`);
  if (text.endsWith('\n\n')) errors.push(`${file}: ends with a blank line`);
  return errors;
}

export function checkJsonDocument(text, file) {
  const errors = checkTextFormatting(text, file);
  try {
    return { errors, value: JSON.parse(text) };
  } catch (error) {
    errors.push(`${file}: invalid JSON (${error.message})`);
    return { errors, value: null };
  }
}

export function isPlainObject(value) {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export function checkClosedObject(value, allowed, file) {
  const errors = [];
  if (!isPlainObject(value)) {
    errors.push(`${file}: expected a JSON object`);
    return errors;
  }
  for (const key of Object.keys(value)) {
    if (!allowed.includes(key)) errors.push(`${file}: unknown field "${key}"`);
  }
  for (const key of allowed) {
    if (!(key in value)) errors.push(`${file}: missing field "${key}"`);
  }
  return errors;
}

export function checkNonEmptyString(value, file, field) {
  return typeof value === 'string' && value.trim() !== '' ? [] : [`${file}: field "${field}" must be a non-empty string`];
}
