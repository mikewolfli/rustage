// SPDX-License-Identifier: MIT
//
// The strict SVG subset the custom host glyphs are authored in, plus the conversions the
// dependency's built-in icon contract needs.
//
// The subset exists so that a glyph can be verified and converted without a general SVG
// engine, and so that nothing is dropped by accident: absolute `M`, `L`, `H`, `V`, `C` and `Z`
// only, one command letter per segment (no implicit repetition), explicit fill rule, no
// transform, no gradient, no nested elements. Anything else is an error here rather than
// something a later conversion would silently approximate.
//
// A glyph declares geometry in one of two forms, and they are converted differently:
//
// * a **fill path** is the artwork itself: closed, hole free, and drawn as authored;
// * a **stroke path** is a *centerline*: open, and expanded into outline geometry by
//   `strokeToOutline`, because the dependency's `IconData` carries filled outline contours and
//   nothing else. The expansion is a stated round-cap/round-join offset, not an approximation
//   chosen at render time.
//
// Both forms end up as closed contours, which is what `toUpstreamPathData` emits. Because the
// emitted geometry is polygonal, the dependency's flattening cost equals the declared point
// count exactly, and the budget is measured on that expanded geometry.
//
// Geometry keeps the `24` authoring grid and is converted to the dependency's coordinate
// convention (`x: 0..grid`, `y: -grid..0`) by `y' = y - grid`, which is the transform recorded
// in `docs/plan/resources.md` section 2.2. Sources stay in positive coordinates.

export const GLYPH_GRID = 24;
export const SAFE_AREA_MIN = 2;
export const SAFE_AREA_MAX = 22;

// The authoring stroke of the minimal glyph language (`docs/plan/resources.md` section 5.2).
// One shared width, one cap and one join: the set carries a single optical weight.
export const STROKE_WIDTH = 2;
export const STROKE_CAP = 'round';
export const STROKE_JOIN = 'round';

// Source-level guards for the authoring geometry a glyph declares.
export const MAX_SOURCE_CONTOURS = 64;
export const MAX_SOURCE_POINTS = 1024;

// The dependency's outline limits for one icon (`IconData`, revision `c957fdec`). They are
// checked against the expanded geometry, so what they measure is the real per-icon cost.
export const MAX_OUTLINE_CONTOURS = 64;
export const MAX_OUTLINE_POINTS = 1024;

// A centerline whose inner offset would run further than this from the vertex doubles back on
// itself; that is an authoring error rather than something to approximate.
const MITER_LIMIT = 6;

// Flattening tolerance in grid units for curved centerlines. 0.05 units stays below a quarter
// of a device pixel at the largest logical size (32 px), so the polyline is indistinguishable
// from the curve it replaces.
const FLATTEN_TOLERANCE = 0.05;

// Round joins and caps are approximated by an arc whose sagitta stays under 0.02 units, which
// is what makes the rounded ends and corners smooth at the largest logical size too.
const ARC_STEP = Math.PI / 8;

const MAX_SUBDIVISION_DEPTH = 12;

const ARITY = new Map([
  ['M', 2], ['L', 2], ['H', 1], ['V', 1], ['C', 6], ['Z', 0],
]);

const ALLOWED_ROOT_ATTRIBUTES = new Set(['xmlns', 'viewBox', 'width', 'height', 'fill']);
const STROKE_ATTRIBUTES = ['stroke', 'stroke-width', 'stroke-linecap', 'stroke-linejoin'];
const ALLOWED_PATH_ATTRIBUTES = new Set(['d', 'fill', 'fill-rule', ...STROKE_ATTRIBUTES]);
const TOKEN_PATTERN = /[A-Za-z]|-?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?/g;

function parseAttributes(source) {
  const attributes = new Map();
  const pattern = /([A-Za-z_:][\w:.-]*)\s*=\s*"([^"]*)"/g;
  for (const [, name, value] of source.matchAll(pattern)) attributes.set(name, value);
  return attributes;
}

