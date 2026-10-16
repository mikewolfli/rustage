// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Data and events' commands: schemas, bindings, events, dispatch, routes, state machines,
//! adapters and the joint acceptance log.

// Every branch of the flag table answers with `return`, including the ones that could be a
// trailing expression: the branches are independent flags, and writing them alike is worth more
// than saving a keyword. The returns that are *not* needless — the ones inside a loop that refuses
// a bad value — are why the table reads the way it does.
#![allow(clippy::needless_return)]

use super::*;

/// How to declare a data schema and the sample/runtime/error datasets (task T13.01).
#[derive(Debug, PartialEq, Eq)]
pub struct DataOptions {
    /// The operations: `schema:<name>:<version>`, `field:<name>:<kind>[:required]`,
    /// `sample:<field>=<value>`, `runtime:<field>=<value>`, `error:<reason>`, `check`, `active`,
    /// `state`, `show`.
    pub ops: Vec<String>,
}

impl DataOptions {
    /// Options with an empty schema.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to declare typed bindings between data and node properties (task T13.02).
#[derive(Debug, PartialEq, Eq)]
pub struct BindingOptions {
    /// The operations: `schema:<name>:<version>`, `field:<name>:<kind>`, `slot:<node>.<property>:<type>`,
    /// `bind:<source>=<target>:<direction>:<conversion>`, `check`, `state`, `show`; a source/target
    /// is `f:<field>` or `<node>.<property>`.
    pub ops: Vec<String>,
}

impl BindingOptions {
    /// Options with an empty graph.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to declare events, wire handlers and fire triggers (task T13.03).
#[derive(Debug, PartialEq, Eq)]
pub struct EventsOptions {
    /// The operations: `event:<name>`, `field:<event>:<field>:<kind>[:required]`,
    /// `attach:<node>:<event>:<handler>`, `status:<node>:<event>`,
    /// `fire:<node>:<event>:<field>=<value>[,...]`, `check`, `state`, `show`.
    pub ops: Vec<String>,
}

impl EventsOptions {
    /// Options with no declarations.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to bind handler actions, check permissions and release subscriptions (task T13.04).
#[derive(Debug, PartialEq, Eq)]
pub struct DispatchOptions {
    /// The operations: `slot:<node>.<property>:<type>`, `command:<id>`, `function:<name>`, `trust`,
    /// `authorize:<capability>`, `bind:<node>:<event>:set:<property>=<value>`,
    /// `bind:<node>:<event>:command:<id>`, `bind:<node>:<event>:call:<function>:<capability>`,
    /// `unbind:<node>:<event>`, `dispatch:<node>:<event>`, `check`, `state`, `show`.
    pub ops: Vec<String>,
}

impl DispatchOptions {
    /// Options with nothing bound.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to declare pages, navigate and drive a window's lifecycle (task T13.05).
#[derive(Debug, PartialEq, Eq)]
pub struct RoutesOptions {
    /// The operations: `page:<name>`, `entry:<name>`, `edge:<from>:<to>`, `open:<page>`, `ready`,
    /// `go:<page>`, `back`, `forward`, `close`, `finish-close`, `check`, `state`, `show`.
    pub ops: Vec<String>,
}

impl RoutesOptions {
    /// Options with no pages declared.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to declare a state machine and send events to it (task T13.06).
#[derive(Debug, PartialEq, Eq)]
pub struct MachineOptions {
    /// The operations: `state:<name>`, `initial:<name>`, `final:<name>`,
    /// `transition:<from>:<on>:<to>[:<guard>]`, `answer:<guard>:<true|false>:<sync|async>`, `start`,
    /// `send:<event>[:<sync|async>]`, `inspect`, `show`.
    pub ops: Vec<String>,
}

impl MachineOptions {
    /// Options with no states declared.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive a file/network adapter with a sample double and a retry policy (task T13.07).
#[derive(Debug, PartialEq, Eq)]
pub struct AdapterOptions {
    /// The operations: `sample:<outcome>`, `live:<target>:<outcome>`, `request:<kind>:<target>`,
    /// `cancel`, `online`, `offline`, `attempt:<n>`, `run`, `check`, `state`, `show`; an outcome is
    /// `ok:<value>`/`failed:<reason>`/`unknown:<reason>`/`cancelled`.
    pub ops: Vec<String>,
}

impl AdapterOptions {
    /// Options with no adapter.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to drive the joint acceptance: the undo log, vectors and the two-way loop (task T13.08).
#[derive(Debug, PartialEq, Eq)]
pub struct SessionLogOptions {
    /// The operations: `human:<label>:<before>:<after>`, `business:<label>:<before>:<after>`,
    /// `undo`, `loop:<identity|drifting>:<value>`, `state:<name>`, `initial:<name>`, `final:<name>`,
    /// `transition:<from>:<on>:<to>`, `expect:<event>:<state>`, `vector`, `check`, `show`.
    pub ops: Vec<String>,
}

impl SessionLogOptions {
    /// Options with an empty log.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_data(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--data" => {
            let mut options = DataOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Data(options)));
        }
        "--binding" => {
            let mut options = BindingOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Binding(options)));
        }
        "--events" => {
            let mut options = EventsOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Events(options)));
        }
        "--dispatch" => {
            let mut options = DispatchOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Dispatch(options)));
        }
        "--routes" => {
            let mut options = RoutesOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Routes(options)));
        }
        "--machine" => {
            let mut options = MachineOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Machine(options)));
        }
        "--adapter" => {
            let mut options = AdapterOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Adapter(options)));
        }
        "--session-log" => {
            let mut options = SessionLogOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::SessionLog(options)));
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn parses_the_data_operations() {
        assert_eq!(parse(["--data"]), Ok(Command::Data(DataOptions::new())),);
        let mut expected = DataOptions::new();
        expected.ops = vec![
            "schema:user:3".to_owned(),
            "field:name:text:required".to_owned(),
            "active".to_owned(),
        ];
        assert_eq!(
            parse([
                "--data",
                "schema:user:3",
                "field:name:text:required",
                "active"
            ]),
            Ok(Command::Data(expected)),
        );
        assert!(parse(["--data", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_binding_operations() {
        assert_eq!(
            parse(["--binding"]),
            Ok(Command::Binding(BindingOptions::new())),
        );
        let mut expected = BindingOptions::new();
        expected.ops = vec![
            "field:count:integer".to_owned(),
            "slot:1.width:number".to_owned(),
            "bind:f:count=1.width:one-way:identity".to_owned(),
            "check".to_owned(),
        ];
        assert_eq!(
            parse([
                "--binding",
                "field:count:integer",
                "slot:1.width:number",
                "bind:f:count=1.width:one-way:identity",
                "check"
            ]),
            Ok(Command::Binding(expected)),
        );
        assert!(parse(["--binding", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_events_operations() {
        assert_eq!(
            parse(["--events"]),
            Ok(Command::Events(EventsOptions::new())),
        );
        let mut expected = EventsOptions::new();
        expected.ops = vec![
            "event:pressed".to_owned(),
            "field:pressed:count:integer:required".to_owned(),
            "attach:1:pressed:on_press".to_owned(),
        ];
        assert_eq!(
            parse([
                "--events",
                "event:pressed",
                "field:pressed:count:integer:required",
                "attach:1:pressed:on_press"
            ]),
            Ok(Command::Events(expected)),
        );
        assert!(parse(["--events", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_dispatch_operations() {
        assert_eq!(
            parse(["--dispatch"]),
            Ok(Command::Dispatch(DispatchOptions::new())),
        );
        let mut expected = DispatchOptions::new();
        expected.ops = vec![
            "slot:1.label:text:100".to_owned(),
            "command:save".to_owned(),
            "bind:1:pressed:command:save".to_owned(),
        ];
        assert_eq!(
            parse([
                "--dispatch",
                "slot:1.label:text:100",
                "command:save",
                "bind:1:pressed:command:save"
            ]),
            Ok(Command::Dispatch(expected)),
        );
        assert!(parse(["--dispatch", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_routes_operations() {
        assert_eq!(
            parse(["--routes"]),
            Ok(Command::Routes(RoutesOptions::new())),
        );
        let mut expected = RoutesOptions::new();
        expected.ops = vec![
            "page:home".to_owned(),
            "entry:home".to_owned(),
            "edge:home:detail".to_owned(),
        ];
        assert_eq!(
            parse(["--routes", "page:home", "entry:home", "edge:home:detail"]),
            Ok(Command::Routes(expected)),
        );
        assert!(parse(["--routes", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_machine_operations() {
        assert_eq!(
            parse(["--machine"]),
            Ok(Command::Machine(MachineOptions::new())),
        );
        let mut expected = MachineOptions::new();
        expected.ops = vec![
            "state:idle".to_owned(),
            "transition:idle:go:done".to_owned(),
            "send:go".to_owned(),
        ];
        assert_eq!(
            parse([
                "--machine",
                "state:idle",
                "transition:idle:go:done",
                "send:go"
            ]),
            Ok(Command::Machine(expected)),
        );
        assert!(parse(["--machine", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_adapter_operations() {
        assert_eq!(
            parse(["--adapter"]),
            Ok(Command::Adapter(AdapterOptions::new())),
        );
        let mut expected = AdapterOptions::new();
        expected.ops = vec![
            "sample:ok:bytes".to_owned(),
            "request:file-read:user.png".to_owned(),
            "run".to_owned(),
        ];
        assert_eq!(
            parse([
                "--adapter",
                "sample:ok:bytes",
                "request:file-read:user.png",
                "run"
            ]),
            Ok(Command::Adapter(expected)),
        );
        assert!(parse(["--adapter", "--unknown"]).is_err());
    }

    #[test]
    fn parses_the_session_log_operations() {
        assert_eq!(
            parse(["--session-log"]),
            Ok(Command::SessionLog(SessionLogOptions::new())),
        );
        let mut expected = SessionLogOptions::new();
        expected.ops = vec![
            "human:set width:8:16".to_owned(),
            "business:sent the order:none:order-7".to_owned(),
            "undo".to_owned(),
        ];
        assert_eq!(
            parse([
                "--session-log",
                "human:set width:8:16",
                "business:sent the order:none:order-7",
                "undo"
            ]),
            Ok(Command::SessionLog(expected)),
        );
        assert!(parse(["--session-log", "--unknown"]).is_err());
    }
}
