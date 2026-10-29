// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Configured tasks: what runs, in what order, and what a re-run may skip (task T16.05).
//!
//! The acceptance line is *"pre- and post-tasks are honoured, a re-run is skipped only when it is
//! safe, and cleanup removes just this task's output"*.
//!
//! * **Cleanup is scoped.** [`cleanup_plan`] reports every declared output with whether it exists
//!   and how many bytes it costs, and refuses a path that is absolute, escapes the project root,
//!   resolves outside it, or is a directory. [`Cleanup::apply`] deletes only the entries the plan
//!   allowed, so a cleanup cannot become "remove a directory I did not name".
//! * **Order is computed, not configured twice.** [`schedule`] assigns every task to a wave: nothing
//!   in a wave depends on anything else in it, so a wave may run concurrently, and every task runs
//!   after its dependencies, its pre-tasks and before its post-tasks. A cycle, or a name that is not
//!   a task, is a refusal naming the tasks involved — a task graph that cannot be ordered is a
//!   defect, not a reason to run the tasks in file order.
//! * **The order is deterministic.** Ties are broken by name, so two people running the same task
//!   file get the same sequence (principle 16).
//! * **A cache hit must be earned.** [`cache_key`] is a function of the command, the environment and
//!   the declared inputs, so a task whose inputs changed is not skipped because its name did not.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

/// One configured task.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Task {
    /// The task's name, unique in a file.
    pub name: String,
    /// The command, program first.
    pub command: Vec<String>,
    /// The names this task depends on; they run first.
    pub depends_on: Vec<String>,
    /// Tasks that run immediately before this one.
    ///
    /// A pre-task is an edge like any dependency: it must finish first. It is declared separately so
    /// a task file can say "build runs after lint" without pretending the build *depends* on lint.
    pub pre: Vec<String>,
    /// Tasks that run immediately after this one.
    pub post: Vec<String>,
    /// The working directory, when it is not the project's.
    pub directory: Option<String>,
    /// The files this task reads, for the cache key.
    pub inputs: Vec<String>,
    /// The files this task writes, for a scoped cleanup.
    pub outputs: Vec<String>,
}

/// Orders the tasks so every dependency, pre-task and post-task runs in its place.
///
/// The order is deterministic (dependencies first, then by name) and a cycle is refused with the
/// names involved.
pub fn order(tasks: &[Task]) -> Result<Vec<String>, String> {
    Ok(schedule(tasks)?.into_iter().flatten().collect())
}

/// Assigns every task to a wave that may run concurrently.
///
/// A task is in the first wave in which all of its dependencies, pre-tasks and predecessors' post
/// edges are satisfied. Tasks in one wave have no edge between them, so running them at once is
/// safe; none is promised to run at once by this module.
pub fn schedule(tasks: &[Task]) -> Result<Vec<Vec<String>>, String> {
    let mut names: BTreeSet<&str> = BTreeSet::new();
    for task in tasks {
        if !names.insert(task.name.as_str()) {
            return Err(format!("two tasks are both named {:?}", task.name));
        }
    }
    // The edges: dependency -> task, pre -> task, task -> post.
    let mut edges: Vec<(&str, &str)> = Vec::new();
    for task in tasks {
        for dependency in task.depends_on.iter().chain(&task.pre) {
            if !names.contains(dependency.as_str()) {
                return Err(format!(
                    "task {:?} depends on {dependency:?}, which is not a task",
                    task.name
                ));
            }
            edges.push((dependency.as_str(), task.name.as_str()));
        }
        for post in &task.post {
            if !names.contains(post.as_str()) {
                return Err(format!(
                    "task {:?} runs {post:?} after it, which is not a task",
                    task.name
                ));
            }
            edges.push((task.name.as_str(), post.as_str()));
        }
    }

    let mut satisfied: BTreeSet<&str> = BTreeSet::new();
    let mut waves: Vec<Vec<String>> = Vec::new();
    loop {
        let mut ready: Vec<&str> = tasks
            .iter()
            .map(|task| task.name.as_str())
            .filter(|name| !satisfied.contains(name))
            .filter(|name| {
                edges
                    .iter()
                    .filter(|(_, to)| to == name)
                    .all(|(from, _)| satisfied.contains(from))
            })
            .collect();
        if ready.is_empty() {
            break;
        }
        ready.sort_unstable();
        let wave: Vec<String> = ready.iter().map(|name| (*name).to_owned()).collect();
        for name in ready {
            satisfied.insert(name);
        }
        waves.push(wave);
    }
    let stuck: Vec<&str> = tasks
        .iter()
        .map(|task| task.name.as_str())
        .filter(|name| !satisfied.contains(name))
        .collect();
    if !stuck.is_empty() {
        return Err(format!("these tasks form a cycle: {}", stuck.join(", ")));
    }
    Ok(waves)
}

