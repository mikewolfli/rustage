// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The task graph, its problem matchers and its terminals (task T16.09).
//!
//! The acceptance line is *"tasks are ordered by a typed graph, a background task's readiness is
//! decided from its output, and a task knows which terminal it runs in"*.
//!
//! * **An edge carries its order.** A task either depends on another **in sequence** or **in
//!   parallel**; [`waves`] turns the graph into the groups that may run together, so a task that must
//!   wait is not started alongside its dependency by mistake.
//! * **A problem matcher is a real matcher.** [`Matcher`] compiles a line pattern with `(.*)` and
//!   `(\d+)` groups into parts and matches a line against it — no regular-expression dependency, and
//!   no "parse the message with `split(':')`" that breaks on the first colon in a path.
//! * **Readiness is observed, then declared.** [`Ready`] watches output for a marker; a background
//!   task is ready when the marker appears, and a task that never prints it is not ready.

#![allow(dead_code)]

use std::collections::BTreeMap;

/// How a dependant waits for the thing it depends on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Order {
    /// The dependant runs after the dependency finishes.
    After,
    /// The dependant runs alongside the dependency.
    Together,
}

impl Order {
    /// The order in one line.
    pub fn render(&self) -> &'static str {
        match self {
            Self::After => "after",
            Self::Together => "together with",
        }
    }
}

/// One node of the task graph.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Node {
    /// The task's name.
    pub name: String,
    /// The dependencies and how to wait for each.
    pub depends_on: Vec<(String, Order)>,
    /// Whether the task starts a long-running process that only signals readiness.
    pub background: bool,
    /// The terminal the task runs in, if it was given one.
    pub terminal: Option<String>,
}

/// A task graph.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Dag {
    /// The nodes, in declaration order.
    pub nodes: Vec<Node>,
    /// The task a caller runs when it names none.
    pub default: Option<String>,
}

impl Dag {
    /// The groups of tasks that may run together, in order.
    ///
    /// A task's wave is one past its `After` dependencies and the same wave as its `Together`
    /// dependencies, so a `Together` sibling of a step lands in that step's wave rather than after
    /// it. The level is a fixpoint: it rises until no dependency can raise it further, and a cycle is
    /// detected because it would keep rising.
    pub fn waves(&self) -> Result<Vec<Vec<String>>, String> {
        let names: Vec<&str> = self.nodes.iter().map(|node| node.name.as_str()).collect();
        for node in &self.nodes {
            for (dependency, _) in &node.depends_on {
                if !names.contains(&dependency.as_str()) {
                    return Err(format!(
                        "task {:?} depends on {dependency:?}, which is not a task",
                        node.name
                    ));
                }
            }
        }
        let mut wave: BTreeMap<&str, usize> = self
            .nodes
            .iter()
            .map(|node| (node.name.as_str(), 0))
            .collect();
        // Each pass can raise a level by at most one; a graph of n nodes settles within n passes,
        // and a cycle keeps raising, which the bound turns into a named refusal.
        for _ in 0..=self.nodes.len() {
            let mut changed = false;
            for node in &self.nodes {
                let mut level = 0;
                for (dependency, order) in &node.depends_on {
                    let dependency_level = *wave.get(dependency.as_str()).unwrap_or(&0);
                    let candidate = match order {
                        Order::After => dependency_level + 1,
                        Order::Together => dependency_level,
                    };
                    level = level.max(candidate);
                }
                if level > wave[node.name.as_str()] {
                    wave.insert(node.name.as_str(), level);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let highest = wave.values().copied().max().unwrap_or(0);
        if highest > self.nodes.len() {
            let stuck: Vec<&str> = self
                .nodes
                .iter()
                .filter(|node| wave[node.name.as_str()] > self.nodes.len())
                .map(|node| node.name.as_str())
                .collect();
            return Err(format!("these tasks form a cycle: {}", stuck.join(", ")));
        }
        let mut waves: Vec<Vec<String>> = vec![Vec::new(); highest + 1];
        for node in &self.nodes {
            waves[wave[node.name.as_str()]].push(node.name.clone());
        }
        for group in &mut waves {
            group.sort();
        }
        Ok(waves)
    }

    /// The default task.
    pub fn default_task(&self) -> Result<String, String> {
        if let Some(name) = &self.default {
            if self.nodes.iter().any(|node| node.name == *name) {
                return Ok(name.clone());
            }
            return Err(format!("the default task {name:?} is not a task"));
        }
        Err("no default task is named".to_owned())
    }
}

/// One problem a matcher found in output.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Problem {
    /// The file the problem is in.
    pub file: String,
    /// The line, one-based.
    pub line: u32,
    /// The message.
    pub message: String,
}

impl Problem {
    /// The problem in one line.
    pub fn render(&self) -> String {
        format!("{}:{}: {}", self.file, self.line, self.message)
    }
}

/// One piece of a compiled pattern.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Part {
    /// Literal text that must match exactly.
    Literal(String),
    /// `(.*)` or `(.+)`: any text, at least the given number of characters.
    Any(usize),
    /// `(\d+)`: one or more digits.
    Number,
}

/// A compiled line pattern.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Pattern {
    parts: Vec<Part>,
    groups: usize,
}

