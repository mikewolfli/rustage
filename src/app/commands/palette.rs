// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The command palette: fuzzy search, recents, argument completion and help (task T05.04).
//!
//! The acceptance line is *"a disabled command's reason is queryable, and a command can be found from
//! the keyboard"*. Both come from the palette being built **on the registry** rather than on a second
//! list it keeps in step:
//!
//! * **Every hit is a registry command, with the registry's own verdict.** [`search`] asks
//!   [`Registry::why_not`] for each result, so an entry that cannot run says why — the same sentence
//!   the menu (T05.02) would show — and one that can run has no reason. A palette that greyed an item
//!   out without saying why is exactly what this task forbids.
//! * **Fuzzy, but predictable.** [`score`] is a subsequence match over the command's id, its title and
//!   its category, favouring consecutive runs and word starts, and it is a pure function of the text:
//!   the same query gives the same ranking, so a person's fingers can learn it. A query that matches
//!   nothing by subsequence falls back to nothing rather than to everything.
//! * **Recents are a nudge, not a filter.** [`Recents`] records what was run and lifts those commands,
//!   but a command that was never run is still findable — a palette that hid unseen commands would make
//!   new features undiscoverable.
//! * **Completion is the command's own parameters.** [`help_for`] answers with the command's title, its
//!   help line and its typed parameters, taken from the registry entry, so the palette cannot offer an
//!   argument the command does not take.

// The palette's grouping, help accessors and the entry-to-command conversion are exercised by the
// tests until a palette surface (T04.07/T26) reads them; the silence is stated once here with its
// reason, as the neighbouring trees do.
#![allow(dead_code)]

use crate::app::commands::command::{Command, CommandId, ParamType, Registry};

use std::collections::BTreeSet;

/// One result in the palette.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    id: String,
    title: String,
    category: String,
    score: u32,
    enabled: bool,
    reason: Option<String>,
}

impl Entry {
    /// The command's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The command's title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The command's category.
    pub fn category(&self) -> &str {
        &self.category
    }

    /// The match score, higher first.
    pub const fn score(&self) -> u32 {
        self.score
    }

    /// Whether the command would run right now.
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Why it would not, when it would not.
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    /// The entry in one line, the way a palette row reads.
    pub fn render(&self) -> String {
        match &self.reason {
            None => format!("{:<18} {} ({})", self.id, self.title, self.category),
            Some(reason) => format!("{:<18} {} — {reason}", self.id, self.title),
        }
    }
}

/// A command that has been run, kept so the palette can offer it first.
#[derive(Clone, Debug, Default)]
pub struct Recents {
    order: Vec<String>,
    capacity: usize,
}

impl Recents {
    /// A recents list holding at most `capacity` commands.
    pub fn new(capacity: usize) -> Self {
        Self {
            order: Vec::new(),
            capacity,
        }
    }

    /// Records a run: the command moves to the front, and the list is capped.
    pub fn record(&mut self, id: impl Into<String>) {
        let id = id.into();
        self.order.retain(|existing| existing != &id);
        self.order.insert(0, id);
        self.order.truncate(self.capacity);
    }

    /// The recents, most recent first.
    pub fn list(&self) -> &[String] {
        &self.order
    }

    /// How recently a command was run: `None` when it never was, else its rank (0 is most recent).
    fn rank(&self, id: &str) -> Option<usize> {
        self.order.iter().position(|existing| existing == id)
    }
}

/// A fuzzy match score: `None` when `needle` is not a subsequence of `haystack`.
///
/// The scoring rewards a consecutive run and a word start, which is what makes `ss` find `file.save`
/// over `file.save-all` while `sav` finds both — a rule a person can feel rather than one that needs
/// reading.
pub fn score(haystack: &str, needle: &str) -> Option<u32> {
    if needle.is_empty() {
        return Some(0);
    }
    let haystack: Vec<char> = haystack.to_lowercase().chars().collect();
    let needle: Vec<char> = needle.to_lowercase().chars().collect();

    let mut total = 0u32;
    let mut hay = 0usize;
    let mut previous: Option<usize> = None;
    for wanted in &needle {
        let mut found = None;
        while hay < haystack.len() {
            if haystack[hay] == *wanted {
                found = Some(hay);
                hay += 1;
                break;
            }
            hay += 1;
        }
        let at = found?;
        total += 1;
        if previous == Some(at.wrapping_sub(1)) {
            // Consecutive: worth far more than a scattered match.
            total += 5;
        } else if at == 0
            || haystack
                .get(at - 1)
                .is_some_and(|previous| *previous == '.' || *previous == '-' || *previous == ' ')
        {
            // A word start: the beginning of the id, or just after a separator.
            total += 3;
        }
        previous = Some(at);
    }
    Some(total)
}