/// A cache key for a task: its command, its environment and its declared inputs' fingerprints.
///
/// `fingerprint` is a caller-supplied digest of one input file (a size and hash, say). It is a
/// parameter rather than a file read here because the caller is the one that watches the files.
pub fn cache_key(
    task: &Task,
    environment: &[(String, String)],
    inputs: &[(String, String)],
) -> String {
    let mut parts = vec![format!("cmd:{}", task.command.join(" "))];
    for (name, value) in environment {
        parts.push(format!("env:{name}={value}"));
    }
    for (path, fingerprint) in inputs {
        parts.push(format!("in:{path}={fingerprint}"));
    }
    parts.join("\n")
}

/// The outputs a task declares, for a scoped cleanup.
pub fn cleanup(task: &Task) -> Vec<String> {
    task.outputs.clone()
}

/// One declared output, as a cleanup plan sees it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CleanupEntry {
    /// The task that declared it.
    pub task: String,
    /// The path, as declared.
    pub path: String,
    /// Whether the path exists right now.
    pub exists: bool,
    /// Its size in bytes, when it exists and is a file.
    pub bytes: u64,
    /// Why it will not be removed, when it will not be.
    pub refused: Option<String>,
    /// The declared path joined to the project root that was checked.
    absolute: PathBuf,
}

/// Plans the cleanup of the named tasks' declared outputs inside `root`.
///
/// A path is refused when it is empty, absolute, contains `..`, resolves outside `root`, or is a
/// directory. The refusals are part of the plan, so the scope a person reads is the scope that will
/// be applied.
pub fn cleanup_plan(root: &Path, tasks: &[Task], names: &[&str]) -> Cleanup {
    let canonical_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut plan = Cleanup::default();
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    for name in names {
        let Some(task) = tasks.iter().find(|task| &task.name == name) else {
            plan.problems.push(format!("no task {name:?}"));
            continue;
        };
        for output in &task.outputs {
            let absolute = root.join(output);
            if !seen.insert(absolute.clone()) {
                continue;
            }
            let refused = refusal_reason(root, &canonical_root, &absolute, output);
            let (exists, bytes) = if refused.is_none() {
                match std::fs::metadata(&absolute) {
                    Ok(metadata) if metadata.is_file() => (true, metadata.len()),
                    _ => (false, 0),
                }
            } else {
                (false, 0)
            };
            plan.entries.push(CleanupEntry {
                task: task.name.clone(),
                path: output.clone(),
                exists,
                bytes,
                refused,
                absolute,
            });
        }
    }
    plan
}

/// Why an output path must not be deleted, if it must not.
fn refusal_reason(
    root: &Path,
    canonical_root: &Path,
    absolute: &Path,
    declared: &str,
) -> Option<String> {
    if declared.trim().is_empty() {
        return Some("the path is empty".to_owned());
    }
    if Path::new(declared).is_absolute() {
        return Some("an absolute path is outside the project".to_owned());
    }
    if Path::new(declared)
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Some("the path escapes the project root".to_owned());
    }
    if absolute == root {
        return Some("the project root itself is not a task output".to_owned());
    }
    if let Ok(canonical) = std::fs::canonicalize(absolute)
        && !canonical.starts_with(canonical_root)
    {
        return Some(format!(
            "{} resolves outside the project",
            canonical.display()
        ));
    }
    if absolute.is_dir() {
        return Some("a directory is not removed by a task cleanup".to_owned());
    }
    None
}