/// Compiles a line pattern with `(.*)`, `(.+)` and `(\d+)` groups.
pub fn compile(pattern: &str) -> Result<Pattern, String> {
    let mut parts = Vec::new();
    let mut groups = 0;
    let mut rest = pattern;
    while let Some(start) = rest.find('(') {
        if start > 0 {
            parts.push(Part::Literal(rest[..start].to_owned()));
        }
        let after = &rest[start..];
        let end = after
            .find(')')
            .ok_or_else(|| format!("{pattern:?} has a `(` with no closing `)`"))?;
        let inside = &after[1..end];
        match inside {
            ".*" => parts.push(Part::Any(0)),
            ".+" => parts.push(Part::Any(1)),
            "\\d+" => parts.push(Part::Number),
            other => return Err(format!("{pattern:?} has an unsupported group ({other})")),
        }
        groups += 1;
        rest = &after[end + 1..];
    }
    if !rest.is_empty() {
        parts.push(Part::Literal(rest.to_owned()));
    }
    Ok(Pattern { parts, groups })
}

/// Matches a compiled pattern against a whole line, returning the captured groups.
pub fn apply(pattern: &Pattern, line: &str) -> Option<Vec<String>> {
    let mut groups = Vec::new();
    if match_parts(&pattern.parts, line, &mut groups) {
        debug_assert_eq!(groups.len(), pattern.groups);
        Some(groups)
    } else {
        None
    }
}

fn match_parts(parts: &[Part], input: &str, groups: &mut Vec<String>) -> bool {
    let Some((first, rest)) = parts.split_first() else {
        return input.is_empty();
    };
    match first {
        Part::Literal(text) => input
            .strip_prefix(text)
            .is_some_and(|remaining| match_parts(rest, remaining, groups)),
        Part::Any(min) => {
            for end in (*min..=input.len()).rev() {
                if !input.is_char_boundary(end) {
                    continue;
                }
                groups.push(input[..end].to_owned());
                if match_parts(rest, &input[end..], groups) {
                    return true;
                }
                groups.pop();
            }
            false
        }
        Part::Number => {
            let digits = input
                .find(|character: char| !character.is_ascii_digit())
                .unwrap_or(input.len());
            for end in (1..=digits).rev() {
                groups.push(input[..end].to_owned());
                if match_parts(rest, &input[end..], groups) {
                    return true;
                }
                groups.pop();
            }
            false
        }
    }
}

/// A problem matcher: which output lines become problems, and which groups hold what.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Matcher {
    /// The tool that owns the matcher.
    pub owner: String,
    /// The compiled line pattern.
    pub pattern: Pattern,
    /// Which group holds the file.
    pub file_group: usize,
    /// Which group holds the line number.
    pub line_group: usize,
    /// Which group holds the message.
    pub message_group: usize,
}

/// Builds a matcher, refusing group indices the pattern does not have.
pub fn matcher(
    owner: &str,
    pattern: &str,
    file_group: usize,
    line_group: usize,
    message_group: usize,
) -> Result<Matcher, String> {
    let pattern = compile(pattern)?;
    for index in [file_group, line_group, message_group] {
        if index >= pattern.groups {
            return Err(format!(
                "{owner:?}'s matcher names group {index} but the pattern has {}",
                pattern.groups
            ));
        }
    }
    Ok(Matcher {
        owner: owner.to_owned(),
        pattern,
        file_group,
        line_group,
        message_group,
    })
}

/// Tests one line and, when it matches, returns the problem it describes.
pub fn match_line(matcher: &Matcher, line: &str) -> Option<Problem> {
    let groups = apply(&matcher.pattern, line)?;
    let file = groups.get(matcher.file_group)?.clone();
    let line_number: u32 = groups.get(matcher.line_group)?.parse().ok()?;
    let message = groups.get(matcher.message_group)?.clone();
    Some(Problem {
        file,
        line: line_number,
        message,
    })
}

