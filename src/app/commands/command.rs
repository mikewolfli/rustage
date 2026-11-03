// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The typed command registry (task T05.01): every command a person can run, described once, and the
//! one gate a call has to pass.
//!
//! The acceptance line is *"an unauthorized or badly-parameterised command cannot bypass the menu and
//! run anyway"*. The word that matters is **bypass**, and it is answered structurally rather than by
//! discipline:
//!
//! * **A command is a value, not a string switch.** A [`Command`] carries its id, its title, its
//!   category, its typed [`Param`]s, the [`EffectClass`]es it has, the undo domain its change lands
//!   in, and the task it belongs to. Registration refuses a duplicate id, so "which command is this"
//!   has one answer.
//! * **There is exactly one way to run something.** [`Registry::resolve`] turns a raw [`Call`] (what a
//!   menu item, a shortcut or a command line produces) plus a [`Context`] into an [`Invocation`], and
//!   an `Invocation` **cannot be built any other way** — its fields are private and it has no public
//!   constructor. A caller with an `Invocation` holds proof that the id existed, the context allowed
//!   it, the effects were authorized, and every argument parsed. A caller without one has nothing to
//!   run. That is the difference between "the menu checks" and "nothing else can run".
//! * **Every refusal says which of the four things went wrong.** Unknown id, a context that disables
//!   it (focus, trust, a task already running), a bad or missing argument, and an unauthorized effect
//!   are four different findings, and each names the command and the reason. A disabled command and an
//!   unauthorized one are not the same failure: the first is "not here, not now", the second is "not
//!   you" — and the roadmap asks for exactly that distinction.
//!
//! Nothing here reaches a GUI. A menu (T05.02), a shortcut (T05.03) and a command palette (T05.04)
//! all produce [`Call`]s and all go through [`Registry::resolve`]; the registry is where "can this run"
//! is decided once.

// A command's metadata is what a menu (T05.02), a shortcut (T05.03), a palette (T05.04) and a
// toolbar (T04.07) all read, and those consumers arrive with their own tasks; until then most of
// this surface is exercised by the tests rather than by a running caller, which is why the silence
// is here and says so rather than being spread over a dozen `#[allow]`s.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::contracts::permission::EffectClass;

/// A command's identity: a namespace and an action, as `命名空间.动作` (the `P-CMD-ID` convention).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct CommandId {
    namespace: String,
    action: String,
}

impl CommandId {
    /// Parses `namespace.action`.
    ///
    /// Exactly one dot and two non-empty halves, because a command id is a name a person reads in a
    /// menu and a key a shortcut binds to: `file.save` is one command, `file..save` and `file` are
    /// neither.
    pub fn parse(text: &str) -> Option<Self> {
        let (namespace, action) = text.split_once('.')?;
        if namespace.is_empty() || action.is_empty() || action.contains('.') {
            return None;
        }
        if !namespace
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '-')
            || !action.chars().all(|c| c.is_ascii_lowercase() || c == '-')
        {
            return None;
        }
        Some(Self {
            namespace: namespace.to_owned(),
            action: action.to_owned(),
        })
    }

    /// The namespace, which is also the command's group in the menu.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The action.
    pub fn action(&self) -> &str {
        &self.action
    }
}

impl fmt::Display for CommandId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.namespace, self.action)
    }
}

/// What a parameter holds.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ParamType {
    /// Any text.
    Text,
    /// A whole number.
    Integer,
    /// `true` or `false`.
    Bool,
    /// One of a fixed set, in the order the help shows them.
    Choice(Vec<String>),
    /// A node identity.
    NodeId,
}

impl ParamType {
    /// The word a report uses.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Integer => "integer",
            Self::Bool => "true/false",
            Self::Choice(_) => "choice",
            Self::NodeId => "node id",
        }
    }

    /// Parses one value, or says why it is not one.
    fn parse(&self, raw: &str) -> Result<Value, String> {
        match self {
            Self::Text => Ok(Value::Text(raw.to_owned())),
            Self::Integer => raw
                .parse::<i64>()
                .map(Value::Integer)
                .map_err(|error| error.to_string()),
            Self::Bool => match raw {
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                other => Err(format!("{other:?} is not true or false")),
            },
            Self::Choice(options) => {
                if options.iter().any(|option| option == raw) {
                    Ok(Value::Text(raw.to_owned()))
                } else {
                    Err(format!("{raw:?} is not one of {}", options.join(", ")))
                }
            }
            Self::NodeId => {
                if raw.is_empty() {
                    Err("a node id cannot be empty".to_owned())
                } else {
                    Ok(Value::Text(raw.to_owned()))
                }
            }
        }
    }
}

/// A parsed argument value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Value {
    /// Text, a choice or a node id.
    Text(String),
    /// A whole number.
    Integer(i64),
    /// A flag.
    Bool(bool),
}

impl Value {
    /// The value as text, for a shortcut key or a report.
    pub fn as_text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Integer(number) => number.to_string(),
            Self::Bool(flag) => flag.to_string(),
        }
    }
}

/// One parameter a command takes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Param {
    name: String,
    kind: ParamType,
    required: bool,
    help: String,
}

impl Param {
    /// A required parameter.
    pub fn required(name: impl Into<String>, kind: ParamType, help: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind,
            required: true,
            help: help.into(),
        }
    }

    /// An optional parameter.
    pub fn optional(name: impl Into<String>, kind: ParamType, help: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind,
            required: false,
            help: help.into(),
        }
    }

    /// The parameter's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What it holds.
    pub const fn kind(&self) -> &ParamType {
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
}

/// Which undo domain a command's change lands in, which is what makes one gesture one step back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Undo {
    /// The command changes nothing that is undone.
    None,
    /// The change is a text edit.
    Text,
    /// The change is a model change set.
    Model,
    /// The command's own result is a whole gesture (a drag) held as one step.
    Gesture,
}

impl Undo {
    /// The word a report uses.
    pub const fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Text => "text",
            Self::Model => "model",
            Self::Gesture => "gesture",
        }
    }
}

/// Where a command is available.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Focus {
    /// Anywhere.
    Any,
    /// Only in the named area (a mode id, a panel id).
    Only(String),
}

/// One command, described once.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Command {
    id: CommandId,
    title: String,
    category: String,
    params: Vec<Param>,
    effects: Vec<EffectClass>,
    undo: Undo,
    task: Option<String>,
    needs_trust: bool,
    /// Refuses while the focused editor has nothing to undo.
    needs_undo: bool,
    /// Refuses while the focused editor has nothing to redo.
    needs_redo: bool,
    focus: Focus,
    blocks_on: Option<String>,
    help: String,
}

