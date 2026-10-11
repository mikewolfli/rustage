import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const requiredFiles = [
  'LICENSE', 'README.md', 'README.zh-CN.md', 'CONTRIBUTING.md',
  'CODE_OF_CONDUCT.md', 'SECURITY.md', 'SUPPORT.md', 'CHANGELOG.md',
  '.gitignore', '.gitattributes', '.editorconfig', '.github/CODEOWNERS',
  '.github/PULL_REQUEST_TEMPLATE.md', '.github/ISSUE_TEMPLATE/config.yml',
  '.github/ISSUE_TEMPLATE/bug_report.yml',
  '.github/ISSUE_TEMPLATE/feature_request.yml',
  '.github/ISSUE_TEMPLATE/design_decision.yml',
  '.github/ISSUE_TEMPLATE/target_compatibility.yml',
  '.github/workflows/docs.yml', 'docs/README.md', 'docs/github.md',
  'docs/plan/principle.md', 'docs/plan/whitepaper.md', 'docs/plan/roadmap.md',
  'tools/check-docs.mjs', 'tools/check-docs.test.mjs',
];

export function checkText(text, file) {
  const errors = [];
  text.split('\n').forEach((line, index) => {
    if (/[ \t]+$/.test(line)) {
      errors.push(`${file}:${index + 1}: trailing whitespace`);
    }
    if (/^(?:<{7}(?: |$)|={7}$|>{7}(?: |$))/.test(line)) {
      errors.push(`${file}:${index + 1}: unresolved conflict marker`);
    }
  });
  if (!text.endsWith('\n')) errors.push(`${file}: missing final newline`);
  if (text.includes('\r')) errors.push(`${file}: use LF line endings`);
  return errors;
}

