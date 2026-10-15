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
    Disabled {
        /// The command.
        id: CommandId,
        /// Why.
        reason: String,
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

impl Refusal {
    /// The refusal in one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::Unknown { id } => format!("there is no command {id}"),
            Self::Disabled { id, reason } => format!("{id} is not available: {reason}"),
            Self::Missing { id, name } => format!("{id} needs a {name} argument"),
            Self::BadArgument { id, name, reason } => {
                format!("{id}: {name} is not valid: {reason}")
            }
            Self::Unauthorized { id, effect } => {
                format!(
                    "{id} has the {} effect, which is not authorized",
                    effect.name()
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
        match CommandId::parse(id) {
            Some(parsed) if self.commands.contains_key(&parsed) => None,
            Some(_) => Some(format!("there is no command {id}")),
            None => Some(format!("{id:?} is not a command id")),
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
                reason: format!(
                    "it is only available in {area}, and the keyboard is in {}",
                    context.focus()
                ),
            });
        }
        if command.needs_trust && !context.is_trusted() {
            return Err(Refusal::Disabled {
                id: parsed.clone(),
                reason: "the project is not trusted".to_owned(),
            });
        }
        if let Some(task) = &command.blocks_on
            && context.running.contains(task)
        {
            return Err(Refusal::Disabled {
                id: parsed.clone(),
                reason: format!("{task} is already running"),
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

/// The commands this build registers: the ones the roadmap's §3 menu tree names and that have a
/// handler today. Registering a command here is a claim that it exists, so the set is deliberately
/// small — the id convention (`P-CMD-ID`) fixes 16 namespaces, but a namespace is not a command.
pub fn built_in() -> Registry {
    let mut registry = Registry::new();
    let commands = [
        Command::new(
            CommandId::parse("file.save").expect("a valid id"),
            "Save",
            "File",
        )
        .with_undo(Undo::None)
        .described_as("write the current document back to disk"),
        Command::new(
            CommandId::parse("file.save-all").expect("a valid id"),
            "Save All",
            "File",
        )
        .with_undo(Undo::None)
        .described_as("write every document with unsaved changes"),
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
            CommandId::parse("edit.undo").expect("a valid id"),
            "Undo",
            "Edit",
        )
        .with_undo(Undo::None)
        .described_as("take back the newest change, in its own domain"),
        Command::new(
            CommandId::parse("edit.redo").expect("a valid id"),
            "Redo",
            "Edit",
        )
        .with_undo(Undo::None)
        .described_as("put back the newest change that was taken back"),
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
        // The commands the default keymap binds and the menu names as planned. Registering them is
        // the same claim every command here makes — that it is described once — and the audit
        // (T05.08) checks that the menu tree and the keymap only ever name ids registered here.
        Command::new(
            CommandId::parse("file.save-as").expect("a valid id"),
            "Save As",
            "File",
        )
        .with_undo(Undo::None)
        .described_as("write the document to another file"),
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
        Command::new(
            CommandId::parse("panel.close").expect("a valid id"),
            "Close Panel",
            "View",
        )
        .described_as("hide the panel with the keyboard"),
        Command::new(
            CommandId::parse("tree.activate").expect("a valid id"),
            "Activate",
            "View",
        )
        .described_as("open the selected tree item"),
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
        assert!(registry.why_not(&Call::to("file.save"), &context).is_none());
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
