// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Threads, stack frames, scopes, paged variables, watches and evaluation (task T19.05).
//!
//! The acceptance line is *"a slow adapter or a large variable does not block the UI, and evaluation
//! with side effects follows a policy"*. Three bounds carry it:
//!
//! * **Nothing is expanded wholesale.** [`page`] never returns more than [`PAGE_LIMIT`] variables,
//!   a stack request asks for [`STACK_LIMIT`] frames, and both say whether more remain. A variable
//!   with children is a reference to be asked about later, not a tree to render up front.
//! * **A timeout is its own result.** [`Evaluation::TimedOut`] is a variant, not an empty string;
//!   the protocol's deadline produces it and the UI can say "still waiting" without inventing a
//!   value.
//! * **Evaluation is a policy decision.** [`EvalPolicy::Deny`] refuses every expression,
//!   [`EvalPolicy::Ask`] asks before evaluating anything that could call code, and
//!   [`EvalPolicy::Allow`] runs it. A watch expression is re-evaluated under the same policy; the
//!   value and the error are separate fields so a failed watch cannot look like an empty value.

#![allow(dead_code)]

use std::time::Duration;

/// The most variables one page returns.
pub const PAGE_LIMIT: usize = 100;

/// The most stack frames one request asks for.
pub const STACK_LIMIT: usize = 20;

/// How long an evaluation may take before the protocol layer reports a timeout.
pub const EVALUATE_TIMEOUT: Duration = Duration::from_secs(5);

/// One thread.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Thread {
    /// Its adapter-assigned id.
    pub id: i64,
    /// Its name.
    pub name: String,
}

/// One stack frame.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Frame {
    /// Its adapter-assigned id.
    pub id: i64,
    /// Its function name.
    pub name: String,
    /// The generated file, when the frame has one.
    pub file: Option<String>,
    /// The line, counting from one.
    pub line: u32,
    /// The column, counting from one.
    pub column: u32,
    /// The source version the frame's debug information belongs to.
    pub source_version: u64,
}

impl Frame {
    /// The frame in one line.
    pub fn render(&self) -> String {
        match &self.file {
            Some(file) => format!(
                "{} at {file}:{}:{} (source version {})",
                self.name, self.line, self.column, self.source_version
            ),
            None => format!(
                "{} (no generated source; source version {})",
                self.name, self.source_version
            ),
        }
    }
}

/// One stack page.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StackPage {
    /// The frames.
    pub frames: Vec<Frame>,
    /// How many the adapter said there are in total, when it says.
    pub total: Option<usize>,
}

impl StackPage {
    /// Whether the adapter reported more frames than this page.
    pub fn more(&self) -> bool {
        self.total.is_some_and(|total| total > self.frames.len())
    }
}

/// One scope of a frame.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Scope {
    /// Its name (`Local`, `Global`, ...).
    pub name: String,
    /// Its variables reference.
    pub variables_reference: i64,
    /// Whether reading it is expensive, as the adapter says.
    pub expensive: bool,
    /// How many variables the adapter says it holds.
    pub count: Option<usize>,
}

/// One variable.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Variable {
    /// Its name.
    pub name: String,
    /// Its value, as the adapter rendered it.
    pub value: String,
    /// Its type, when the adapter names one.
    pub type_name: Option<String>,
    /// A reference to its children, or `0`.
    pub variables_reference: i64,
    /// How many named children, when the adapter says.
    pub named: Option<usize>,
    /// How many indexed children, when the adapter says.
    pub indexed: Option<usize>,
}

impl Variable {
    /// Whether it can be expanded.
    pub fn expandable(&self) -> bool {
        self.variables_reference != 0
    }

    /// The variable in one line.
    pub fn render(&self) -> String {
        let kind = self
            .type_name
            .as_ref()
            .map(|name| format!(": {name}"))
            .unwrap_or_default();
        format!("{}{kind} = {}", self.name, self.value)
    }
}

/// One bounded page of variables.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Page {
    /// Where in the list it started.
    pub start: usize,
    /// The variables it holds.
    pub variables: Vec<Variable>,
    /// How many exist in total, when the adapter said.
    pub total: Option<usize>,
    /// Whether more remain after this page.
    pub more: bool,
}

impl Page {
    /// The page in one line.
    pub fn render(&self) -> String {
        format!(
            "{}-{} of {}{}",
            self.start + 1,
            self.start + self.variables.len(),
            self.total
                .map(|total| total.to_string())
                .unwrap_or_else(|| "?".to_owned()),
            if self.more { " (more)" } else { "" }
        )
    }
}