export function checkLinks(text, file, root, exists = existsSync) {
  const errors = [];
  const withoutFences = text.replace(/^(`{3,}|~{3,})[^\n]*\n[\s\S]*?^\1[ \t]*$/gm, '');
  const links = withoutFences.matchAll(/\[[^\]\n]*\]\((<[^>\n]+>|[^\s)]+)(?:\s+"[^"\n]*")?\)/g);
  for (const match of links) {
    const link = match[1].replace(/^<|>$/g, '');
    if (/^(?:[a-z][a-z\d+.-]*:|\/\/|#)/i.test(link)) continue;
    let relative;
    try {
      relative = decodeURIComponent(link.split(/[?#]/, 1)[0]);
    } catch (error) {
      if (!(error instanceof URIError)) throw error;
      errors.push(`${file}: invalid link encoding: ${link}`);
      continue;
    }
    const target = path.resolve(root, path.dirname(file), relative);
    const fromRoot = path.relative(root, target);
    if (path.isAbsolute(relative) || fromRoot === '..'
      || fromRoot.startsWith(`..${path.sep}`)) {
      errors.push(`${file}: link leaves the repository: ${link}`);
    } else if (!exists(target)) {
      errors.push(`${file}: missing link target: ${link}`);
    }
  }
  return errors;
}

export function checkRoadmap(text) {
  const errors = [];
  const tasks = new Map();
  for (const line of text.split('\n')) {
    if (!/^\| T\d{2}\.\d{2} \|/.test(line)) continue;
    const columns = line.split('|').map(value => value.trim());
    const id = columns[1];
    if (columns.length !== 7) errors.push(`roadmap: malformed task row: ${id}`);
    if (tasks.has(id)) errors.push(`roadmap: duplicate task: ${id}`);
    tasks.set(id, (columns[3] ?? '').match(/T\d{2}\.\d{2}/g) ?? []);
  }
  if (tasks.size === 0) errors.push('roadmap: no implementation tasks found');
  for (const [id, dependencies] of tasks) {
    for (const dependency of dependencies) {
      if (!tasks.has(dependency)) errors.push(`roadmap: ${id} requires missing ${dependency}`);
    }
  }
  const visiting = new Set();
  const visited = new Set();
  function visit(id) {
    if (visiting.has(id)) {
      errors.push(`roadmap: dependency cycle at ${id}`);
      return;
    }
    if (visited.has(id)) return;
    visiting.add(id);
    for (const dependency of tasks.get(id) ?? []) visit(dependency);
    visiting.delete(id);
    visited.add(id);
  }
  for (const id of tasks.keys()) visit(id);
  const domains = new Set([...tasks.keys()].map(id => id.slice(1, 3)));
  const totals = text.match(/共 \*\*(\d+) 个功能域、(\d+) 项实施任务、(\d+) 个里程碑、(\d+) 条用户旅程\*\*/);
  const milestones = [...text.matchAll(/^\| (MS\d{2}) [^|\n]*\|([^|\n]+)\|([^|\n]+)\|/gm)];
  if (totals) {
    const actual = [
      domains.size,
      tasks.size,
      new Set(milestones.map(match => match[1])).size,
      new Set([...text.matchAll(/^\| (J\d{2}) /gm)].map(match => match[1])).size,
    ];
    actual.forEach((count, index) => {
      if (count !== Number(totals[index + 1])) {
        errors.push(`roadmap: declared total ${index + 1} is ${totals[index + 1]}, found ${count}`);
      }
    });
  }
  if (milestones.length > 0) {
    const phases = new Map();
    const future = [...text.matchAll(/^\| (FUTURE) [^|\n]*\|([^|\n]+)\|/gm)];
    const entries = new Map(milestones.map(([, id, , entry]) => {
      const dependencies = new Set();
      for (const [range] of entry.matchAll(/MS\d{2}(?:[–-]MS\d{2})?/g)) {
        const [start, end = start] = range.split(/[–-]/);
        for (let index = Number(start.slice(2)); index <= Number(end.slice(2)); index += 1) {
          dependencies.add(`MS${String(index).padStart(2, '0')}`);
        }
      }
      return [id, dependencies];
    }));
    function dependsOn(phase, required, visited = new Set()) {
      if (phase === required) return true;
      if (visited.has(phase)) return false;
      visited.add(phase);
      return [...(entries.get(phase) ?? [])]
        .some(dependency => dependsOn(dependency, required, visited));
    }
    for (const [, milestone, scope] of [...milestones, ...future]) {
      const references = scope.matchAll(/\b([FT])(\d{2})(?:\.(\d{2}))?(?:[–-]([FT])(\d{2})(?:\.(\d{2}))?)?/g);
      for (const [, kind, domain, task, endKind, endDomain, endTask] of references) {
        if ((kind === 'F' && task)
          || (endKind && (kind !== endKind || Boolean(task) !== Boolean(endTask)))) {
          errors.push(`roadmap: invalid task scope in ${milestone}`);
          continue;
        }
        const start = Number(domain) * 100 + Number(task ?? 0);
        const end = Number(endDomain ?? domain) * 100 + Number(endTask ?? task ?? 99);
        const matches = [...tasks.keys()].filter(id => {
          const value = Number(id.slice(1, 3)) * 100 + Number(id.slice(4));
          return value >= start && value <= end;
        });
        const knownStart = kind === 'T' ? tasks.has(`T${domain}.${task}`) : domains.has(domain);
        const knownEnd = kind === 'T'
          ? tasks.has(`T${endDomain ?? domain}.${endTask ?? task}`)
          : domains.has(endDomain ?? domain);
        if (start > end || matches.length === 0 || !knownStart || !knownEnd) {
          errors.push(`roadmap: unknown or invalid task scope in ${milestone}`);
          continue;
        }
        for (const id of matches) {
          if (milestone === 'FUTURE' && phases.has(id) && phases.get(id) !== 'FUTURE') {
            errors.push(`roadmap: ${id} is both release scope and future scope`);
          }
          if (!phases.has(id)) phases.set(id, milestone);
        }
      }
    }
    const releaseTotals = text.match(/正式版范围为 (\d+) 个功能域、(\d+) 项任务、(\d+) 条旅程/);
    if (releaseTotals) {
      const releaseTasks = [...tasks.keys()].filter(id => phases.has(id) && phases.get(id) !== 'FUTURE');
      const releaseJourneys = [...text.matchAll(/^\| (J\d{2}) [^|\n]*\|[^|\n]*\|[^|\n]*\|([^|\n]+)\|/gm)]
        .filter(match => !match[2].includes('FUTURE'));
      const actual = [
        new Set(releaseTasks.map(id => id.slice(1, 3))).size,
        releaseTasks.length,
        new Set(releaseJourneys.map(match => match[1])).size,
      ];
      actual.forEach((count, index) => {
        if (count !== Number(releaseTotals[index + 1])) {
          errors.push(`roadmap: declared release total ${index + 1} is ${releaseTotals[index + 1]}, found ${count}`);
        }
      });
    }
    for (const [id, dependencies] of tasks) {
      const phase = phases.get(id);
      if (!phase) errors.push(`roadmap: ${id} has no milestone assignment`);
      for (const dependency of dependencies) {
        const requiredPhase = phases.get(dependency);
        if (phase && phase !== 'FUTURE' && requiredPhase === 'FUTURE') {
          errors.push(`roadmap: ${id} in ${phase} requires future task ${dependency}`);
        } else if (phase === 'FUTURE') {
          continue;
        } else if (phase && requiredPhase && requiredPhase > phase) {
          errors.push(`roadmap: ${id} in ${phase} requires later ${dependency} in ${requiredPhase}`);
        } else if (phase && requiredPhase && !dependsOn(phase, requiredPhase)) {
          errors.push(`roadmap: ${id} in ${phase} requires ${dependency} in ${requiredPhase} without an entry dependency`);
        }
      }
    }
  }
  return errors;
}

export function checkRepository(root) {
  const files = [...new Set(execFileSync(
    'git', ['ls-files', '--cached', '--others', '--exclude-standard', '-z'],
    { cwd: root, encoding: 'utf8' },
  ).split('\0').filter(Boolean))].sort();
  const errors = [];
  for (const file of requiredFiles) {
    if (!files.includes(file) || !existsSync(path.join(root, file))) {
      errors.push(`missing required repository file: ${file}`);
    }
  }
  let checked = 0;
  for (const file of files) {
    if (!/\.(?:md|mjs|ya?ml)$/.test(file)
      && !['LICENSE', '.gitignore', '.gitattributes', '.editorconfig', '.github/CODEOWNERS'].includes(file)) {
      continue;
    }
    if (!existsSync(path.join(root, file))) {
      errors.push(`tracked file missing from working tree: ${file}`);
      continue;
    }
    const text = readFileSync(path.join(root, file), 'utf8');
    checked += 1;
    errors.push(...checkText(text, file));
    if (file.endsWith('.md')) errors.push(...checkLinks(text, file, root));
    if (file === 'docs/plan/roadmap.md') errors.push(...checkRoadmap(text));
  }
  return { checked, errors };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
  const { checked, errors } = checkRepository(root);
  if (errors.length > 0) {
    console.error(errors.join('\n'));
    process.exitCode = 1;
  } else {
    console.log(`Documentation checks passed (${checked} files).`);
  }
}