impl Command {
    /// A command with no parameters and no effects.
    pub fn new(id: CommandId, title: impl Into<String>, category: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            category: category.into(),
            params: Vec::new(),
            effects: Vec::new(),
            undo: Undo::None,
            task: None,
            needs_trust: false,
            needs_undo: false,
            needs_redo: false,
            focus: Focus::Any,
            blocks_on: None,
            help: String::new(),
        }
    }

    /// Adds a parameter.
    pub fn with_param(mut self, param: Param) -> Self {
        self.params.push(param);
        self
    }

    /// Declares an external effect the command has.
    pub fn with_effect(mut self, effect: EffectClass) -> Self {
        if !self.effects.contains(&effect) {
            self.effects.push(effect);
        }
        self
    }

    /// Declares the undo domain its change lands in.
    pub fn with_undo(mut self, undo: Undo) -> Self {
        self.undo = undo;
        self
    }

    /// Names the task the command runs as, so a second run can be refused while one is going.
    pub fn with_task(mut self, task: impl Into<String>) -> Self {
        self.task = Some(task.into());
        self
    }

    /// Requires the project to be trusted.
    pub fn needing_trust(mut self) -> Self {
        self.needs_trust = true;
        self
    }

    /// Refuses while the focused editor's own history says there is nothing to undo.
    ///
    /// The fact comes from the control (`CodeEditor::can_undo`), not from a host counter, so a
    /// document whose last edit was undone right back to its saved text answers honestly.
    pub fn needing_undo(mut self) -> Self {
        self.needs_undo = true;
        self
    }

    /// Refuses while the focused editor's own history says there is nothing to redo.
    pub fn needing_redo(mut self) -> Self {
        self.needs_redo = true;
        self
    }

    /// Makes the command available only in one area.
    pub fn in_focus(mut self, area: impl Into<String>) -> Self {
        self.focus = Focus::Only(area.into());
        self
    }

    /// Refuses the command while another command's task of this name is running.
    pub fn blocked_while(mut self, task: impl Into<String>) -> Self {
        self.blocks_on = Some(task.into());
        self
    }

    /// A one-line description.
    pub fn described_as(mut self, help: impl Into<String>) -> Self {
        self.help = help.into();
        self
    }

    /// The command's id.
    pub fn id(&self) -> &CommandId {
        &self.id
    }

    /// The title a menu shows.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The category (the menu group).
    pub fn category(&self) -> &str {
        &self.category
    }

    /// Its parameters, in the order the help shows them.
    pub fn params(&self) -> &[Param] {
        &self.params
    }

    /// Its external effects.
    pub fn effects(&self) -> &[EffectClass] {
        &self.effects
    }

    /// Its undo domain.
    pub const fn undo(&self) -> Undo {
        self.undo
    }

    /// The task it runs as, when it runs as one.
    pub fn task(&self) -> Option<&str> {
        self.task.as_deref()
    }

    /// Whether it needs the project to be trusted.
    pub const fn needs_trust(&self) -> bool {
        self.needs_trust
    }

    /// Whether the command needs the focused editor to have something to undo.
    pub const fn needs_undo(&self) -> bool {
        self.needs_undo
    }

    /// Whether the command needs the focused editor to have something to redo.
    pub const fn needs_redo(&self) -> bool {
        self.needs_redo
    }

    /// The one-line help.
    pub fn help(&self) -> &str {
        &self.help
    }
}

/// The raw call a menu item, a shortcut or a command line produces.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Call {
    id: String,
    arguments: BTreeMap<String, String>,
}

impl Call {
    /// A call to a command by id.
    pub fn to(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            arguments: BTreeMap::new(),
        }
    }

    /// The same call with one argument.
    pub fn with(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.arguments.insert(name.into(), value.into());
        self
    }

    /// The command the call names.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The raw arguments.
    pub fn arguments(&self) -> &BTreeMap<String, String> {
        &self.arguments
    }
}

/// What is true of the host right now, which decides availability.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Context {
    /// Where the keyboard is: a mode id or a panel id.
    focus: String,
    /// Whether the project is trusted.
    trusted: bool,
    /// The effects the person has authorized for this project.
    authorized: BTreeSet<EffectClass>,
    /// The tasks that are running now.
    running: BTreeSet<String>,
    /// Whether the focused editor can undo, when the caller knows.
    ///
    /// `None` means "the caller did not say", which leaves a history command enabled: guessing a
    /// disabled state from a fact nobody supplied would grey out a command that may well work. The
    /// window supplies the editor's own `can_undo`/`can_redo` on every dispatch.
    can_undo: Option<bool>,
    /// The redo half of [`Self::can_undo`].
    can_redo: Option<bool>,
}

impl Context {
    /// A context in an area, with nothing trusted and nothing authorized.
    pub fn in_area(focus: impl Into<String>) -> Self {
        Self {
            focus: focus.into(),
            ..Self::default()
        }
    }

    /// The same context with the project trusted.
    pub fn trusted(mut self) -> Self {
        self.trusted = true;
        self
    }

    /// The same context with an effect authorized.
    pub fn authorizing(mut self, effect: EffectClass) -> Self {
        self.authorized.insert(effect);
        self
    }

    /// The same context with a task running.
    pub fn running(mut self, task: impl Into<String>) -> Self {
        self.running.insert(task.into());
        self
    }

    /// The same context with what the focused editor's history can do.
    ///
    /// `None` for either half means the caller has no answer, which leaves the command enabled
    /// rather than inventing a refusal.
    pub fn with_history(mut self, can_undo: Option<bool>, can_redo: Option<bool>) -> Self {
        self.can_undo = can_undo;
        self.can_redo = can_redo;
        self
    }

    /// Whether the focused editor can undo, when the caller said.
    pub const fn undo_state(&self) -> Option<bool> {
        self.can_undo
    }

    /// Whether the focused editor can redo, when the caller said.
    pub const fn redo_state(&self) -> Option<bool> {
        self.can_redo
    }

    /// Where the keyboard is.
    pub fn focus(&self) -> &str {
        &self.focus
    }

    /// Whether the project is trusted.
    pub const fn is_trusted(&self) -> bool {
        self.trusted
    }
}

/// Why a call could not become an invocation.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// No command has that id.
    Unknown {
        /// The id the call named.
        id: String,
    },
    /// The context does not allow it.
    /// The command exists but is not available here and now.
    Disabled {
        /// The command.
        id: CommandId,
        /// Why, as a value rather than a finished sentence.
        ///
        /// The sentence depends on the interface language, which this layer does not own; the
        /// palette, the menu and the log all read the same reason.
        reason: Disabled,
    },
    /// A required parameter was not given.
    Missing {
        /// The command.
        id: CommandId,
        /// The parameter.
        name: String,
    },
    /// A parameter was given but does not parse.
    BadArgument {
        /// The command.
        id: CommandId,
        /// The parameter.
        name: String,
        /// What the type said.
        reason: String,
    },
    /// An effect the command has is not authorized.
    Unauthorized {
        /// The command.
        id: CommandId,
        /// The effect that was not authorized.
        effect: EffectClass,
    },
}

/// Why a command is not available here and now.
///
/// A value rather than a finished sentence: the sentence has to be in the interface language, and
/// this layer only knows what is true. [`Self::localized`] reads the catalogue; [`Self::message`]
/// keeps an English form for logs and diagnostics that must not change with the UI language.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Disabled {
    /// The keyboard is in another area.
    Focus {
        /// The one area the command is available in.
        area: String,
        /// Where the keyboard is.
        focus: String,
    },
    /// The project is not trusted.
    Untrusted,
    /// A task the command waits on is running.
    Running {
        /// The task.
        task: String,
    },
    /// The focused editor has nothing to undo.
    NothingToUndo,
    /// The focused editor has nothing to redo.
    NothingToRedo,
}

impl Disabled {
    /// The reason in the interface language.
    pub fn localized(&self) -> String {
        let catalog = crate::app::locale::catalog::Catalog::current();
        match self {
            Self::Focus { area, focus } => catalog.fill(
                "cmd.reason.focus",
                &[("area", area.as_str()), ("focus", focus.as_str())],
            ),
            Self::Untrusted => catalog.fill("cmd.reason.untrusted", &[]),
            Self::Running { task } => {
                catalog.fill("cmd.reason.running", &[("task", task.as_str())])
            }
            Self::NothingToUndo => catalog.fill("cmd.reason.nothing-to-undo", &[]),
            Self::NothingToRedo => catalog.fill("cmd.reason.nothing-to-redo", &[]),
        }
    }

    /// The reason in English, for a log line or a report that is not the interface.
    pub fn message(&self) -> String {
        match self {
            Self::Focus { area, focus } => {
                format!("it is only available in {area}, and the keyboard is in {focus}")
            }
            Self::Untrusted => "the project is not trusted".to_owned(),
            Self::Running { task } => format!("{task} is already running"),
            Self::NothingToUndo => "the focused editor has nothing to undo".to_owned(),
            Self::NothingToRedo => "the focused editor has nothing to redo".to_owned(),
        }
    }
}