impl From<&Command> for Entry {
    fn from(command: &Command) -> Self {
        Self {
            id: command.id().to_string(),
            title: command.title().to_owned(),
            category: command.category().to_owned(),
            score: 0,
            enabled: true,
            reason: None,
        }
    }
}

/// Searches the registry, best match first, with each result's availability already resolved.
///
/// An empty query returns every command in id order (a browsable list); a non-empty query returns the
/// subsequence matches, ranked by [`score`] then by recency then by id, so the ordering is total and
/// reproducible.
pub fn search(
    registry: &Registry,
    context: &crate::app::commands::command::Context,
    query: &str,
    recents: &Recents,
) -> Vec<Entry> {
    let mut entries: Vec<Entry> = registry
        .commands()
        .filter_map(|command| {
            let id = command.id().to_string();
            let best = if query.is_empty() {
                Some(0)
            } else {
                [id.as_str(), command.title(), command.category()]
                    .iter()
                    .filter_map(|haystack| score(haystack, query))
                    .max()
            };
            let matched = best?;
            let call = crate::app::commands::command::Call::to(id.clone());
            let reason = registry.why_not(&call, context);
            Some(Entry {
                id,
                title: command.title().to_owned(),
                category: command.category().to_owned(),
                score: matched,
                enabled: reason.is_none(),
                reason,
            })
        })
        .collect();

    entries.sort_by(|a, b| {
        let recency = |entry: &Entry| recents.rank(&entry.id).unwrap_or(usize::MAX);
        b.score
            .cmp(&a.score)
            .then_with(|| recency(a).cmp(&recency(b)))
            .then_with(|| a.id.cmp(&b.id))
    });
    entries
}

/// A command's help, with its parameters, for argument completion.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Help {
    id: String,
    title: String,
    category: String,
    description: String,
    params: Vec<ParamHelp>,
}

/// One parameter in a command's help.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ParamHelp {
    name: String,
    kind: String,
    required: bool,
    help: String,
    choices: Vec<String>,
}

impl ParamHelp {
    /// The parameter's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its type, as the registry names it.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// Whether it must be given.
    pub const fn is_required(&self) -> bool {
        self.required
    }

    /// The one-line help.
    pub fn help(&self) -> &str {
        &self.help
    }

    /// The allowed values, for a choice parameter.
    pub fn choices(&self) -> &[String] {
        &self.choices
    }
}

impl Help {
    /// The command's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Its title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Its parameters, in the order the command declared them.
    pub fn params(&self) -> &[ParamHelp] {
        &self.params
    }

    /// The help in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("{} — {} ({})", self.id, self.title, self.category)];
        if !self.description.is_empty() {
            lines.push(format!("  {}", self.description));
        }
        for param in &self.params {
            let required = if param.required {
                "required"
            } else {
                "optional"
            };
            let choices = if param.choices.is_empty() {
                String::new()
            } else {
                format!(": {}", param.choices.join(", "))
            };
            lines.push(format!(
                "  {} ({required}, {}){choices} — {}",
                param.name, param.kind, param.help
            ));
        }
        lines
    }
}

/// A command's help, or why it has none.
pub fn help_for(registry: &Registry, id: &str) -> Result<Help, String> {
    let Some(parsed) = CommandId::parse(id) else {
        return Err(format!("{id:?} is not a command id"));
    };
    let Some(command) = registry.command(&parsed) else {
        return Err(format!("there is no command {id}"));
    };
    Ok(Help {
        id: command.id().to_string(),
        title: command.title().to_owned(),
        category: command.category().to_owned(),
        description: command.help().to_owned(),
        params: command
            .params()
            .iter()
            .map(|param| ParamHelp {
                name: param.name().to_owned(),
                kind: param.kind().name().to_owned(),
                required: param.is_required(),
                help: param.help().to_owned(),
                choices: match param.kind() {
                    ParamType::Choice(options) => options.clone(),
                    _ => Vec::new(),
                },
            })
            .collect(),
    })
}