function splitElements(text) {
  const withoutComments = text.replace(/<!--[\s\S]*?-->/g, '');
  if (/<!|<\?/.test(withoutComments)) {
    return { error: 'declarations, doctypes and processing instructions are not part of the glyph subset' };
  }
  const elements = [];
  for (const [, name, attributes] of withoutComments.matchAll(/<([A-Za-z][\w:-]*)([^<>]*)>/g)) {
    elements.push({ name, attributes: parseAttributes(attributes) });
  }
  if (/<\//.test(withoutComments) && elements.some(element => element.name === 'svg')) {
    const closingNames = [...withoutComments.matchAll(/<\/([A-Za-z][\w:-]*)>/g)].map(match => match[1]);
    if (closingNames.length !== 1 || closingNames[0] !== 'svg') {
      return { error: 'nested or unclosed elements are not part of the glyph subset' };
    }
  } else if (elements.length > 0) {
    return { error: 'the root <svg> element must be closed explicitly' };
  }
  return { elements };
}

export function parsePathData(data, options = {}) {
  const allowOpen = options.allowOpen === true;
  const errors = [];
  const subpaths = [];
  let current = null;
  let currentPoint = null;
  let startPoint = null;
  let pending = null;

  function applySegment(segment) {
    const { command, args } = segment;
    if (current === null) return;
    switch (command) {
      case 'M':
        currentPoint = [args[0], args[1]];
        startPoint = currentPoint;
        break;
      case 'L':
        currentPoint = [args[0], args[1]];
        break;
      case 'H':
        currentPoint = [args[0], currentPoint[1]];
        break;
      case 'V':
        currentPoint = [currentPoint[0], args[0]];
        break;
      case 'C':
        currentPoint = [args[4], args[5]];
        break;
      case 'Z':
        current.closed = true;
        if (current.points.length < 3) errors.push('a subpath closed with Z needs at least three points');
        currentPoint = startPoint;
        break;
      default:
        break;
    }
    if (command === 'M' || command === 'L' || command === 'H' || command === 'V') {
      current.points.push(currentPoint);
    } else if (command === 'C') {
      current.points.push([args[0], args[1]], [args[2], args[3]], currentPoint);
    }
  }

  for (const token of data.match(TOKEN_PATTERN) ?? []) {
    if (/[A-Za-z]/.test(token)) {
      if (pending !== null && pending.remaining !== 0) {
        errors.push(`command ${pending.command} has ${pending.args.length} of ${ARITY.get(pending.command)} arguments`);
      }
      if (!ARITY.has(token)) {
        errors.push(`command ${token} is outside the glyph subset (absolute M, L, H, V, C, Z only)`);
        pending = null;
        continue;
      }
      pending = { command: token, args: [], remaining: ARITY.get(token) };
      if (token === 'M') {
        current = { points: [], segments: [] };
        subpaths.push(current);
        startPoint = null;
      } else if (current === null) {
        errors.push(`command ${token} appears before any M`);
      }
      if (pending.remaining === 0) {
        current.segments.push({ command: token, args: [], start: currentPoint });
        applySegment(pending);
        pending = null;
      }
      continue;
    }

    if (pending === null) {
      errors.push(`number ${token} appears without a pending command (implicit repetition is not allowed)`);
      continue;
    }
    pending.args.push(Number(token));
    pending.remaining -= 1;
    if (pending.remaining !== 0) continue;
    current.segments.push({ command: pending.command, args: pending.args.slice(), start: currentPoint });
    applySegment(pending);
    pending = null;
  }
  if (pending !== null && pending.remaining !== 0) {
    errors.push(`command ${pending.command} has ${pending.args.length} of ${ARITY.get(pending.command)} arguments`);
  }

  for (const [index, subpath] of subpaths.entries()) {
    if (!subpath.closed && !allowOpen) errors.push(`subpath ${index + 1} is not closed with Z`);
    if (subpath.points.length === 0) errors.push(`subpath ${index + 1} has no points`);
  }
  if (subpaths.length === 0) errors.push('the path data declares no subpath');
  return { subpaths, errors };
}