impl Refusal {
    /// The refusal in one sentence.
    ///
    /// The sentence comes from the interface catalog (T24.03), so a translated pack changes the
    /// diagnostic a person reads; the language is the process's installed one
    /// ([`crate::app::locale::catalog::Catalog::current`]), and the reason a caller passed in stays as
    /// it was.
    pub fn message(&self) -> String {
        let catalog = crate::app::locale::catalog::Catalog::current();
        match self {
            Self::Unknown { id } => catalog.fill("diag.command.unknown", &[("id", id)]),
            Self::Disabled { id, reason } => {
                let id = id.to_string();
                let reason = reason.localized();
                catalog.fill(
                    "diag.command.disabled",
                    &[("id", id.as_str()), ("reason", reason.as_str())],
                )
            }
            Self::Missing { id, name } => {
                let id = id.to_string();
                catalog.fill(
                    "diag.command.missing",
                    &[("id", id.as_str()), ("name", name)],
                )
            }
            Self::BadArgument { id, name, reason } => {
                let id = id.to_string();
                catalog.fill(
                    "diag.command.bad-argument",
                    &[("id", id.as_str()), ("name", name), ("reason", reason)],
                )
            }
            Self::Unauthorized { id, effect } => {
                let id = id.to_string();
                catalog.fill(
                    "diag.command.unauthorized",
                    &[("id", id.as_str()), ("effect", effect.name())],
                )
            }
        }
    }
}

/// A command that has passed the gate, with its arguments parsed and its metadata resolved.
///
/// There is no public constructor and the fields are private: the only way to hold one is to have
/// been given it by [`Registry::resolve`], so a caller holding an `Invocation` holds proof that the
/// command existed, the context allowed it, the effects were authorized and the arguments were valid.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Invocation {
    id: CommandId,
    title: String,
    arguments: BTreeMap<String, Value>,
    undo: Undo,
    task: Option<String>,
}

impl Invocation {
    /// The command to run.
    pub fn id(&self) -> &CommandId {
        &self.id
    }

    /// Its title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// A parsed argument.
    pub fn argument(&self, name: &str) -> Option<&Value> {
        self.arguments.get(name)
    }

    /// The undo domain the change lands in.
    pub const fn undo(&self) -> Undo {
        self.undo
    }

    /// The task it runs as, when it runs as one.
    pub fn task(&self) -> Option<&str> {
        self.task.as_deref()
    }

    /// The invocation in one line, for a report or a log.
    pub fn describe(&self) -> String {
        if self.arguments.is_empty() {
            format!("{} ({})", self.id, self.undo.name())
        } else {
            let arguments: Vec<String> = self
                .arguments
                .iter()
                .map(|(name, value)| format!("{name}={}", value.as_text()))
                .collect();
            format!(
                "{} ({}; {})",
                self.id,
                self.undo.name(),
                arguments.join(", ")
            )
        }
    }
}

/// Every command the host knows, and the gate they all pass through.
#[derive(Clone, Debug, Default)]
pub struct Registry {
    commands: BTreeMap<CommandId, Command>,
}

/// Why a command could not be registered.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RegistrationRefusal {
    id: CommandId,
    reason: String,
}

impl RegistrationRefusal {
    /// The id that was refused.
    pub fn id(&self) -> &CommandId {
        &self.id
    }

    /// Why.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a command, refusing a duplicate id.
    ///
    /// A duplicate is refused rather than replaced: two commands with one id would make "which one
    /// runs" depend on registration order, and a shortcut bound to that id would run whichever was
    /// registered last.
    pub fn register(&mut self, command: Command) -> Result<(), RegistrationRefusal> {
        let id = command.id().clone();
        if self.commands.contains_key(&id) {
            return Err(RegistrationRefusal {
                id,
                reason: "a command with this id is already registered".to_owned(),
            });
        }
        self.commands.insert(id, command);
        Ok(())
    }

    /// The command with an id, when it is registered.
    pub fn command(&self, id: &CommandId) -> Option<&Command> {
        self.commands.get(id)
    }

    /// Every command, in id order.
    pub fn commands(&self) -> impl Iterator<Item = &Command> {
        self.commands.values()
    }

    /// How many commands are registered.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Whether a call could run right now, and why not when it could not (§T05.04's "the reason is
    /// queryable"). A disabled command answers with its reason; an enabled one answers `None`.
    pub fn why_not(&self, call: &Call, context: &Context) -> Option<String> {
        match self.resolve(call, context) {
            Ok(_) => None,
            Err(refusal) => Some(refusal.message()),
        }
    }

    /// Whether an id names a registered command, and why not when it does not.
    ///
    /// The menu tree (T05.02) asks this: a leaf that claims a command has to name one that exists,
    /// and this is the registry's own answer rather than a second list the menu keeps.
    pub fn why_not_exists(&self, id: &str) -> Option<String> {
        let catalog = crate::app::locale::catalog::Catalog::current();
        match CommandId::parse(id) {
            Some(parsed) if self.commands.contains_key(&parsed) => None,
            Some(_) => Some(catalog.fill("diag.command.no-such-command", &[("id", id)])),
            None => Some(catalog.fill("diag.command.not-an-id", &[("id", id)])),
        }
    }

    /// Turns a raw call into an invocation, or refuses it.
    ///
    /// This is the only door. The checks are in the order a person would want them reported: is the
    /// command known, is it available here and now, does the person have the authority, and only then
    /// are the arguments read — because "not available" is a more useful answer than "one argument
    /// would have been wrong" for a command that was never going to run.
    pub fn resolve(&self, call: &Call, context: &Context) -> Result<Invocation, Refusal> {
        let Some(parsed) = CommandId::parse(call.id()) else {
            return Err(Refusal::Unknown {
                id: call.id().to_owned(),
            });
        };
        let Some(command) = self.commands.get(&parsed) else {
            return Err(Refusal::Unknown {
                id: call.id().to_owned(),
            });
        };

        if let Focus::Only(area) = &command.focus
            && context.focus() != area
        {
            return Err(Refusal::Disabled {
                id: parsed.clone(),
                reason: Disabled::Focus {
                    area: area.clone(),
                    focus: context.focus().to_owned(),
                },
            });
        }
        if command.needs_trust && !context.is_trusted() {
            return Err(Refusal::Disabled {
                id: parsed.clone(),
                reason: Disabled::Untrusted,
            });
        }
        if let Some(task) = &command.blocks_on
            && context.running.contains(task)
        {
            return Err(Refusal::Disabled {
                id: parsed.clone(),
                reason: Disabled::Running { task: task.clone() },
            });
        }
        if command.needs_undo && context.undo_state() == Some(false) {
            return Err(Refusal::Disabled {
                id: parsed.clone(),
                reason: Disabled::NothingToUndo,
            });
        }
        if command.needs_redo && context.redo_state() == Some(false) {
            return Err(Refusal::Disabled {
                id: parsed.clone(),
                reason: Disabled::NothingToRedo,
            });
        }
        for effect in &command.effects {
            if !context.authorized.contains(effect) {
                return Err(Refusal::Unauthorized {
                    id: parsed.clone(),
                    effect: *effect,
                });
            }
        }

        let mut arguments = BTreeMap::new();
        for (name, raw) in call.arguments() {
            let Some(param) = command.params.iter().find(|param| param.name() == name) else {
                return Err(Refusal::BadArgument {
                    id: parsed.clone(),
                    name: name.clone(),
                    reason: "there is no such parameter".to_owned(),
                });
            };
            match param.kind().parse(raw) {
                Ok(value) => {
                    arguments.insert(name.clone(), value);
                }
                Err(reason) => {
                    return Err(Refusal::BadArgument {
                        id: parsed.clone(),
                        name: name.clone(),
                        reason,
                    });
                }
            }
        }
        for param in &command.params {
            if param.is_required() && !arguments.contains_key(param.name()) {
                return Err(Refusal::Missing {
                    id: parsed.clone(),
                    name: param.name().to_owned(),
                });
            }
        }

        Ok(Invocation {
            id: parsed,
            title: command.title.clone(),
            arguments,
            undo: command.undo,
            task: command.task.clone(),
        })
    }
}

