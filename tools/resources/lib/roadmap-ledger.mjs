// SPDX-License-Identifier: MIT
//
// The roadmap is the single source of truth for which command groups and panels exist. The
// host icon manifest must carry an explicit icon or no-icon decision for every one of them, so
// the ledger is read here instead of being copied into the manifest as a second list that could
// drift. A roadmap table that stops parsing is a hard error rather than an empty ledger.

import { readFileSync } from 'node:fs';
import path from 'node:path';

function tableRows(text, startHeading, endHeading) {
  const start = text.indexOf(startHeading);
  const end = text.indexOf(endHeading);
  if (start < 0 || end < 0 || end <= start) return null;
  const rows = [];
  for (const line of text.slice(start, end).split('\n')) {
    if (!line.startsWith('|')) continue;
    const cells = line.split('|').slice(1, -1).map(cell => cell.trim());
    if (cells.length < 2) continue;
    rows.push(cells);
  }
  return rows;
}

const GROUP_CELL_PATTERN = /^[a-z][a-z0-9]*(?:\.[a-z0-9]+)*\.\*$/;

export function roadmapCommandGroups(text) {
  const rows = tableRows(text, '### 3.2 ', '### 3.3 ');
  if (rows === null) return null;
  // The column position is not assumed; the group column is the one holding `domain.action.*`.
  const groups = rows
    .map(cells => cells.find(cell => GROUP_CELL_PATTERN.test(cell)))
    .filter(cell => cell !== undefined);
  return groups.length > 0 ? groups : null;
}

export function roadmapPanelIds(text) {
  const rows = tableRows(text, '### 2.2 ', '### 2.3 ');
  if (rows === null) return null;
  const panels = rows
    .map(cells => cells[0])
    .filter(cell => typeof cell === 'string' && cell.startsWith('panel.'))
    .map(cell => cell.replace(/（[^）]*）\s*$/, '').trim());
  return panels.length > 0 ? panels : null;
}

export function readRoadmapLedger(root) {
  const file = 'docs/plan/roadmap.md';
  const text = readFileSync(path.join(root, file), 'utf8');
  const commandGroups = roadmapCommandGroups(text);
  const panels = roadmapPanelIds(text);
  const errors = [];
  if (commandGroups === null) errors.push(`${file}: could not read the command-group ledger of section 3.2`);
  if (panels === null) errors.push(`${file}: could not read the panel ledger of section 2.2`);
  return {
    commandGroups: commandGroups === null ? null : new Set(commandGroups),
    panels: panels === null ? null : new Set(panels),
    errors,
  };
}