impl CleanupEntry {
    /// Whether the plan may delete it.
    pub fn removable(&self) -> bool {
        self.refused.is_none() && self.exists
    }

    /// The entry in one line.
    pub fn render(&self) -> String {
        match &self.refused {
            Some(reason) => format!("{}: {} — kept ({reason})", self.task, self.path),
            None if self.exists => format!("{}: {} ({} byte(s))", self.task, self.path, self.bytes),
            None => format!("{}: {} — not present", self.task, self.path),
        }
    }
}

/// The scope a cleanup covers: every declared output of the named tasks.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Cleanup {
    /// The entries, in task order then declaration order.
    pub entries: Vec<CleanupEntry>,
    /// Tasks that exist but whose outputs could not be planned (a path Git or the disk rejected).
    pub problems: Vec<String>,
}

impl Cleanup {
    /// The bytes the plan would free.
    pub fn bytes(&self) -> u64 {
        self.entries
            .iter()
            .filter(|entry| entry.removable())
            .map(|entry| entry.bytes)
            .sum()
    }

    /// How many entries the plan would delete.
    pub fn removable(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.removable())
            .count()
    }

    /// The plan in lines: every entry, then the scope and the total.
    pub fn render(&self) -> Vec<String> {
        let mut lines: Vec<String> = self.entries.iter().map(CleanupEntry::render).collect();
        for problem in &self.problems {
            lines.push(format!("problem: {problem}"));
        }
        lines.push(format!(
            "cleanup scope: {} path(s), {} removable, {} byte(s)",
            self.entries.len(),
            self.removable(),
            self.bytes()
        ));
        lines
    }

    /// Deletes the entries the plan allowed, and nothing else.
    ///
    /// Only files are removed: a directory output is refused when the plan is made, so a cleanup can
    /// never delete a tree it was not given. A failure is reported per path rather than stopping the
    /// rest.
    pub fn apply(&self) -> (Vec<String>, Vec<String>) {
        let mut removed = Vec::new();
        let mut failed = Vec::new();
        for entry in &self.entries {
            if !entry.removable() {
                continue;
            }
            match std::fs::remove_file(entry.full_path()) {
                Ok(()) => removed.push(entry.path.clone()),
                Err(error) => failed.push(format!("{}: {error}", entry.path)),
            }
        }
        (removed, failed)
    }
}

impl CleanupEntry {
    /// The absolute path, kept next to the declared one so `apply` never re-joins a root.
    fn full_path(&self) -> PathBuf {
        self.absolute.clone()
    }
}

/// The default task: the one a caller runs when it names none.
///
/// A file with no explicit default uses the task that is nobody's dependency and nobody's post-task:
/// a pre-task is run *by* its main task and a post-task runs *after* it, so neither is what you
/// invoke. Two candidates is ambiguous and is refused rather than guessed.
pub fn default_task(tasks: &[Task], declared: Option<&str>) -> Result<String, String> {
    if let Some(name) = declared {
        if tasks.iter().any(|task| task.name == name) {
            return Ok(name.to_owned());
        }
        return Err(format!("the declared default task {name:?} is not a task"));
    }
    let depended: BTreeSet<&str> = tasks
        .iter()
        .flat_map(|task| task.depends_on.iter().chain(&task.pre).map(String::as_str))
        .collect();
    let after: BTreeSet<&str> = tasks
        .iter()
        .flat_map(|task| task.post.iter().map(String::as_str))
        .collect();
    let roots: Vec<&str> = tasks
        .iter()
        .filter(|task| !depended.contains(task.name.as_str()))
        .filter(|task| !after.contains(task.name.as_str()))
        .map(|task| task.name.as_str())
        .collect();
    match roots.len() {
        1 => Ok(roots[0].to_owned()),
        0 => Err("every task depends on another; there is no default".to_owned()),
        _ => Err(format!(
            "more than one task has no dependents ({}); name a default",
            roots.join(", ")
        )),
    }
}