/// The categories the registry has, in id order, for a palette's grouping.
pub fn categories(registry: &Registry) -> Vec<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for command in registry.commands() {
        seen.insert(command.category().to_owned());
    }
    seen.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::commands::command::{Context, built_in};
    use crate::contracts::permission::EffectClass;

    #[test]
    fn a_query_finds_a_command_from_the_keyboard_and_ranks_tight_matches_first() {
        let registry = built_in();
        let context = Context::in_area("design");
        let recents = Recents::new(5);

        let hits = search(&registry, &context, "save", &recents);
        let ids: Vec<&str> = hits.iter().map(|entry| entry.id()).collect();
        assert!(ids.contains(&"file.save"), "{ids:?}");
        assert!(ids.contains(&"file.save-all"), "{ids:?}");
        assert!(!ids.contains(&"edit.undo"), "{ids:?}");

        let hits = search(&registry, &context, "undo", &recents);
        assert_eq!(hits[0].id(), "edit.undo");
    }

    #[test]
    fn a_disabled_command_appears_with_its_reason_and_an_enabled_one_has_none() {
        let registry = built_in();
        // Not trusted, nothing authorized: build.run is disabled and says why.
        let context = Context::in_area("design");
        let hits = search(&registry, &context, "run", &Recents::new(5));
        let build = hits
            .iter()
            .find(|entry| entry.id() == "build.run")
            .expect("build.run is a hit");
        assert!(!build.is_enabled());
        assert!(build.reason().unwrap().contains("not trusted"), "{build:?}");
        assert!(build.render().contains("not trusted"));

        // file.save is always available and carries no reason.
        let save = search(&registry, &context, "file.save", &Recents::new(5))
            .into_iter()
            .find(|entry| entry.id() == "file.save")
            .expect("file.save is a hit");
        assert!(save.is_enabled());
        assert!(save.reason().is_none());
    }

    #[test]
    fn an_empty_query_lists_everything_so_a_new_command_is_discoverable() {
        let registry = built_in();
        let context = Context::in_area("design")
            .trusted()
            .authorizing(EffectClass::Command);
        let all = search(&registry, &context, "", &Recents::new(5));
        assert_eq!(all.len(), registry.len());
        for command in registry.commands() {
            assert!(
                all.iter()
                    .any(|entry| entry.id() == command.id().to_string()),
                "{} is missing",
                command.id()
            );
        }
    }

    #[test]
    fn recents_lift_a_command_but_do_not_hide_the_others() {
        let registry = built_in();
        let context = Context::in_area("design");
        let mut recents = Recents::new(3);
        recents.record("edit.redo");

        // "e" matches several commands; the recent one comes first among equal scores.
        let hits = search(&registry, &context, "e", &recents);
        assert_eq!(
            hits[0].id(),
            "edit.redo",
            "{:?}",
            hits.iter().map(Entry::id).collect::<Vec<_>>()
        );
        // And a command that was never run is still found.
        assert!(
            search(&registry, &context, "welcome", &recents)
                .iter()
                .any(|entry| entry.id() == "view.welcome")
        );
    }

    #[test]
    fn recents_are_capped_and_move_a_repeat_to_the_front() {
        let mut recents = Recents::new(2);
        recents.record("a.one");
        recents.record("a.two");
        recents.record("a.three");
        assert_eq!(recents.list(), ["a.three", "a.two"]);
        recents.record("a.two");
        assert_eq!(recents.list(), ["a.two", "a.three"]);
    }

    #[test]
    fn the_score_prefers_consecutive_and_word_starting_matches() {
        // A consecutive run beats a scattered one.
        assert!(score("file.save", "save").unwrap() > score("sampler.vase", "save").unwrap());
        // A word start (after a dot) beats a match in the middle.
        assert!(score("file.save", "sav").unwrap() > score("assets.ave", "sav").unwrap());
        // Not a subsequence at all.
        assert!(score("file.save", "xyz").is_none());
        // Empty query matches everything with nothing added.
        assert_eq!(score("anything", ""), Some(0));
    }

    #[test]
    fn a_command_that_matches_nothing_is_not_offered() {
        let registry = built_in();
        let context = Context::in_area("design");
        assert!(search(&registry, &context, "zzzz", &Recents::new(5)).is_empty());
    }

    #[test]
    fn help_answers_with_the_commands_own_parameters() {
        let registry = built_in();
        let help = help_for(&registry, "file.close").expect("a command with parameters");
        assert_eq!(help.title(), "Close");
        assert_eq!(help.params().len(), 1);
        let param = &help.params()[0];
        assert_eq!(param.name(), "decision");
        assert!(!param.is_required(), "the decision is optional");
        assert_eq!(param.choices(), ["save-all", "discard-all", "cancel"]);
        assert!(
            help.render()
                .join("\n")
                .contains("save-all, discard-all, cancel")
        );

        assert!(
            help_for(&registry, "file.save")
                .unwrap()
                .params()
                .is_empty()
        );
        assert!(help_for(&registry, "nothing.here").is_err());
        assert!(help_for(&registry, "not an id").is_err());
    }

    #[test]
    fn the_categories_are_the_registrys_own_and_in_order() {
        let registry = built_in();
        let categories = categories(&registry);
        assert!(categories.contains(&"File".to_owned()));
        assert!(categories.contains(&"Build".to_owned()));
        // No duplicates, and sorted (BTreeSet order).
        let mut sorted = categories.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(categories, sorted);
    }
}