export function measureSubpath(subpath) {
  const bounds = { minX: Infinity, minY: Infinity, maxX: -Infinity, maxY: -Infinity };
  let area = 0;
  const points = subpath.points;
  for (const [x, y] of points) {
    bounds.minX = Math.min(bounds.minX, x);
    bounds.minY = Math.min(bounds.minY, y);
    bounds.maxX = Math.max(bounds.maxX, x);
    bounds.maxY = Math.max(bounds.maxY, y);
  }
  for (let index = 0; index < points.length; index += 1) {
    const [x1, y1] = points[index];
    const [x2, y2] = points[(index + 1) % points.length];
    area += x1 * y2 - x2 * y1;
  }
  return { bounds, signedArea: area / 2 };
}

function subtract(a, b) {
  return [a[0] - b[0], a[1] - b[1]];
}

function add(a, b) {
  return [a[0] + b[0], a[1] + b[1]];
}

function scale(vector, factor) {
  return [vector[0] * factor, vector[1] * factor];
}

// In the authoring coordinate system (`y` grows downwards) this is the 90 degree clockwise
// side of the direction of travel.
function normal(direction) {
  return [-direction[1], direction[0]];
}

function unitDirection(from, to) {
  const delta = subtract(to, from);
  const length = Math.hypot(delta[0], delta[1]);
  return length === 0 ? null : scale(delta, 1 / length);
}

function distanceToSegment(point, from, to) {
  const span = subtract(to, from);
  const lengthSquared = span[0] ** 2 + span[1] ** 2;
  if (lengthSquared === 0) return Math.hypot(point[0] - from[0], point[1] - from[1]);
  const t = Math.max(0, Math.min(1, ((point[0] - from[0]) * span[0] + (point[1] - from[1]) * span[1]) / lengthSquared));
  return Math.hypot(point[0] - (from[0] + t * span[0]), point[1] - (from[1] + t * span[1]));
}