/// Cuts a variable list to a bounded page.
///
/// `count` is a request, not a promise: it is clamped to [`PAGE_LIMIT`], so a caller that asks for
/// everything gets one page and a `more` flag instead of a frozen UI.
pub fn page(variables: Vec<Variable>, start: usize, count: usize, total: Option<usize>) -> Page {
    let count = count.min(PAGE_LIMIT);
    let all = variables.len();
    let total = total.or(Some(all));
    let variables: Vec<Variable> = variables.into_iter().skip(start).take(count).collect();
    let more = total.is_some_and(|total| start + variables.len() < total);
    Page {
        start,
        variables,
        total,
        more,
    }
}

/// What may be evaluated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EvalPolicy {
    /// Evaluate anything.
    Allow,
    /// Ask before evaluating something that could call code.
    Ask,
    /// Refuse evaluation.
    Deny,
}

impl EvalPolicy {
    /// The policy's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Deny => "deny",
        }
    }

    /// Reads a policy name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "allow" => Some(Self::Allow),
            "ask" => Some(Self::Ask),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }
}

/// What an evaluation request may do.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EvalDecision {
    /// Run it.
    Run,
    /// Ask the person first.
    Confirm {
        /// The question.
        prompt: String,
    },
    /// Do not run it.
    Refuse {
        /// Why.
        reason: String,
    },
}

/// Decides an evaluation request under a policy.
///
/// An expression that is a plain path or literal has no side effect, so `Ask` runs it without a
/// prompt. Anything with a call — `(`, `!` after an identifier, an assignment — may run code, so
/// `Ask` asks. `Deny` refuses both.
pub fn decide(policy: EvalPolicy, expression: &str) -> EvalDecision {
    let expression = expression.trim();
    if expression.is_empty() {
        return EvalDecision::Refuse {
            reason: "the expression is empty".to_owned(),
        };
    }
    if policy == EvalPolicy::Deny {
        return EvalDecision::Refuse {
            reason: "evaluation is disabled by policy".to_owned(),
        };
    }
    if policy == EvalPolicy::Ask && may_have_side_effects(expression) {
        return EvalDecision::Confirm {
            prompt: format!("{expression:?} may call code; evaluate it for this stop?"),
        };
    }
    EvalDecision::Run
}

/// Whether an expression could call code.
///
/// This is a conservative syntactic check, not a type check: a false positive asks one extra
/// question, while a false negative would run a function without asking. That asymmetry is why the
/// check is broader than it needs to be.
pub fn may_have_side_effects(expression: &str) -> bool {
    expression.contains('(')
        || expression.contains('=')
        || expression.contains('!')
        || expression.contains('{')
        || expression.contains("..")
}

/// One evaluation attempt.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Evaluation {
    /// The adapter answered.
    Value {
        /// The rendered value.
        value: String,
        /// Its type, when named.
        type_name: Option<String>,
        /// A reference to children, or `0`.
        variables_reference: i64,
    },
    /// The adapter answered with an error.
    Error {
        /// What it said.
        message: String,
    },
    /// The deadline passed first.
    TimedOut {
        /// How long was waited.
        milliseconds: u64,
    },
    /// The policy did not let it run.
    Refused {
        /// Why.
        reason: String,
    },
}

impl Evaluation {
    /// The evaluation in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Value {
                value, type_name, ..
            } => match type_name {
                Some(kind) => format!("{kind} {value}"),
                None => value.clone(),
            },
            Self::Error { message } => format!("evaluation failed: {message}"),
            Self::TimedOut { milliseconds } => {
                format!("evaluation timed out after {milliseconds} ms")
            }
            Self::Refused { reason } => format!("evaluation refused: {reason}"),
        }
    }
}

/// One watch expression and its latest result.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Watch {
    /// The expression.
    pub expression: String,
    /// The latest evaluation, when one ran.
    pub result: Option<Evaluation>,
}

impl Watch {
    /// A watch with no result yet.
    pub fn new(expression: impl Into<String>) -> Self {
        Self {
            expression: expression.into(),
            result: None,
        }
    }

    /// The watch in one line.
    pub fn render(&self) -> String {
        match &self.result {
            Some(result) => format!("{}: {}", self.expression, result.render()),
            None => format!("{}: not evaluated", self.expression),
        }
    }
}

/// The watches of one session.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Watches {
    entries: Vec<Watch>,
}

impl Watches {
    /// No watches.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a watch, refusing an empty or duplicate expression.
    pub fn add(&mut self, expression: &str) -> Result<usize, String> {
        let expression = expression.trim();
        if expression.is_empty() {
            return Err("a watch needs an expression".to_owned());
        }
        if self
            .entries
            .iter()
            .any(|entry| entry.expression == expression)
        {
            return Err(format!("{expression:?} is already watched"));
        }
        self.entries.push(Watch::new(expression));
        Ok(self.entries.len() - 1)
    }

