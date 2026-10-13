// SPDX-License-Identifier: MIT
//
// Safety checks for the SVGs rustage keeps as sources.
//
// Two consumers, two shapes:
//
// * Branding masters may be multi-colour and may use gradients, because they are rasterised
//   for platform packaging instead of being converted into icon geometry. They are checked
//   against an explicit element and attribute allowlist.
// * Glyph sources are validated against the converter subset in `svg-subset.mjs`, which is
//   itself an allowlist; they only call the shared hygiene checks below.
//
// The hygiene rules mirror the untrusted-input limits in `docs/plan/resources.md` section 15.1:
// nothing in a resource source may reach outside its own file, and every source carries an
// SPDX identifier so the licence chain is auditable.

export function checkSvgHygiene(text, file) {
  const errors = [];
  if (!/SPDX-License-Identifier:\s*\S+/.test(text)) {
    errors.push(`${file}: missing an SPDX-License-Identifier comment`);
  }
  if (/<!DOCTYPE|<\?xml-stylesheet/i.test(text)) {
    errors.push(`${file}: doctypes and stylesheet processing instructions are not allowed`);
  }
  const withoutComments = text.replace(/<!--[\s\S]*?-->/g, '');
  const definedIds = new Set([...withoutComments.matchAll(/\sid\s*=\s*"([^"]+)"/g)].map(match => match[1]));
  for (const [, value] of withoutComments.matchAll(/url\(\s*([^)]*?)\s*\)/g)) {
    const reference = value.replace(/^['"]|['"]$/g, '');
    if (!reference.startsWith('#')) {
      errors.push(`${file}: url(${reference}) is an external reference`);
    } else if (!definedIds.has(reference.slice(1))) {
      errors.push(`${file}: url(${reference}) points at no definition in this file`);
    }
  }
  return errors;
}

export function checkBrandingSvg(text, file, { allowedElements, allowedAttributes }) {
  const errors = checkSvgHygiene(text, file);
  const withoutComments = text.replace(/<!--[\s\S]*?-->/g, '');
  for (const [, name] of withoutComments.matchAll(/<([A-Za-z][\w:-]*)/g)) {
    if (name === 'svg') continue;
    if (!allowedElements.includes(name)) {
      errors.push(`${file}: element <${name}> is outside the allowed branding source subset`);
    }
  }
  const allowed = new Set(['xmlns', ...allowedAttributes]);
  for (const [, name] of withoutComments.matchAll(/\s([A-Za-z_:][\w:.-]*)\s*=\s*"/g)) {
    if (!allowed.has(name)) {
      errors.push(`${file}: attribute ${name} is outside the allowed branding source subset`);
    }
  }
  const root = text.match(/<svg\b[^>]*>/);
  if (root === null) {
    errors.push(`${file}: no <svg> root element`);
    return errors;
  }
  for (const attribute of ['viewBox', 'width', 'height']) {
    if (!new RegExp(`\\s${attribute}\\s*=\\s*"`).test(root[0])) {
      errors.push(`${file}: <svg> is missing ${attribute}`);
    }
  }
  return errors;
}