/// Every problem a matcher finds in a block of output.
pub fn match_output(matcher: &Matcher, output: &str) -> Vec<Problem> {
    output
        .lines()
        .filter_map(|line| match_line(matcher, line))
        .collect()
}

/// A background task's readiness marker.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Ready {
    /// The task this readiness belongs to.
    pub task: String,
    /// The text whose appearance in the output means the task is ready.
    pub marker: String,
}

impl Ready {
    /// Whether the output shows the task is ready.
    pub fn is_ready(&self, output: &str) -> bool {
        output.contains(&self.marker)
    }

    /// The readiness in one line.
    pub fn render(&self) -> String {
        format!("{} is ready when it prints {:?}", self.task, self.marker)
    }
}

/// Which terminal a task runs in.
pub fn terminal_of<'a>(dag: &'a Dag, task: &str) -> Option<&'a str> {
    dag.nodes
        .iter()
        .find(|node| node.name == task)
        .and_then(|node| node.terminal.as_deref())
}

/// The graph's tasks by name, for callers that need to look one up.
pub fn by_name(dag: &Dag) -> BTreeMap<&str, &Node> {
    dag.nodes
        .iter()
        .map(|node| (node.name.as_str(), node))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Dag, Node, Order, Ready, by_name, match_line, match_output, matcher, terminal_of};

    fn node(name: &str, depends: &[(&str, Order)]) -> Node {
        Node {
            name: name.to_owned(),
            depends_on: depends
                .iter()
                .map(|(name, order)| ((*name).to_owned(), *order))
                .collect(),
            background: false,
            terminal: None,
        }
    }

    #[test]
    fn a_sequential_dependency_waits_and_a_parallel_one_does_not() {
        let dag = Dag {
            nodes: vec![
                node("gen", &[]),
                node("build", &[("gen", Order::After)]),
                node("watch", &[("build", Order::Together)]),
            ],
            default: Some("build".to_owned()),
        };
        let waves = dag.waves().expect("waves");
        assert_eq!(waves[0], vec!["gen"]);
        // `watch` runs together with `build`, so it shares `build`'s wave.
        assert_eq!(waves[1], vec!["build", "watch"]);
        assert_eq!(dag.default_task().unwrap(), "build");
    }

    #[test]
    fn a_cycle_is_refused() {
        let dag = Dag {
            nodes: vec![
                node("a", &[("b", Order::After)]),
                node("b", &[("a", Order::After)]),
            ],
            default: None,
        };
        assert!(dag.waves().unwrap_err().contains("cycle"));
    }

    #[test]
    fn a_problem_matcher_reads_a_real_line() {
        let m = matcher("cargo", "(.+):(\\d+): (.+)", 0, 1, 2).expect("a matcher");
        let problem = match_line(&m, "src/main.rs:12: mismatched types").expect("a problem");
        assert_eq!(problem.file, "src/main.rs");
        assert_eq!(problem.line, 12);
        assert_eq!(problem.message, "mismatched types");
        assert_eq!(problem.render(), "src/main.rs:12: mismatched types");
        // A line that is not a problem does not become one.
        assert!(match_line(&m, "Compiling demo").is_none());
    }

    #[test]
    fn a_matcher_refuses_a_group_the_pattern_does_not_have() {
        assert!(matcher("x", "(\\d+)", 3, 0, 0).is_err());
    }

    #[test]
    fn a_matcher_finds_every_problem_in_a_block() {
        let m = matcher("cargo", "(.+):(\\d+): (.+)", 0, 1, 2).expect("a matcher");
        let output = "Compiling\nsrc/a.rs:1: first\nsrc/b.rs:9: second\n";
        assert_eq!(match_output(&m, output).len(), 2);
    }

    #[test]
    fn a_background_task_is_ready_only_when_its_marker_appears() {
        let ready = Ready {
            task: "server".to_owned(),
            marker: "listening on".to_owned(),
        };
        assert!(!ready.is_ready("starting up"));
        assert!(ready.is_ready("server: listening on 127.0.0.1:0"));
        assert!(ready.render().contains("listening on"));
    }

    #[test]
    fn a_task_knows_its_terminal() {
        let mut build = node("build", &[]);
        build.terminal = Some("build".to_owned());
        let dag = Dag {
            nodes: vec![build],
            default: None,
        };
        assert_eq!(terminal_of(&dag, "build"), Some("build"));
        assert_eq!(terminal_of(&dag, "missing"), None);
        assert!(by_name(&dag).contains_key("build"));
    }
}