/// The panel ids §2.2 requires, in the ledger's order, as `view.panel-show` accepts them.
///
/// The command takes the ledger id (`panel.pages`), not the window's host id (`pages`): §2.2 is the
/// roadmap's own vocabulary, and a second list here would drift from it.
fn panel_choices() -> Vec<String> {
    crate::app::shell::panels::REQUIRED_PANELS
        .iter()
        .map(|panel| panel.id().to_owned())
        .collect()
}

/// The commands this build registers: the ones the roadmap's §3 menu tree names and that have a
/// handler today. Registering a command here is a claim that it exists, so the set is deliberately
/// small — the id convention (`P-CMD-ID`) fixes 16 namespaces, but a namespace is not a command.
pub fn built_in() -> Registry {
    let mut registry = Registry::new();
    let commands = [
        Command::new(
            CommandId::parse("file.close").expect("a valid id"),
            "Close",
            "File",
        )
        .with_param(Param::optional(
            "decision",
            ParamType::Choice(vec![
                "save-all".to_owned(),
                "discard-all".to_owned(),
                "cancel".to_owned(),
            ]),
            "what to do with unsaved changes",
        ))
        .described_as("close the current document, deciding what to do with unsaved changes"),
        Command::new(
            CommandId::parse("file.close-others").expect("a valid id"),
            "Close Others",
            "File",
        )
        .described_as("close every other document in the focused editor group"),
        Command::new(
            CommandId::parse("file.close-saved").expect("a valid id"),
            "Close Saved",
            "File",
        )
        .with_undo(Undo::None)
        .described_as("close every document that has no unsaved changes"),
        Command::new(
            CommandId::parse("file.close-project").expect("a valid id"),
            "Close Project",
            "File",
        )
        .with_undo(Undo::None)
        .described_as("close the open project after its documents have no unsaved changes"),
        Command::new(
            CommandId::parse("file.open-reopen").expect("a valid id"),
            "Reopen Closed Document",
            "File",
        )
        .with_undo(Undo::None)
        .described_as("reopen the most recently closed document"),
        Command::new(
            CommandId::parse("file.exit").expect("a valid id"),
            "Exit",
            "File",
        )
        .described_as("close the window and end the session"),
        Command::new(
            CommandId::parse("file.copy-path").expect("a valid id"),
            "Copy Project Path",
            "File",
        )
        .with_undo(Undo::None)
        .described_as("copy the open project's root path to the clipboard"),
        Command::new(
            CommandId::parse("file.reveal").expect("a valid id"),
            "Reveal Project",
            "File",
        )
        .with_undo(Undo::None)
        .described_as("hand the project root to the system's file manager"),
        Command::new(
            CommandId::parse("file.save").expect("a valid id"),
            "Save",
            "File",
        )
        .with_undo(Undo::None)
        .with_effect(EffectClass::FileWrite)
        .needing_trust()
        .described_as("write the focused document back to the file it was opened from"),
        Command::new(
            CommandId::parse("file.save-all").expect("a valid id"),
            "Save All",
            "File",
        )
        .with_undo(Undo::None)
        .with_effect(EffectClass::FileWrite)
        .needing_trust()
        .described_as("write every open document that came from disk"),
        Command::new(
            CommandId::parse("file.save-as").expect("a valid id"),
            "Save As",
            "File",
        )
        .with_undo(Undo::None)
        .with_effect(EffectClass::FileWrite)
        .needing_trust()
        .described_as("write the focused document under a new project path"),
        Command::new(
            CommandId::parse("file.save-copy").expect("a valid id"),
            "Save Copy",
            "File",
        )
        .with_undo(Undo::None)
        .with_effect(EffectClass::FileWrite)
        .needing_trust()
        .described_as("write a copy without changing the active document"),
        Command::new(
            CommandId::parse("edit.undo").expect("a valid id"),
            "Undo",
            "Edit",
        )
        .with_undo(Undo::None)
        .described_as("take back the newest change, in its own domain")
        .needing_undo(),
        Command::new(
            CommandId::parse("edit.cut").expect("a valid id"),
            "Cut",
            "Edit",
        )
        .with_undo(Undo::Text)
        .described_as("move the editor selection to the clipboard"),
        Command::new(
            CommandId::parse("edit.copy").expect("a valid id"),
            "Copy",
            "Edit",
        )
        .with_undo(Undo::None)
        .described_as("copy the editor selection to the clipboard"),
        Command::new(
            CommandId::parse("edit.copy-node-id").expect("a valid id"),
            "Copy Node ID",
            "Edit",
        )
        .with_undo(Undo::None)
        .described_as("copy the selected design node's stable id"),
        Command::new(
            CommandId::parse("edit.history-view").expect("a valid id"),
            "Operation History",
            "Edit",
        )
        .with_undo(Undo::None)
        .described_as("show the commands this window executed, newest first"),
        Command::new(
            CommandId::parse("edit.paste").expect("a valid id"),
            "Paste",
            "Edit",
        )
        .with_undo(Undo::Text)
        .described_as("paste the clipboard at the caret"),
        Command::new(
            CommandId::parse("nav.design-parent").expect("a valid id"),
            "Select Parent",
            "Navigate",
        )
        .with_undo(Undo::None)
        .described_as("select the selected design node's parent"),
        Command::new(
            CommandId::parse("nav.design-child").expect("a valid id"),
            "Select First Child",
            "Navigate",
        )
        .with_undo(Undo::None)
        .described_as("select the selected design node's first child"),
        Command::new(
            CommandId::parse("nav.design-next-sibling").expect("a valid id"),
            "Select Next Sibling",
            "Navigate",
        )
        .with_undo(Undo::None)
        .described_as("select the selected design node's next sibling in tree order"),
        Command::new(
            CommandId::parse("nav.design-previous-sibling").expect("a valid id"),
            "Select Previous Sibling",
            "Navigate",
        )
        .with_undo(Undo::None)
        .described_as("select the selected design node's previous sibling in tree order"),
        Command::new(
            CommandId::parse("nav.goto-line").expect("a valid id"),
            "Go to Line",
            "Navigate",
        )
        .with_undo(Undo::None)
        .described_as("move the caret to a line (and column) in the focused editor"),
        Command::new(
            CommandId::parse("nav.open-file").expect("a valid id"),
            "Open File",
            "Navigate",
        )
        .with_undo(Undo::None)
        .described_as("open a project file by exact path or fuzzy query"),
        Command::new(
            CommandId::parse("nav.file-symbol").expect("a valid id"),
            "Go to Symbol in File",
            "Navigate",
        )
        .with_undo(Undo::None)
        .described_as("jump to a symbol in the focused editor's document"),
        Command::new(
            CommandId::parse("nav.workspace-symbol").expect("a valid id"),
            "Go to Symbol in Workspace",
            "Navigate",
        )
        .with_undo(Undo::None)
        .described_as("search the project's symbols through the language server"),
        Command::new(
            CommandId::parse("nav.reveal-current-file").expect("a valid id"),
            "Reveal Current File",
            "Navigate",
        )
        .with_undo(Undo::None)
        .described_as("hand the focused editor's document to the system's file manager"),
        Command::new(
            CommandId::parse("nav.code-definition").expect("a valid id"),
            "Go to Definition",
            "Navigate",
        )
        .with_undo(Undo::None)
        .described_as("move to where the symbol under the caret is defined"),
        Command::new(
            CommandId::parse("nav.code-declaration").expect("a valid id"),
            "Go to Declaration",
            "Navigate",
        )
        .with_undo(Undo::None)
        .described_as("move to where the symbol under the caret is declared"),
        Command::new(
            CommandId::parse("nav.code-type-definition").expect("a valid id"),
            "Go to Type Definition",
            "Navigate",
        )
        .with_undo(Undo::None)
        .described_as("move to the definition of the symbol's type"),
        Command::new(
            CommandId::parse("nav.code-implementation").expect("a valid id"),
            "Go to Implementation",
            "Navigate",
        )
        .with_undo(Undo::None)
        .described_as("move to the implementation of the symbol under the caret"),
        Command::new(
            CommandId::parse("nav.code-references").expect("a valid id"),
            "Find References",
            "Navigate",
        )
        .with_undo(Undo::None)
        .described_as("list every reference to the symbol under the caret"),
        Command::new(
            CommandId::parse("nav.code-back").expect("a valid id"),
            "Go Back",
            "Navigate",
        )
        .with_undo(Undo::None)
        .described_as("return to the previous jump location"),
        Command::new(
            CommandId::parse("nav.code-forward").expect("a valid id"),
            "Go Forward",
            "Navigate",
        )
        .with_undo(Undo::None)
        .described_as("move to the next jump location"),
        Command::new(
            CommandId::parse("design.fit").expect("a valid id"),
            "Fit Canvas",
            "Design",
        )
        .with_undo(Undo::None)
        .described_as("scale the design surface to fit its control"),
        Command::new(
            CommandId::parse("design.actual-size").expect("a valid id"),
            "Actual Size",
            "Design",
        )
        .with_undo(Undo::None)
        .described_as("show the design surface at one screen pixel per design pixel"),
        Command::new(
            CommandId::parse("design.grid").expect("a valid id"),
            "Show Grid",
            "Design",
        )
        .with_undo(Undo::None)
        .described_as("show or hide the design grid"),
        Command::new(
            CommandId::parse("design.rulers").expect("a valid id"),
            "Show Rulers",
            "Design",
        )
        .with_undo(Undo::None)
        .described_as("show or hide the rulers along the design surface's edges"),
        Command::new(
            CommandId::parse("view.mode-welcome").expect("a valid id"),
            "Welcome Mode",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("switch the workbench to the welcome mode"),
        Command::new(
            CommandId::parse("view.mode-project").expect("a valid id"),
            "Project Mode",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("switch the workbench to the project mode"),
        Command::new(
            CommandId::parse("view.mode-design").expect("a valid id"),
            "Design Mode",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("switch the workbench to the design mode"),
        Command::new(
            CommandId::parse("view.mode-code").expect("a valid id"),
            "Code Mode",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("switch the workbench to the code mode"),
        Command::new(
            CommandId::parse("view.mode-debug").expect("a valid id"),
            "Debug Mode",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("switch the workbench to the debug mode"),
        Command::new(
            CommandId::parse("view.mode-test").expect("a valid id"),
            "Test Mode",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("switch the workbench to the test mode"),
        Command::new(
            CommandId::parse("view.mode-release").expect("a valid id"),
            "Release Mode",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("switch the workbench to the release mode"),
        Command::new(
            CommandId::parse("edit.select-all").expect("a valid id"),
            "Select All",
            "Edit",
        )
        .with_undo(Undo::None)
        .described_as("select the whole document in the focused editor"),
        Command::new(
            CommandId::parse("edit.find").expect("a valid id"),
            "Find",
            "Edit",
        )
        .with_undo(Undo::None)
        .described_as("open the find bar in the focused editor"),
        Command::new(
            CommandId::parse("edit.replace").expect("a valid id"),
            "Replace",
            "Edit",
        )
        .with_undo(Undo::None)
        .described_as("open the find bar with its replacement field"),
        Command::new(
            CommandId::parse("edit.find-next").expect("a valid id"),
            "Find Next",
            "Edit",
        )
        .with_undo(Undo::None)
        .described_as("go to the next match of the current search"),
        Command::new(
            CommandId::parse("edit.find-previous").expect("a valid id"),
            "Find Previous",
            "Edit",
        )
        .with_undo(Undo::None)
        .described_as("go to the previous match of the current search"),
        Command::new(
            CommandId::parse("edit.find-selection").expect("a valid id"),
            "Find in Selection",
            "Edit",
        )
        .with_undo(Undo::None)
        .described_as("search for the selected text in the focused editor"),
        Command::new(
            CommandId::parse("edit.select-clear").expect("a valid id"),
            "Clear Selection",
            "Edit",
        )
        .with_undo(Undo::None)
        .described_as("collapse the editor selection to the caret"),
        Command::new(
            CommandId::parse("edit.select-expand").expect("a valid id"),
            "Expand Selection",
            "Edit",
        )
        .with_undo(Undo::None)
        .described_as("widen the editor selection to the next structural unit"),
        Command::new(
            CommandId::parse("edit.select-shrink").expect("a valid id"),
            "Shrink Selection",
            "Edit",
        )
        .with_undo(Undo::None)
        .described_as("restore the selection before the last expansion"),
        Command::new(
            CommandId::parse("code.fold").expect("a valid id"),
            "Fold Block",
            "Code",
        )
        .with_undo(Undo::None)
        .described_as("fold the block at the caret"),
        Command::new(
            CommandId::parse("code.unfold").expect("a valid id"),
            "Unfold Block",
            "Code",
        )
        .with_undo(Undo::None)
        .described_as("unfold the region at the caret"),
        Command::new(
            CommandId::parse("code.fold-all").expect("a valid id"),
            "Fold All",
            "Code",
        )
        .with_undo(Undo::None)
        .described_as("fold every multi-line block in the document"),
        Command::new(
            CommandId::parse("code.unfold-all").expect("a valid id"),
            "Unfold All",
            "Code",
        )
        .with_undo(Undo::None)
        .described_as("unfold every collapsed region"),
        Command::new(
            CommandId::parse("code.outline").expect("a valid id"),
            "Show Outline",
            "Code",
        )
        .with_undo(Undo::None)
        .described_as("show the source outline panel"),
        Command::new(
            CommandId::parse("code.breadcrumbs").expect("a valid id"),
            "Show Breadcrumbs",
            "Code",
        )
        .with_undo(Undo::None)
        .described_as("name the symbols containing the caret"),
        Command::new(
            CommandId::parse("code.inlay-hints").expect("a valid id"),
            "Toggle Inlay Hints",
            "Code",
        )
        .with_undo(Undo::None)
        .described_as("show or hide inlay hints without discarding them"),
        Command::new(
            CommandId::parse("code.line-numbers").expect("a valid id"),
            "Toggle Line Numbers",
            "Code",
        )
        .with_undo(Undo::None)
        .described_as("show or hide the gutter line numbers at runtime"),
        Command::new(
            CommandId::parse("code.minimap").expect("a valid id"),
            "Toggle Minimap",
            "Code",
        )
        .with_undo(Undo::None)
        .described_as("show or hide the minimap at runtime"),
        Command::new(
            CommandId::parse("code.word-wrap").expect("a valid id"),
            "Toggle Word Wrap",
            "Code",
        )
        .with_undo(Undo::None)
        .described_as("turn soft wrapping on or off at runtime"),
        Command::new(
            CommandId::parse("code.whitespace").expect("a valid id"),
            "Toggle Whitespace",
            "Code",
        )
        .with_undo(Undo::None)
        .described_as("show or hide whitespace markers at runtime"),
        Command::new(
            CommandId::parse("code.bookmark-toggle").expect("a valid id"),
            "Toggle Bookmark",
            "Code",
        )
        .with_undo(Undo::None)
        .described_as("add or remove a bookmark on the caret line"),
        Command::new(
            CommandId::parse("edit.select-occurrences").expect("a valid id"),
            "Select All Occurrences",
            "Edit",
        )
        .with_undo(Undo::None)
        .described_as("add a cursor at every occurrence of the selection"),
        Command::new(
            CommandId::parse("edit.text-comment").expect("a valid id"),
            "Toggle Comment",
            "Edit",
        )
        .with_undo(Undo::Text)
        .described_as("toggle the line comment on the selected lines"),
        Command::new(
            CommandId::parse("edit.text-indent").expect("a valid id"),
            "Indent",
            "Edit",
        )
        .with_undo(Undo::Text)
        .described_as("indent the selected lines by one level"),
        Command::new(
            CommandId::parse("edit.text-outdent").expect("a valid id"),
            "Outdent",
            "Edit",
        )
        .with_undo(Undo::Text)
        .described_as("remove one level of indentation from the selected lines"),
        Command::new(
            CommandId::parse("edit.text-move-up").expect("a valid id"),
            "Move Lines Up",
            "Edit",
        )
        .with_undo(Undo::Text)
        .described_as("move the selected lines up"),
        Command::new(
            CommandId::parse("edit.text-move-down").expect("a valid id"),
            "Move Lines Down",
            "Edit",
        )
        .with_undo(Undo::Text)
        .described_as("move the selected lines down"),
        Command::new(
            CommandId::parse("edit.text-duplicate").expect("a valid id"),
            "Duplicate Lines",
            "Edit",
        )
        .with_undo(Undo::Text)
        .described_as("duplicate the selected lines"),
        Command::new(
            CommandId::parse("edit.text-delete").expect("a valid id"),
            "Delete Lines",
            "Edit",
        )
        .with_undo(Undo::Text)
        .described_as("delete the selected lines"),
        Command::new(
            CommandId::parse("edit.text-join").expect("a valid id"),
            "Join Lines",
            "Edit",
        )
        .with_undo(Undo::Text)
        .described_as("join the selected lines into one"),
        Command::new(
            CommandId::parse("edit.text-sort").expect("a valid id"),
            "Sort Lines",
            "Edit",
        )
        .with_undo(Undo::Text)
        .described_as("sort the selected lines in ascending order"),
        Command::new(
            CommandId::parse("edit.select-next-occurrence").expect("a valid id"),
            "Select Next Occurrence",
            "Edit",
        )
        .with_undo(Undo::None)
        .described_as("add a cursor at the next occurrence of the selection"),
        Command::new(
            CommandId::parse("edit.select-cursor-above").expect("a valid id"),
            "Add Cursor Above",
            "Edit",
        )
        .with_undo(Undo::None)
        .described_as("add a cursor one line above the primary caret"),
        Command::new(
            CommandId::parse("edit.select-cursor-below").expect("a valid id"),
            "Add Cursor Below",
            "Edit",
        )
        .with_undo(Undo::None)
        .described_as("add a cursor one line below the primary caret"),
        Command::new(
            CommandId::parse("edit.redo").expect("a valid id"),
            "Redo",
            "Edit",
        )
        .with_undo(Undo::None)
        .described_as("put back the newest change that was taken back")
        .needing_redo(),
        // The window's own layout and editor actions, reachable from the menu, the toolbar and
        // the palette as the same commands (T05.02/T04.02).
        Command::new(
            CommandId::parse("view.split").expect("a valid id"),
            "Split Editor",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("split the focused editor group and show the document in the new group"),
        Command::new(
            CommandId::parse("view.next-group").expect("a valid id"),
            "Next Editor Group",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("move the keyboard focus to the next editor group"),
        Command::new(
            CommandId::parse("view.previous-group").expect("a valid id"),
            "Previous Editor Group",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("move the keyboard focus to the previous editor group"),
        Command::new(
            CommandId::parse("view.split-down").expect("a valid id"),
            "Split Editor Down",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("split the focused editor group into stacked panes"),
        Command::new(
            CommandId::parse("view.merge-group").expect("a valid id"),
            "Merge Editor Group",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("move the focused group's views into a neighbour and close it"),
        Command::new(
            CommandId::parse("view.move-tab").expect("a valid id"),
            "Move Tab to Next Group",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("move the focused group's active tab into the next editor group"),
        Command::new(
            CommandId::parse("view.save-layout").expect("a valid id"),
            "Save Layout",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("write the panels and editor groups to the layout file"),
        Command::new(
            CommandId::parse("view.layout-load").expect("a valid id"),
            "Load Layout",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("apply the saved layout file without restarting"),
        Command::new(
            CommandId::parse("view.layout-lock").expect("a valid id"),
            "Lock Layout",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("lock or unlock the layout against save, load and reset"),
        Command::new(
            CommandId::parse("view.layout-zen").expect("a valid id"),
            "Zen Mode",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("hide every docked panel and the window chrome, and restore them on exit"),
        Command::new(
            CommandId::parse("view.layout-fullscreen").expect("a valid id"),
            "Full Screen",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("ask the platform window to enter or leave full screen"),
        Command::new(
            CommandId::parse("view.reset-layout").expect("a valid id"),
            "Reset Layout",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("restore the layout the window started with"),
        // Roadmap §3's view/panels group: one command whose argument names the panel, so the menu's
        // every panel entry binds the same id and the §2.2 ledger is the only list of panels.
        Command::new(
            CommandId::parse("view.panel-show").expect("a valid id"),
            "Show Panel",
            "View",
        )
        .with_param(Param::required(
            "panel",
            ParamType::Choice(panel_choices()),
            "the §2.2 panel to show",
        ))
        .described_as("show a panel, hiding the panel sharing its dock slot"),
        Command::new(
            CommandId::parse("view.panel-hide-current").expect("a valid id"),
            "Hide Current Panel",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("hide the panel that holds the keyboard focus"),
        Command::new(
            CommandId::parse("view.panel-restore").expect("a valid id"),
            "Restore Closed Panel",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("show the panel hidden most recently"),
        Command::new(
            CommandId::parse("view.display-toolbar").expect("a valid id"),
            "Toolbar",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("show or hide the toolbar, reflowing the editor area"),
        Command::new(
            CommandId::parse("view.display-status-bar").expect("a valid id"),
            "Status Bar",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("show or hide the status bar, reflowing the content area"),
        Command::new(
            CommandId::parse("view.display-zoom-in").expect("a valid id"),
            "Zoom In",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("enlarge the focused editor's text"),
        Command::new(
            CommandId::parse("view.display-zoom-out").expect("a valid id"),
            "Zoom Out",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("shrink the focused editor's text"),
        Command::new(
            CommandId::parse("view.display-zoom-reset").expect("a valid id"),
            "Reset Zoom",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("put the focused editor's text back to its default size"),
        Command::new(
            CommandId::parse("build.cancel").expect("a valid id"),
            "Cancel Build",
            "Build",
        )
        .with_undo(Undo::None)
        .described_as("ask the running build to stop"),
        Command::new(
            CommandId::parse("build.copy-output").expect("a valid id"),
            "Copy Build Output",
            "Build",
        )
        .with_undo(Undo::None)
        .described_as("copy the build log to the clipboard"),
        Command::new(
            CommandId::parse("preview.start").expect("a valid id"),
            "Start Preview",
            "Preview",
        )
        .with_undo(Undo::None)
        .described_as("start the preview service and report when it is ready"),
        Command::new(
            CommandId::parse("preview.stop").expect("a valid id"),
            "Stop Preview",
            "Preview",
        )
        .with_undo(Undo::None)
        .described_as("stop the preview service"),
        Command::new(
            CommandId::parse("preview.rebuild").expect("a valid id"),
            "Rebuild Preview UI",
            "Preview",
        )
        .with_undo(Undo::None)
        .described_as("mount or update the preview from the startup page's library document"),
        Command::new(
            CommandId::parse("vcs.refresh").expect("a valid id"),
            "Refresh Version Control",
            "Version Control",
        )
        .with_undo(Undo::None)
        .described_as("re-read the repository state now"),
        Command::new(
            CommandId::parse("terminal.start").expect("a valid id"),
            "Start Terminal",
            "Terminal",
        )
        .with_undo(Undo::None)
        .described_as("start a shell session in the project directory"),
        Command::new(
            CommandId::parse("terminal.stop").expect("a valid id"),
            "Stop Terminal",
            "Terminal",
        )
        .with_undo(Undo::None)
        .described_as("ask the running shell session to stop"),
        Command::new(
            CommandId::parse("test.run").expect("a valid id"),
            "Run Tests",
            "Test",
        )
        .with_undo(Undo::None)
        .described_as("discover and run the project's tests"),
        Command::new(
            CommandId::parse("test.run-failed").expect("a valid id"),
            "Rerun Failed Tests",
            "Test",
        )
        .with_undo(Undo::None)
        .described_as("run exactly the tests that failed last time"),
        Command::new(
            CommandId::parse("test.run-current-file").expect("a valid id"),
            "Run Tests in Current File",
            "Test",
        )
        .with_undo(Undo::None)
        .described_as("run the tests whose module path belongs to the focused document"),
        Command::new(
            CommandId::parse("test.run-stop").expect("a valid id"),
            "Stop Tests",
            "Test",
        )
        .with_undo(Undo::None)
        .described_as("ask the running test run to stop"),
        Command::new(
            CommandId::parse("help.shortcuts").expect("a valid id"),
            "Keyboard Shortcuts",
            "Help",
        )
        .with_undo(Undo::None)
        .described_as("open the shortcut reference in an editor tab"),
        Command::new(
            CommandId::parse("help.commands").expect("a valid id"),
            "Command Reference",
            "Help",
        )
        .with_undo(Undo::None)
        .described_as("open the command reference in an editor tab"),
        Command::new(
            CommandId::parse("help.controls").expect("a valid id"),
            "Widget Reference",
            "Help",
        )
        .with_undo(Undo::None)
        .described_as("open the widget reference in an editor tab"),
        Command::new(
            CommandId::parse("project.check").expect("a valid id"),
            "Check Project",
            "Project",
        )
        .with_undo(Undo::None)
        .described_as("report the project file's format, identity and references"),
        Command::new(
            CommandId::parse("project.trust").expect("a valid id"),
            "Trust Project",
            "Project",
        )
        .with_undo(Undo::None)
        .described_as("record that this session may write the open project"),
        Command::new(
            CommandId::parse("project.clean-resources").expect("a valid id"),
            "Clean Unused Resources",
            "Project",
        )
        .with_undo(Undo::None)
        .with_effect(EffectClass::FileWrite)
        .needing_trust()
        .described_as("list declared resources no page references and remove the declarations after confirmation"),
        Command::new(
            CommandId::parse("project.rescan").expect("a valid id"),
            "Rescan Project",
            "Project",
        )
        .with_undo(Undo::None)
        .described_as("re-read the project structure into the Project panel"),
        Command::new(
            CommandId::parse("project.migration").expect("a valid id"),
            "Migration Preview",
            "Project",
        )
        .with_undo(Undo::None)
        .described_as("report the format version and the steps writing would migrate"),
        Command::new(
            CommandId::parse("project.check-references").expect("a valid id"),
            "Check References",
            "Project",
        )
        .with_undo(Undo::None)
        .described_as("list every project reference and whether it resolves"),
        Command::new(
            CommandId::parse("debug.start").expect("a valid id"),
            "Start Debugging",
            "Debug",
        )
        .with_undo(Undo::None)
        .described_as("start a session with the selected node as its breakpoint anchor"),
        Command::new(
            CommandId::parse("debug.continue").expect("a valid id"),
            "Continue",
            "Debug",
        )
        .with_undo(Undo::None)
        .described_as("continue to the next breakpoint or the end"),
        Command::new(
            CommandId::parse("debug.step").expect("a valid id"),
            "Step Over",
            "Debug",
        )
        .with_undo(Undo::None)
        .described_as("step over one line in the session"),
        Command::new(
            CommandId::parse("debug.step-in").expect("a valid id"),
            "Step Into",
            "Debug",
        )
        .with_undo(Undo::None)
        .described_as("step into the call on the current line"),
        Command::new(
            CommandId::parse("debug.step-out").expect("a valid id"),
            "Step Out",
            "Debug",
        )
        .with_undo(Undo::None)
        .described_as("step out of the current frame"),
        Command::new(
            CommandId::parse("debug.pause").expect("a valid id"),
            "Pause",
            "Debug",
        )
        .with_undo(Undo::None)
        .described_as("pause the running debuggee"),
        Command::new(
            CommandId::parse("debug.stop").expect("a valid id"),
            "Stop Debugging",
            "Debug",
        )
        .with_undo(Undo::None)
        .described_as("end the session and terminate the adapter"),
        Command::new(
            CommandId::parse("run.start").expect("a valid id"),
            "Run Application",
            "Run",
        )
        .with_undo(Undo::None)
        .described_as("start the configured program in the project directory"),
        Command::new(
            CommandId::parse("run.stop").expect("a valid id"),
            "Stop Application",
            "Run",
        )
        .with_undo(Undo::None)
        .described_as("ask the running program to stop"),
        Command::new(
            CommandId::parse("run.clear").expect("a valid id"),
            "Clear Run Output",
            "Run",
        )
        .with_undo(Undo::None)
        .described_as("clear the application output panel"),
        Command::new(
            CommandId::parse("run.copy-output").expect("a valid id"),
            "Copy Run Output",
            "Run",
        )
        .with_undo(Undo::None)
        .described_as("copy the application output to the clipboard"),
        Command::new(
            CommandId::parse("view.welcome").expect("a valid id"),
            "Welcome",
            "View",
        )
        .with_undo(Undo::None)
        .described_as("show the welcome page"),
        Command::new(
            CommandId::parse("build.run").expect("a valid id"),
            "Run",
            "Build",
        )
        .with_effect(EffectClass::Command)
        .with_task("build")
        .blocked_while("build")
        .needing_trust()
        .described_as("build the project for its target, one at a time"),
        Command::new(
            CommandId::parse("build.check").expect("a valid id"),
            "Check",
            "Build",
        )
        .with_effect(EffectClass::Command)
        .with_task("build")
        .blocked_while("build")
        .needing_trust()
        .described_as("type-check the project without producing a binary"),
        Command::new(
            CommandId::parse("build.rebuild").expect("a valid id"),
            "Rebuild",
            "Build",
        )
        .with_effect(EffectClass::Command)
        .with_task("build")
        .blocked_while("build")
        .needing_trust()
        .described_as("remove this package's artifacts, then build again"),
        Command::new(
            CommandId::parse("build.clean").expect("a valid id"),
            "Clean",
            "Build",
        )
        .with_effect(EffectClass::Command)
        .with_task("build")
        .blocked_while("build")
        .needing_trust()
        .described_as("remove this package's artifacts without building"),
        Command::new(
            CommandId::parse("build.copy-command").expect("a valid id"),
            "Copy Build Command",
            "Build",
        )
        .with_undo(Undo::None)
        .described_as("copy the cargo command the last build actually ran"),
        Command::new(
            CommandId::parse("build.locate-first-error").expect("a valid id"),
            "Go to First Error",
            "Build",
        )
        .with_undo(Undo::None)
        .described_as("open the first source location the build output names"),
        // The commands the default keymap binds and the menu names as planned. Registering them is
        // the same claim every command here makes — that it is described once — and the audit
        // (T05.08) checks that the menu tree and the keymap only ever name ids registered here.
        Command::new(
            CommandId::parse("view.palette").expect("a valid id"),
            "Command Palette",
            "View",
        )
        .described_as("find a command from the keyboard"),
        Command::new(
            CommandId::parse("keys.show").expect("a valid id"),
            "Keyboard Shortcuts",
            "View",
        )
        .described_as("show the keyboard shortcuts"),
    ];
    for command in commands {
        registry.register(command).expect("ids are unique");
    }
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        built_in()
    }

    #[test]
    fn an_id_is_a_namespace_and_an_action_and_nothing_else() {
        let id = CommandId::parse("file.save").expect("a valid id");
        assert_eq!(id.namespace(), "file");
        assert_eq!(id.action(), "save");
        assert_eq!(id.to_string(), "file.save");
        for bad in [
            "file",
            "file.",
            ".save",
            "file.save.now",
            "File.Save",
            "file..save",
        ] {
            assert!(CommandId::parse(bad).is_none(), "{bad} parsed");
        }
    }

    #[test]
    fn a_duplicate_id_is_refused_rather_than_replacing_the_first() {
        let mut registry = Registry::new();
        let first = Command::new(CommandId::parse("file.save").unwrap(), "Save", "File");
        registry.register(first).expect("the first registers");
        let second = Command::new(
            CommandId::parse("file.save").unwrap(),
            "Save Elsewhere",
            "File",
        );
        let refusal = registry
            .register(second)
            .expect_err("the second is refused");
        assert_eq!(refusal.id().to_string(), "file.save");
        assert_eq!(registry.len(), 1);
        assert_eq!(
            registry
                .command(&CommandId::parse("file.save").unwrap())
                .unwrap()
                .title(),
            "Save"
        );
    }

    #[test]
    fn a_command_runs_only_when_its_effects_are_authorized() {
        let registry = registry();
        let call = Call::to("build.run");
        let mut context = Context::in_area("design").trusted();

        // Trusted but not authorized: the effect is the missing thing, and it is named.
        let refusal = registry
            .resolve(&call, &context)
            .expect_err("not authorized");
        assert_eq!(
            refusal,
            Refusal::Unauthorized {
                id: CommandId::parse("build.run").unwrap(),
                effect: EffectClass::Command,
            }
        );
        assert!(refusal.message().contains("not authorized"));

        context = context.authorizing(EffectClass::Command);
        assert!(registry.resolve(&call, &context).is_ok());
    }

    #[test]
    fn a_history_command_refuses_only_when_the_editor_says_there_is_nothing_to_do() {
        let registry = built_in();
        let undo = Call::to("edit.undo");
        let redo = Call::to("edit.redo");

        // The caller did not say: the command stays available rather than being greyed out on a
        // guess, and the handler still answers honestly when it runs.
        let unknown = Context::in_area("design");
        assert!(registry.resolve(&undo, &unknown).is_ok());
        assert!(registry.resolve(&redo, &unknown).is_ok());

        // The editor said "nothing to undo" / "nothing to redo": refused, with the reason.
        let empty = Context::in_area("design").with_history(Some(false), Some(false));
        for (call, needle) in [(&undo, "nothing to undo"), (&redo, "nothing to redo")] {
            let refusal = registry.resolve(call, &empty).expect_err("refused");
            assert!(matches!(refusal, Refusal::Disabled { .. }), "{refusal:?}");
            assert!(refusal.message().contains(needle), "{}", refusal.message());
        }

        // The halves are independent: undo available, redo not.
        let mixed = Context::in_area("design").with_history(Some(true), Some(false));
        assert!(registry.resolve(&undo, &mixed).is_ok());
        assert!(registry.resolve(&redo, &mixed).is_err());
    }

    #[test]
    fn an_untrusted_project_cannot_run_a_command_that_needs_trust() {
        let registry = registry();
        let context = Context::in_area("design").authorizing(EffectClass::Command);
        let refusal = registry
            .resolve(&Call::to("build.run"), &context)
            .expect_err("untrusted");
        assert!(matches!(refusal, Refusal::Disabled { .. }));
        assert!(refusal.message().contains("not trusted"));
    }

    #[test]
    fn a_bad_or_missing_argument_is_refused_before_anything_runs() {
        let registry = registry();
        let context = Context::in_area("design").trusted();
        let refusal = registry
            .resolve(&Call::to("file.close").with("decision", "maybe"), &context)
            .expect_err("not a choice");
        assert!(
            matches!(refusal, Refusal::BadArgument { ref name, .. } if name == "decision"),
            "{refusal:?}"
        );
        assert!(refusal.message().contains("not one of"));

        let refusal = registry
            .resolve(&Call::to("file.close").with("nonsense", "yes"), &context)
            .expect_err("no such parameter");
        assert!(refusal.message().contains("no such parameter"));

        // A missing required parameter, on a command that has one.
        let mut registry = Registry::new();
        registry
            .register(
                Command::new(CommandId::parse("go.to").unwrap(), "Go", "Go")
                    .with_param(Param::required("node", ParamType::NodeId, "the node")),
            )
            .expect("registers");
        let refusal = registry
            .resolve(&Call::to("go.to"), &Context::in_area("design"))
            .expect_err("missing");
        assert_eq!(
            refusal,
            Refusal::Missing {
                id: CommandId::parse("go.to").unwrap(),
                name: "node".to_owned()
            }
        );
    }

    #[test]
    fn a_command_that_is_not_here_and_now_is_disabled_rather_than_unknown() {
        let registry = registry();
        let call = Call::to("view.welcome");
        // Focus is "any", so it is available everywhere and known.
        assert!(registry.resolve(&call, &Context::in_area("debug")).is_ok());

        let mut registry = Registry::new();
        registry
            .register(
                Command::new(CommandId::parse("panel.toggle").unwrap(), "Toggle", "View")
                    .in_focus("design"),
            )
            .expect("registers");
        let refusal = registry
            .resolve(&Call::to("panel.toggle"), &Context::in_area("debug"))
            .expect_err("wrong area");
        assert!(matches!(refusal, Refusal::Disabled { .. }));
        assert!(refusal.message().contains("only available in design"));

        let unknown = registry.resolve(&Call::to("nothing.here"), &Context::in_area("design"));
        assert!(matches!(unknown, Err(Refusal::Unknown { .. })));
    }

    #[test]
    fn a_task_already_running_disables_a_command_that_would_run_it_again() {
        let registry = registry();
        let context = Context::in_area("design")
            .trusted()
            .authorizing(EffectClass::Command);
        assert!(registry.resolve(&Call::to("build.run"), &context).is_ok());
        let busy = context.running("build");
        let refusal = registry
            .resolve(&Call::to("build.run"), &busy)
            .expect_err("busy");
        assert!(refusal.message().contains("build is already running"));
    }

    #[test]
    fn an_invocation_carries_its_metadata_and_its_parsed_arguments() {
        let registry = registry();
        let context = Context::in_area("design").trusted();
        let invocation = registry
            .resolve(
                &Call::to("file.close").with("decision", "save-all"),
                &context,
            )
            .expect("resolves");
        assert_eq!(invocation.id().to_string(), "file.close");
        assert_eq!(invocation.title(), "Close");
        assert_eq!(
            invocation.argument("decision"),
            Some(&Value::Text("save-all".to_owned()))
        );
        assert_eq!(invocation.undo(), Undo::None);
        assert!(invocation.describe().contains("decision=save-all"));
    }

    #[test]
    fn why_not_answers_with_the_refusal_or_nothing() {
        let registry = registry();
        let context = Context::in_area("welcome");
        assert!(
            registry
                .why_not(&Call::to("file.close"), &context)
                .is_none()
        );
        assert!(
            registry
                .why_not(&Call::to("build.run"), &context)
                .unwrap()
                .contains("not trusted")
        );
        assert!(
            registry
                .why_not(&Call::to("nope.nope"), &context)
                .unwrap()
                .contains("no command")
        );
    }

    #[test]
    fn the_built_in_commands_all_have_parsable_ids_and_non_empty_titles() {
        let registry = registry();
        assert!(!registry.is_empty());
        for command in registry.commands() {
            assert!(CommandId::parse(&command.id().to_string()).is_some());
            assert!(!command.title().is_empty());
            assert!(!command.category().is_empty());
        }
    }
}