    /// Removes the watch at an index.
    pub fn remove(&mut self, index: usize) -> Result<(), String> {
        if index >= self.entries.len() {
            return Err(format!("there is no watch {index}"));
        }
        self.entries.remove(index);
        Ok(())
    }

    /// Every watch.
    pub fn entries(&self) -> &[Watch] {
        &self.entries
    }

    /// Records a result by index.
    pub fn record(&mut self, index: usize, result: Evaluation) -> Result<(), String> {
        let entry = self
            .entries
            .get_mut(index)
            .ok_or_else(|| format!("there is no watch {index}"))?;
        entry.result = Some(result);
        Ok(())
    }

    /// The watches in one line each.
    pub fn render(&self) -> Vec<String> {
        self.entries.iter().map(Watch::render).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EvalDecision, EvalPolicy, Evaluation, Frame, PAGE_LIMIT, Page, Variable, Watches, decide,
        page,
    };

    fn variable(name: &str, reference: i64) -> Variable {
        Variable {
            name: name.to_owned(),
            value: "1".to_owned(),
            type_name: Some("i32".to_owned()),
            variables_reference: reference,
            named: None,
            indexed: None,
        }
    }

    #[test]
    fn a_page_is_bounded_even_when_everything_was_asked_for() {
        let variables: Vec<Variable> = (0..250)
            .map(|index| variable(&format!("v{index}"), 0))
            .collect();
        let first = page(variables, 0, 10_000, Some(250));
        assert_eq!(first.variables.len(), PAGE_LIMIT);
        assert!(first.more);
        assert!(first.render().contains("more"));
        let second = page(
            (0..250)
                .map(|index| variable(&format!("v{index}"), 0))
                .collect(),
            200,
            10_000,
            Some(250),
        );
        assert_eq!(second.variables.len(), 50);
        assert!(!second.more);
    }

    #[test]
    fn a_variable_with_children_is_expanded_only_on_request() {
        let parent = variable("map", 42);
        assert!(parent.expandable());
        let page: Page = page(vec![parent.clone()], 0, 10, None);
        assert_eq!(page.variables[0].variables_reference, 42);
        assert_eq!(page.variables[0].render(), "map: i32 = 1");
    }

    #[test]
    fn evaluation_policy_refuses_asks_or_runs() {
        assert!(matches!(
            decide(EvalPolicy::Deny, "x"),
            EvalDecision::Refuse { .. }
        ));
        assert_eq!(decide(EvalPolicy::Allow, "f(x)"), EvalDecision::Run);
        assert!(matches!(
            decide(EvalPolicy::Ask, "x + 1"),
            EvalDecision::Run
        ));
        assert!(matches!(
            decide(EvalPolicy::Ask, "f(x)"),
            EvalDecision::Confirm { .. }
        ));
        assert!(matches!(
            decide(EvalPolicy::Allow, "  "),
            EvalDecision::Refuse { .. }
        ));
    }

    #[test]
    fn a_timeout_is_a_result_and_not_an_empty_value() {
        let timeout = Evaluation::TimedOut { milliseconds: 5000 };
        assert!(timeout.render().contains("timed out"));
        let error = Evaluation::Error {
            message: "no such variable".to_owned(),
        };
        assert!(error.render().contains("failed"));
    }

    #[test]
    fn a_watch_keeps_its_value_and_its_error_apart() {
        let mut watches = Watches::new();
        let index = watches.add("counter").expect("a watch");
        assert!(watches.add("counter").is_err());
        assert!(watches.add(" ").is_err());
        watches
            .record(
                index,
                Evaluation::Value {
                    value: "7".to_owned(),
                    type_name: Some("usize".to_owned()),
                    variables_reference: 0,
                },
            )
            .expect("a result");
        assert!(watches.render()[0].contains("usize 7"));
        watches
            .record(
                index,
                Evaluation::Error {
                    message: "out of scope".to_owned(),
                },
            )
            .expect("a result");
        assert!(watches.render()[0].contains("out of scope"));
    }

    #[test]
    fn a_frame_names_its_source_version_and_admits_a_missing_source() {
        let frame = Frame {
            id: 1,
            name: "run".to_owned(),
            file: Some("ui.rs".to_owned()),
            line: 12,
            column: 3,
            source_version: 9,
        };
        assert!(frame.render().contains("ui.rs:12:3"));
        assert!(frame.render().contains("source version 9"));
        let no_source = Frame {
            file: None,
            ..frame
        };
        assert!(no_source.render().contains("no generated source"));
    }
}