/// The order and default in one line per fact.
pub fn render(tasks: &[Task], order: &[String], default: &str) -> Vec<String> {
    let by_name: BTreeMap<&str, &Task> = tasks
        .iter()
        .map(|task| (task.name.as_str(), task))
        .collect();
    let mut out = vec![format!("default: {default}")];
    for name in order {
        if let Some(task) = by_name.get(name.as_str()) {
            let mut line = format!("{name}: {}", task.command.join(" "));
            let before: Vec<&String> = task.depends_on.iter().chain(&task.pre).collect();
            if !before.is_empty() {
                let names: Vec<&str> = before.iter().map(|name| name.as_str()).collect();
                line.push_str(&format!(" (after {})", names.join(", ")));
            }
            if !task.post.is_empty() {
                line.push_str(&format!(" (then {})", task.post.join(", ")));
            }
            out.push(line);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Task, cache_key, cleanup, cleanup_plan, default_task, order, schedule};

    fn task(name: &str, depends_on: &[&str]) -> Task {
        Task {
            name: name.to_owned(),
            command: vec!["run".to_owned(), name.to_owned()],
            depends_on: depends_on.iter().map(|s| (*s).to_owned()).collect(),
            pre: Vec::new(),
            post: Vec::new(),
            directory: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
        }
    }

    #[test]
    fn dependencies_run_before_their_dependents() {
        let tasks = vec![
            task("build", &["gen"]),
            task("gen", &[]),
            task("run", &["build"]),
        ];
        assert_eq!(order(&tasks).unwrap(), vec!["gen", "build", "run"]);
    }

    #[test]
    fn a_cycle_is_refused_with_the_names_involved() {
        let tasks = vec![task("a", &["b"]), task("b", &["a"])];
        let error = order(&tasks).expect_err("a cycle");
        assert!(error.contains("cycle"), "{error}");
        assert!(error.contains("a") && error.contains("b"));
    }

    #[test]
    fn a_dependency_that_is_not_a_task_is_refused() {
        let error = order(&[task("a", &["missing"])]).expect_err("a missing dependency");
        assert!(error.contains("missing"), "{error}");
    }

    #[test]
    fn two_tasks_with_one_name_are_refused() {
        let error = order(&[task("a", &[]), task("a", &[])]).expect_err("a duplicate");
        assert!(error.contains("both named"), "{error}");
    }

    #[test]
    fn the_default_is_the_one_thing_nothing_depends_on() {
        let tasks = vec![task("build", &[]), task("run", &["build"])];
        // The default is the task nothing depends on: the one a caller runs.
        assert_eq!(default_task(&tasks, None).unwrap(), "run");
        assert_eq!(default_task(&tasks, Some("build")).unwrap(), "build");
        assert!(default_task(&tasks, Some("nope")).is_err());
        let ambiguous = vec![task("a", &[]), task("b", &[])];
        assert!(default_task(&ambiguous, None).is_err());
    }

    #[test]
    fn a_cache_key_changes_when_an_input_changes() {
        let mut build = task("build", &[]);
        build.inputs = vec!["src/main.rs".to_owned()];
        let before = cache_key(&build, &[], &[("src/main.rs".to_owned(), "aaa".to_owned())]);
        let after = cache_key(&build, &[], &[("src/main.rs".to_owned(), "bbb".to_owned())]);
        assert_ne!(before, after);
        let same = cache_key(&build, &[], &[("src/main.rs".to_owned(), "aaa".to_owned())]);
        assert_eq!(before, same);
    }

    #[test]
    fn cleanup_removes_only_the_declared_outputs() {
        let mut build = task("build", &[]);
        build.outputs = vec!["target/demo".to_owned()];
        assert_eq!(cleanup(&build), vec!["target/demo"]);
        assert!(cleanup(&task("gen", &[])).is_empty());
    }

    #[test]
    fn pre_and_post_tasks_take_their_place_in_the_order() {
        let mut build = task("build", &[]);
        build.pre = vec!["lint".to_owned()];
        let mut test = task("test", &["build"]);
        test.post = vec!["report".to_owned()];
        let tasks = vec![build, task("lint", &[]), test, task("report", &[])];

        assert_eq!(
            schedule(&tasks).expect("waves"),
            vec![vec!["lint"], vec!["build"], vec!["test"], vec!["report"]]
        );
        assert_eq!(
            order(&tasks).expect("an order"),
            vec!["lint", "build", "test", "report"]
        );
    }

    #[test]
    fn independent_tasks_share_one_wave() {
        let tasks = vec![task("a", &[]), task("b", &[]), task("both", &["a", "b"])];
        assert_eq!(
            schedule(&tasks).expect("waves"),
            vec![vec!["a", "b"], vec!["both"]]
        );
    }

    #[test]
    fn a_cycle_through_post_tasks_is_refused() {
        let mut a = task("a", &[]);
        a.post = vec!["b".to_owned()];
        let mut b = task("b", &[]);
        b.post = vec!["a".to_owned()];
        let error = schedule(&[a, b]).expect_err("a cycle");
        assert!(error.contains("cycle"), "{error}");
    }

    #[test]
    fn a_pre_or_post_name_that_is_not_a_task_is_refused() {
        let mut build = task("build", &[]);
        build.pre = vec!["ghost".to_owned()];
        assert!(schedule(&[build]).is_err());
        let mut build = task("build", &[]);
        build.post = vec!["ghost".to_owned()];
        let error = schedule(&[build]).expect_err("an unknown post task");
        assert!(error.contains("ghost"), "{error}");
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rustage-tasks-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("a clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).expect("a directory");
        path
    }

    #[test]
    fn the_cleanup_plan_shows_its_scope_and_refuses_what_it_must_not_touch() {
        let root = scratch("plan");
        std::fs::create_dir_all(root.join("out")).expect("a directory");
        std::fs::create_dir_all(root.join("dist")).expect("a directory");
        std::fs::write(root.join("out/demo"), b"12345").expect("a file");
        std::fs::write(root.join("keep.txt"), b"keep").expect("a file");

        let mut build = task("build", &[]);
        build.outputs = vec![
            "out/demo".to_owned(),
            "dist".to_owned(),
            "/tmp/elsewhere".to_owned(),
            "../escape".to_owned(),
            "keep.txt".to_owned(),
        ];
        let plan = cleanup_plan(&root, &[build], &["build"]);
        assert_eq!(plan.entries.len(), 5);
        assert!(plan.entries[0].removable());
        assert_eq!(plan.entries[0].bytes, 5);
        assert!(plan.entries[1].refused.is_some(), "{plan:?}");
        assert!(plan.entries[2].refused.is_some(), "{plan:?}");
        assert!(plan.entries[3].refused.is_some(), "{plan:?}");
        assert!(plan.entries[4].removable());
        assert_eq!(plan.removable(), 2);
        assert_eq!(plan.bytes(), 5 + 4);

        let report = plan.render().join("\n");
        assert!(report.contains("out/demo (5 byte(s))"), "{report}");
        assert!(report.contains("kept (an absolute path"), "{report}");
        assert!(report.contains("kept (the path escapes"), "{report}");
        assert!(
            report.contains("cleanup scope: 5 path(s), 2 removable"),
            "{report}"
        );

        let (removed, failed) = plan.apply();
        assert_eq!(removed, vec!["out/demo", "keep.txt"]);
        assert!(failed.is_empty(), "{failed:?}");
        assert!(!root.join("out/demo").exists());
        // The directory was refused, so it is still there.
        assert!(root.join("dist").is_dir());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unknown_task_is_a_problem_rather_than_a_silent_skip() {
        let root = scratch("ghost");
        let plan = cleanup_plan(&root, &[task("build", &[])], &["ghost"]);
        assert!(plan.entries.is_empty());
        assert_eq!(plan.problems.len(), 1);
        assert!(plan.problems[0].contains("ghost"), "{plan:?}");
        assert!(plan.render().join("\n").contains("problem: no task"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