function flattenCubic(from, control1, control2, to, into, depth = 0) {
  const flatness = Math.max(distanceToSegment(control1, from, to), distanceToSegment(control2, from, to));
  if (flatness <= FLATTEN_TOLERANCE || depth >= MAX_SUBDIVISION_DEPTH) {
    into.push(to);
    return;
  }
  const midpoint = (a, b) => [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
  const first = midpoint(from, control1);
  const middle = midpoint(control1, control2);
  const last = midpoint(control2, to);
  const head = midpoint(first, middle);
  const tail = midpoint(middle, last);
  const centre = midpoint(head, tail);
  flattenCubic(from, first, head, centre, into, depth + 1);
  flattenCubic(centre, tail, last, to, into, depth + 1);
}

export function flattenSubpath(subpath) {
  const points = [];
  let cursor = null;
  for (const segment of subpath.segments) {
    const { command, args } = segment;
    if (command === 'M') {
      cursor = [args[0], args[1]];
      points.push(cursor);
    } else if (command === 'L') {
      cursor = [args[0], args[1]];
      points.push(cursor);
    } else if (command === 'H' && cursor !== null) {
      cursor = [args[0], cursor[1]];
      points.push(cursor);
    } else if (command === 'V' && cursor !== null) {
      cursor = [cursor[0], args[0]];
      points.push(cursor);
    } else if (command === 'C' && cursor !== null) {
      const to = [args[4], args[5]];
      flattenCubic(cursor, [args[0], args[1]], [args[2], args[3]], to, points);
      cursor = to;
    } else if (command === 'Z' && points.length > 0) {
      points.push(points[0]);
    }
  }
  const cleaned = [];
  for (const point of points) {
    const previous = cleaned[cleaned.length - 1];
    if (previous !== undefined && Math.abs(previous[0] - point[0]) < 1e-9 && Math.abs(previous[1] - point[1]) < 1e-9) {
      continue;
    }
    cleaned.push(point);
  }
  return cleaned;
}

// Offsets a stroked centerline into one closed, hole-free outline contour:
//
//   * both sides are offset by half the stroke width;
//   * the outside of every turn gets a round arc, the inside gets the miter point, which is
//     exactly where the two inner offset lines meet;
//   * both ends get a round cap.
//
// Because a single open centerline can only produce a single simply connected region, the
// contour never carries a hole, which is the property the dependency's per-contour fill needs.
// The contour is oriented so that its signed area is positive, so every contour of a glyph
// shares one winding direction.
export function strokeToOutline(subpath, width = STROKE_WIDTH) {
  const errors = [];
  const half = width / 2;
  const points = flattenSubpath(subpath);
  if (points.length < 2) {
    return { contours: [], errors: ['a stroked centerline needs at least two distinct points'] };
  }
  const directions = [];
  for (let index = 0; index < points.length - 1; index += 1) {
    const step = unitDirection(points[index], points[index + 1]);
    if (step === null) {
      return { contours: [], errors: ['a stroked centerline repeats a point'] };
    }
    directions.push(step);
  }
  const sides = directions.map(step => normal(step));
  const shifted = (point, side) => add(point, scale(side, half));
  const arc = (centre, startAngle, sweep) => {
    const steps = Math.max(2, Math.ceil(Math.abs(sweep) / ARC_STEP));
    const generated = [];
    for (let step = 1; step <= steps; step += 1) {
      const angle = startAngle + (sweep * step) / steps;
      generated.push(add(centre, scale([Math.cos(angle), Math.sin(angle)], half)));
    }
    return generated;
  };

  const sideA = [shifted(points[0], sides[0])];
  const sideB = [shifted(points[0], scale(sides[0], -1))];
  for (let index = 1; index < points.length - 1; index += 1) {
    const before = directions[index - 1];
    const after = directions[index];
    const cross = before[0] * after[1] - before[1] * after[0];
    const dot = before[0] * after[0] + before[1] * after[1];
    if (Math.abs(cross) < 1e-9) {
      sideA.push(shifted(points[index], sides[index - 1]));
      sideB.push(shifted(points[index], scale(sides[index - 1], -1)));
      continue;
    }
    const turn = Math.acos(Math.max(-1, Math.min(1, dot)));
    const miter = half / Math.cos(turn / 2);
    if (!Number.isFinite(miter) || miter > MITER_LIMIT) {
      errors.push(`the centerline turns back on itself at (${points[index][0]}, ${points[index][1]})`);
      return { contours: [], errors };
    }
    const first = sides[index - 1];
    const second = sides[index];
    const bisector = scale(add(first, second), 1 / Math.hypot(first[0] + second[0], first[1] + second[1]));
    const sweep = cross > 0 ? turn : -turn;
    // The outside of a turn is the left hand side of travel. `normal` is the 90 degree
    // clockwise side, so the outside is `-normal` for a clockwise turn (`cross > 0`) and
    // `normal` for a counter-clockwise one. The inside keeps the miter point, which is where the
    // two inner offset lines meet; that is the sharp inner corner a round join is expected to
    // leave alone.
    const outerSide = cross > 0 ? scale(first, -1) : first;
    const outerArc = arc(points[index], Math.atan2(outerSide[1], outerSide[0]), sweep);
    const innerMiter = add(points[index], scale(bisector, cross > 0 ? miter : -miter));
    if (cross > 0) {
      sideA.push(innerMiter);
      sideB.push(...outerArc);
    } else {
      sideA.push(...outerArc);
      sideB.push(innerMiter);
    }
  }
  sideA.push(shifted(points[points.length - 1], sides[directions.length - 1]));
  sideB.push(shifted(points[points.length - 1], scale(sides[directions.length - 1], -1)));

  const last = points[points.length - 1];
  const endCap = arc(last, Math.atan2(sides[directions.length - 1][1], sides[directions.length - 1][0]), -Math.PI);
  const back = sides[0];
  const startCap = arc(points[0], Math.atan2(-back[1], -back[0]), -Math.PI);

  const raw = [...sideA, ...endCap, ...sideB.slice().reverse(), ...startCap];
  const contour = [];
  for (const point of raw) {
    const previous = contour[contour.length - 1];
    if (previous !== undefined && Math.abs(previous[0] - point[0]) < 1e-9 && Math.abs(previous[1] - point[1]) < 1e-9) {
      continue;
    }
    contour.push(point);
  }
  const firstPoint = contour[0];
  const finalPoint = contour[contour.length - 1];
  if (contour.length > 1 && Math.abs(firstPoint[0] - finalPoint[0]) < 1e-9
    && Math.abs(firstPoint[1] - finalPoint[1]) < 1e-9) {
    contour.pop();
  }
  if (polygonArea(contour) < 0) contour.reverse();
  return { contours: [contour], errors };
}

function polygonArea(points) {
  let sum = 0;
  for (let index = 0; index < points.length; index += 1) {
    const [x1, y1] = points[index];
    const [x2, y2] = points[(index + 1) % points.length];
    sum += x1 * y2 - x2 * y1;
  }
  return sum / 2;
}

// Wraps a flattened polyline as the same segment shape `parsePathData` produces, so the
// downstream conversion and measurement code treats authored, flattened and expanded contours
// alike. Every emitted contour is polygonal, which is what makes the point budget exact.
function contourToSubpath(points) {
  const contour = points.slice();
  if (contour.length > 1
    && Math.abs(contour[0][0] - contour[contour.length - 1][0]) < 1e-9
    && Math.abs(contour[0][1] - contour[contour.length - 1][1]) < 1e-9) {
    contour.pop();
  }
  const segments = [{ command: 'M', args: [contour[0][0], contour[0][1]], start: null }];
  let cursor = contour[0];
  for (const point of contour.slice(1)) {
    segments.push({ command: 'L', args: [point[0], point[1]], start: cursor });
    cursor = point;
  }
  segments.push({ command: 'Z', args: [], start: cursor });
  return { points: contour, segments, closed: true };
}

// A fill contour is flattened with the same tolerance as a stroke centerline, so the geometry
// that reaches the renderer never carries a curve the budget cannot count.
function flattenFillSubpath(subpath) {
  const points = flattenSubpath(subpath);
  return points.length >= 3 ? contourToSubpath(points) : null;
}

export function validateGlyphSvg(text, file) {
  const errors = [];
  const { elements, error } = splitElements(text);
  if (error) return { errors: [`${file}: ${error}`], data: null };
  if (elements.length === 0) return { errors: [`${file}: no <svg> element found`], data: null };

  const [root, ...children] = elements;
  if (root.name !== 'svg') errors.push(`${file}: the root element must be <svg>`);
  for (const [name, value] of root.attributes) {
    if (!ALLOWED_ROOT_ATTRIBUTES.has(name)) errors.push(`${file}: <svg> attribute ${name} is not allowed`);
    if (name === 'viewBox' && value !== `0 0 ${GLYPH_GRID} ${GLYPH_GRID}`) {
      errors.push(`${file}: viewBox must be "0 0 ${GLYPH_GRID} ${GLYPH_GRID}"`);
    }
    if ((name === 'width' || name === 'height') && value !== `${GLYPH_GRID}`) {
      errors.push(`${file}: ${name} must be "${GLYPH_GRID}"`);
    }
  }
  for (const attribute of ['xmlns', 'viewBox', 'width', 'height']) {
    if (!root.attributes.has(attribute)) errors.push(`${file}: <svg> is missing ${attribute}`);
  }
  const rootFill = root.attributes.get('fill');
  if (rootFill !== undefined && rootFill !== 'currentColor') {
    errors.push(`${file}: <svg> fill must be omitted or currentColor so the theme supplies the colour`);
  }

  const paths = children.filter(element => element.name === 'path');
  if (paths.length === 0) errors.push(`${file}: no <path> element found`);
  for (const child of children) {
    if (child.name !== 'path') errors.push(`${file}: element <${child.name}> is not part of the glyph subset`);
  }

  const declared = [];
  const stroked = [];
  for (const [index, element] of paths.entries()) {
    const label = `${file}: path ${index + 1}`;
    for (const [name] of element.attributes) {
      if (!ALLOWED_PATH_ATTRIBUTES.has(name)) errors.push(`${label}: attribute ${name} is not allowed`);
    }
    const isStroke = STROKE_ATTRIBUTES.some(name => element.attributes.has(name));
    if (isStroke) {
      if (element.attributes.get('stroke') !== 'currentColor') {
        errors.push(`${label}: stroke must be currentColor so the theme supplies the colour`);
      }
      if (element.attributes.get('stroke-width') !== `${STROKE_WIDTH}`) {
        errors.push(`${label}: stroke-width must be "${STROKE_WIDTH}"; the set carries one optical weight`);
      }
      if (element.attributes.get('stroke-linecap') !== STROKE_CAP) {
        errors.push(`${label}: stroke-linecap must be "${STROKE_CAP}"`);
      }
      if (element.attributes.get('stroke-linejoin') !== STROKE_JOIN) {
        errors.push(`${label}: stroke-linejoin must be "${STROKE_JOIN}"`);
      }
      if (element.attributes.get('fill') !== 'none') {
        errors.push(`${label}: a stroked path must declare fill="none"; the outline is expanded from the stroke`);
      }
      if (element.attributes.has('fill-rule')) {
        errors.push(`${label}: fill-rule does not apply to a stroked path`);
      }
    } else {
      const fill = element.attributes.get('fill');
      if (fill !== undefined && fill !== 'currentColor') {
        errors.push(`${label}: fill must be omitted or currentColor`);
      }
      if (element.attributes.get('fill-rule') !== 'nonzero') {
        errors.push(`${label}: fill-rule="nonzero" must be declared explicitly`);
      }
    }
    const data = element.attributes.get('d');
    if (data === undefined) {
      errors.push(`${label}: missing d attribute`);
      continue;
    }
    const parsed = parsePathData(data, { allowOpen: isStroke });
    errors.push(...parsed.errors.map(message => `${label}: ${message}`));
    for (const subpath of parsed.subpaths) {
      if (isStroke && subpath.closed) {
        errors.push(`${label}: a stroked centerline must stay open; a closed loop would expand to a ring with a hole`);
      }
    }
    declared.push(...parsed.subpaths);
    if (isStroke) stroked.push(...parsed.subpaths);
  }

  if (declared.length > MAX_SOURCE_CONTOURS) {
    errors.push(`${file}: ${declared.length} subpaths exceed the ${MAX_SOURCE_CONTOURS} contour budget`);
  }
  const points = declared.reduce((total, subpath) => total + subpath.points.length, 0);
  if (points > MAX_SOURCE_POINTS) {
    errors.push(`${file}: ${points} declared points exceed the ${MAX_SOURCE_POINTS} point budget`);
  }

  // Everything below measures the geometry the renderer receives: authored fill contours, now
  // flattened, plus the outlines expanded from the stroked centerlines.
  const subpaths = declared
    .filter(subpath => !stroked.includes(subpath))
    .map(flattenFillSubpath)
    .filter(subpath => subpath !== null);
  for (const [index, subpath] of stroked.entries()) {
    const expanded = strokeToOutline(subpath, STROKE_WIDTH);
    errors.push(...expanded.errors.map(message => `${file}: stroked centerline ${index + 1}: ${message}`));
    subpaths.push(...expanded.contours.map(contourToSubpath));
  }
  if (subpaths.length > MAX_OUTLINE_CONTOURS) {
    errors.push(`${file}: the outline has ${subpaths.length} contours, above the ${MAX_OUTLINE_CONTOURS} limit`);
  }
  const outlinePoints = subpaths.reduce((total, subpath) => total + subpath.points.length, 0);
  if (outlinePoints > MAX_OUTLINE_POINTS) {
    errors.push(`${file}: the outline has ${outlinePoints} points, above the ${MAX_OUTLINE_POINTS} limit`);
  }

  let bounds = { minX: Infinity, minY: Infinity, maxX: -Infinity, maxY: -Infinity };
  let areaSign = 0;
  for (const [index, subpath] of subpaths.entries()) {
    const { bounds: subpathBounds, signedArea } = measureSubpath(subpath);
    bounds = {
      minX: Math.min(bounds.minX, subpathBounds.minX),
      minY: Math.min(bounds.minY, subpathBounds.minY),
      maxX: Math.max(bounds.maxX, subpathBounds.maxX),
      maxY: Math.max(bounds.maxY, subpathBounds.maxY),
    };
    for (const [x, y] of subpath.points) {
      if (!Number.isFinite(x) || !Number.isFinite(y)) {
        errors.push(`${file}: subpath ${index + 1} has a non-finite coordinate`);
      } else if (x < SAFE_AREA_MIN || x > SAFE_AREA_MAX || y < SAFE_AREA_MIN || y > SAFE_AREA_MAX) {
        errors.push(
          `${file}: subpath ${index + 1} leaves the ${SAFE_AREA_MIN}..${SAFE_AREA_MAX} safe area at (${x}, ${y})`,
        );
      }
    }
    if (signedArea === 0) errors.push(`${file}: subpath ${index + 1} has zero area`);
    const sign = Math.sign(signedArea);
    if (sign !== 0) {
      if (areaSign !== 0 && sign !== areaSign) {
        errors.push(
          `${file}: subpath ${index + 1} winds opposite to an earlier subpath; a nonzero fill would carve a hole`,
        );
      }
      areaSign = sign;
    }
  }
  if (bounds.maxX <= bounds.minX || bounds.maxY <= bounds.minY) {
    errors.push(`${file}: the artwork has no extent`);
  }

  return {
    errors,
    data: {
      subpaths,
      bounds,
      declaredPoints: points,
      declaredContours: declared.length,
      contours: subpaths.length,
      outlinePoints,
      strokedContours: stroked.length,
    },
  };
}

function formatNumber(value) {
  const rounded = Math.round(value * 1000) / 1000;
  return `${Object.is(rounded, -0) ? 0 : rounded}`;
}

// Emits contours as authoring path data with the placement baked into the coordinates, so a
// preview rasteriser draws the exact geometry at the exact size with no transform to interpret
// and no resampling of a smaller render. The review sheets use this instead of the source, so
// what gets looked at is what the renderer receives.
export function toAuthoringPathData(subpaths, { offset = [0, 0], scale = 1 } = {}) {
  const place = (value, axis) => formatNumber(value * scale + offset[axis]);
  return subpaths.map(subpath => subpath.segments.map((segment) => {
    const { command, args } = segment;
    if (command === 'Z') return 'Z';
    if (command === 'H') return `L${place(args[0], 0)} ${place(segment.start[1], 1)}`;
    if (command === 'V') return `L${place(segment.start[0], 0)} ${place(args[0], 1)}`;
    if (command === 'C') {
      return `C${place(args[0], 0)} ${place(args[1], 1)} ${place(args[2], 0)} ${place(args[3], 1)} `
        + `${place(args[4], 0)} ${place(args[5], 1)}`;
    }
    return `${command}${place(args[0], 0)} ${place(args[1], 1)}`;
  }).join('')).join('');
}

// Converts authoring coordinates to the dependency's icon grid: `y' = y - grid` keeps the
// artwork inside `y: -grid..0` while `x` stays positive, which is what the built-in outline
// table uses. `H` and `V` are expanded to `L`, so the emitted path data is explicit about both
// coordinates and cannot be read as a command with the wrong argument count.
export function toUpstreamPathData(subpaths, grid = GLYPH_GRID) {
  const parts = [];
  for (const subpath of subpaths) {
    const commands = [];
    for (const segment of subpath.segments) {
      const values = segment.args.map(value => value);
      if (segment.command === 'M' || segment.command === 'L'
        || segment.command === 'H' || segment.command === 'V') {
        const [x, y] = segment.command === 'H'
          ? [values[0], segment.start?.[1]]
          : segment.command === 'V'
            ? [segment.start?.[0], values[0]]
            : values;
        const command = segment.command === 'H' || segment.command === 'V' ? 'L' : segment.command;
        commands.push(`${command}${formatNumber(x)} ${formatNumber(y - grid)}`);
      } else if (segment.command === 'C') {
        commands.push(
          `C${formatNumber(values[0])} ${formatNumber(values[1] - grid)} `
          + `${formatNumber(values[2])} ${formatNumber(values[3] - grid)} `
          + `${formatNumber(values[4])} ${formatNumber(values[5] - grid)}`,
        );
      } else if (segment.command === 'Z') {
        commands.push('Z');
      }
    }
    parts.push(commands.join(''));
  }
  return `${parts.join('')}`;
}
