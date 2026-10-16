// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Configured tasks: what runs, in what order, and what a re-run may skip (task T16.05).
//!
//! The acceptance line is *"pre- and post-tasks are honoured, a re-run is skipped only when it is
//! safe, and cleanup removes just this task's output"*.
//!
//! * **Order is computed, not configured twice.** [`order`] is a topological sort over the declared
//!   dependencies, and a cycle is a refusal that names the tasks in it — a task graph that cannot be
//!   ordered is a defect, not a reason to run the tasks in file order.
//! * **The order is deterministic.** Ties are broken by name, so two people running the same task
//!   file get the same sequence (principle 16).
//! * **A cache hit must be earned.** [`cache_key`] is a function of the command, the environment and
//!   the declared inputs, so a task whose inputs changed is not skipped because its name did not.
//! * **Cleanup is scoped.** [`cleanup`] removes only the outputs a task declares, never a directory
//!   it did not name.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

/// One configured task.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Task {
    /// The task's name, unique in a file.
    pub name: String,
    /// The command, program first.
    pub command: Vec<String>,
    /// The names this task depends on; they run first.
    pub depends_on: Vec<String>,
    /// The working directory, when it is not the project's.
    pub directory: Option<String>,
    /// The files this task reads, for the cache key.
    pub inputs: Vec<String>,
    /// The files this task writes, for a scoped cleanup.
    pub outputs: Vec<String>,
}

/// Orders the tasks so every dependency runs before its dependents.
///
/// The order is deterministic (dependencies first, then by name) and a cycle is refused with the
/// names involved.
pub fn order(tasks: &[Task]) -> Result<Vec<String>, String> {
    let mut names: BTreeSet<&str> = BTreeSet::new();
    for task in tasks {
        if !names.insert(task.name.as_str()) {
            return Err(format!("two tasks are both named {:?}", task.name));
        }
    }
    for task in tasks {
        for dependency in &task.depends_on {
            if !names.contains(dependency.as_str()) {
                return Err(format!(
                    "task {:?} depends on {dependency:?}, which is not a task",
                    task.name
                ));
            }
        }
    }

    let mut satisfied: BTreeSet<String> = BTreeSet::new();
    let mut out: Vec<String> = Vec::new();
    loop {
        // Every task whose dependencies are all satisfied, in name order.
        let mut ready: Vec<&Task> = tasks
            .iter()
            .filter(|task| !satisfied.contains(&task.name))
            .filter(|task| {
                task.depends_on
                    .iter()
                    .all(|dependency| satisfied.contains(dependency))
            })
            .collect();
        ready.sort_by(|a, b| a.name.cmp(&b.name));
        if ready.is_empty() {
            break;
        }
        for task in ready {
            satisfied.insert(task.name.clone());
            out.push(task.name.clone());
        }
    }
    if out.len() != tasks.len() {
        let stuck: Vec<&str> = tasks
            .iter()
            .filter(|task| !satisfied.contains(&task.name))
            .map(|task| task.name.as_str())
            .collect();
        return Err(format!("these tasks form a cycle: {}", stuck.join(", ")));
    }
    Ok(out)
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

/// The default task: the one a caller runs when it names none.
///
/// A file with no explicit default uses the task nothing else depends on when there is exactly one;
/// two candidates is ambiguous and is refused rather than guessed.
pub fn default_task(tasks: &[Task], declared: Option<&str>) -> Result<String, String> {
    if let Some(name) = declared {
        if tasks.iter().any(|task| task.name == name) {
            return Ok(name.to_owned());
        }
        return Err(format!("the declared default task {name:?} is not a task"));
    }
    let depended: BTreeSet<&str> = tasks
        .iter()
        .flat_map(|task| task.depends_on.iter().map(String::as_str))
        .collect();
    let roots: Vec<&str> = tasks
        .iter()
        .filter(|task| !depended.contains(task.name.as_str()))
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
            out.push(format!(
                "{name}: {}{}",
                task.command.join(" "),
                if task.depends_on.is_empty() {
                    String::new()
                } else {
                    format!(" (after {})", task.depends_on.join(", "))
                },
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Task, cache_key, cleanup, default_task, order};

    fn task(name: &str, depends_on: &[&str]) -> Task {
        Task {
            name: name.to_owned(),
            command: vec!["run".to_owned(), name.to_owned()],
            depends_on: depends_on.iter().map(|s| (*s).to_owned()).collect(),
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
}
