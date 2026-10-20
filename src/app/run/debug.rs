// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F19: adapters, sessions, breakpoints, execution, inspection, navigation and the hot-update
//! boundary.
//!
//! This is the real call chain the models in [`crate::app::debug`] describe. A session is started by
//! [`Client::spawn`] against the adapter the probe found, then initialized with
//! `initialize`/`launch`/`setBreakpoints`/`configurationDone`. Every answer is parsed from the
//! adapter's own body: a breakpoint is bound only when `verified` is true, a timeout is a timeout,
//! and a refusal is printed with the adapter's message rather than hidden.
//!
//! Nothing here fabricates a session. `probe` reports what is installed, `launch` without an adapter
//! is a refusal, and `attach` uses `terminateDebuggee: false` because the process is not this
//! session's. When a command fails, [`Machine::fail`] disconnects (with the session's own flag) and
//! terminates the adapter, so a failure does not leave a process behind.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use super::*;
use crate::app::debug::adapter::{self, Adapter};
use crate::app::debug::breakpoints::Book;
use crate::app::debug::hotswap;
use crate::app::debug::inspect::{
    self, EvalDecision, EvalPolicy, Evaluation, Frame, PAGE_LIMIT, STACK_LIMIT, Scope, Thread,
    Variable, Watches,
};
use crate::app::debug::protocol::{Client, ProtocolError, Response};
use crate::app::debug::recovery::{self, Incident};
use crate::app::debug::session::{Action, Config, Phase, Session, StopReason};
use crate::app::targets::configuration::Os;
use crate::app::targets::local::Probe;
use crate::project::record::json::{Json, JsonObject};

/// How long to wait when a script does not say.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// The state a debug script drives.
struct Machine {
    host: Os,
    probe: Probe,
    adapter: Option<Adapter>,
    session: Option<Session>,
    client: Option<Client>,
    book: Book,
    watches: Watches,
    policy: EvalPolicy,
    timeout: Duration,
    authorized: bool,
    last_thread: Option<i64>,
    last_frame: Option<i64>,
    wanted_source_version: u64,
    stop_on_entry: bool,
    /// How many variables each reference is known to hold, from `scopes` or a parent `variables`
    /// answer. Used to bound a page before it is asked for.
    reference_counts: BTreeMap<i64, usize>,
    failed: bool,
}

impl Machine {
    fn new() -> Self {
        Self {
            host: Os::host(),
            probe: Probe::from_path(),
            adapter: None,
            session: None,
            client: None,
            book: Book::new(),
            watches: Watches::new(),
            policy: EvalPolicy::Allow,
            timeout: DEFAULT_TIMEOUT,
            authorized: false,
            last_thread: None,
            last_frame: None,
            wanted_source_version: 0,
            stop_on_entry: false,
            reference_counts: BTreeMap::new(),
            failed: false,
        }
    }

    fn select_adapter(&mut self, name: &str) -> Result<(), String> {
        let adapter = if let Some(program) = name.strip_prefix("path:") {
            if program.is_empty() {
                return Err("adapter:path: needs a program".to_owned());
            }
            adapter::explicit(std::path::Path::new(program))
        } else if name == "auto" {
            adapter::first_ready(self.host, &self.probe).map_err(|refusal| refusal.render())?
        } else {
            adapter::select(self.host, &self.probe, name).map_err(|refusal| refusal.render())?
        };
        self.adapter = Some(adapter);
        Ok(())
    }

    fn ensure_adapter(&self) -> Result<Adapter, String> {
        self.adapter
            .clone()
            .ok_or_else(|| "no debug adapter was selected; use adapter:<name|auto>".to_owned())
    }

    fn session(&self) -> Result<&Session, String> {
        self.session
            .as_ref()
            .ok_or_else(|| "no debug session has started".to_owned())
    }

    fn prepare_session(&mut self, attached: bool) -> Result<Session, String> {
        let adapter = self.ensure_adapter()?;
        let mut session = Session::new(&adapter);
        session.authorized = self.authorized;
        session.attached = attached;
        Ok(session)
    }

    fn start_client(&mut self) -> Result<&mut Client, String> {
        if self.client.is_some() {
            return Err("a debug adapter is already running".to_owned());
        }
        let adapter = self.ensure_adapter()?;
        let program = adapter
            .program
            .as_ref()
            .ok_or_else(|| format!("{} has no program on this machine", adapter.name))?;
        let client = Client::spawn(program, &adapter.arguments, None, &[], self.authorized)
            .map_err(|error| error.render())?;
        self.client = Some(client);
        Ok(self.client.as_mut().expect("a client"))
    }

    fn initialize(&mut self) -> Result<(), String> {
        let mut arguments = JsonObject::new();
        arguments.insert("clientID", Json::string("rustage"));
        arguments.insert("clientName", Json::string("rustage"));
        arguments.insert("adapterID", Json::string("rustage"));
        arguments.insert("pathFormat", Json::string("path"));
        arguments.insert("linesStartAt1", Json::bool(true));
        arguments.insert("columnsStartAt1", Json::bool(true));
        arguments.insert("supportsVariableType", Json::bool(true));
        arguments.insert("supportsVariablePaging", Json::bool(true));
        arguments.insert("supportsRunInTerminalRequest", Json::bool(false));
        arguments.insert("supportsStartDebuggingRequest", Json::bool(false));
        self.request("initialize", Json::Object(arguments))?;
        Ok(())
    }

    fn request(&mut self, command: &str, arguments: Json) -> Result<Response, String> {
        let seq = self.send_only(command, arguments)?;
        self.await_request(seq, command)
    }

    fn send_only(&mut self, command: &str, arguments: Json) -> Result<i64, String> {
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| format!("{command} needs a running debug adapter"))?;
        client
            .send_request(command, &arguments)
            .map_err(|error| error.render())
    }

    fn await_request(&mut self, seq: i64, command: &str) -> Result<Response, String> {
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| format!("{command} needs a running debug adapter"))?;
        let response = client
            .await_response(seq, command, self.timeout)
            .map_err(|error| error.render())?;
        if !response.success {
            return Err(format!(
                "{command} was refused by the adapter: {}",
                response
                    .message
                    .clone()
                    .unwrap_or_else(|| "no message".to_owned())
            ));
        }
        Ok(response)
    }

    fn launch(
        &mut self,
        program: &str,
        arguments: Vec<String>,
        directory: Option<PathBuf>,
    ) -> Result<(), String> {
        let mut config = Config::launch(program);
        config.arguments = arguments;
        config.directory = directory;
        config.source_version = self.wanted_source_version;
        config.stop_on_entry = self.stop_on_entry;
        let mut session = if let Some(session) = self.session.clone() {
            session
        } else {
            self.prepare_session(false)?
        };
        session
            .configure(config.clone())
            .map_err(|refusal| refusal.render())?;
        session.launching().map_err(|refusal| refusal.render())?;
        self.session = Some(session);
        self.start_client()?;
        self.initialize()?;

        let mut launch = JsonObject::new();
        launch.insert("program", Json::string(program));
        launch.insert(
            "args",
            Json::Array(
                config
                    .arguments
                    .iter()
                    .map(|argument| Json::string(argument.clone()))
                    .collect(),
            ),
        );
        if let Some(directory) = &config.directory {
            launch.insert("cwd", Json::string(directory.display().to_string()));
        }
        let current = std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        if config.directory.is_none() && !current.is_empty() {
            launch.insert("cwd", Json::string(current));
        }
        let stages = self.ensure_adapter()?.stages_breakpoints_at_entry;
        if stages {
            // GDB's DAP `launch` runs the program inside the request and its `configurationDone` is
            // a no-op (`gdb/dap/launch.py`). Asking it to stop at the main subprogram first is the
            // only point at which breakpoints can be installed before the program passes them.
            launch.insert("stopAtBeginningOfMainSubprogram", Json::bool(true));
        } else {
            launch.insert("stopOnEntry", Json::bool(config.stop_on_entry));
        }
        launch.insert("console", Json::string("internalConsole"));
        launch.insert("sourceLanguages", Json::array([Json::string("rust")]));
        // `lldb-dap` sends `initialized` while it handles `launch` and answers the request only after
        // `configurationDone`. Sending the request and awaiting it later is what keeps the protocol
        // order both sides expect; waiting for the response here would deadlock against the event.
        let launch_seq = match self.send_only("launch", Json::Object(launch)) {
            Ok(seq) => seq,
            Err(reason) => {
                self.cleanup(false);
                return Err(reason);
            }
        };
        if let Err(reason) = self.wait_for_initialized() {
            self.cleanup(false);
            return Err(reason);
        }
        self.apply_breakpoints_all()?;
        self.request("configurationDone", Json::Null)?;
        self.await_request(launch_seq, "launch")?;
        if stages {
            // The program is stopped at main rather than running: that is how this adapter reports
            // a successful launch whose breakpoints were applied. Saying `resumed` here would be a
            // state the adapter is not in.
            if let Some(session) = self.session.as_mut() {
                session.stopped(StopReason::Entry, None);
            }
            println!("launched {program} (staged at main to apply breakpoints; continue to run)");
        } else {
            if let Some(session) = self.session.as_mut() {
                session.resumed();
            }
            println!("launched {program}");
        }
        Ok(())
    }

    fn attach(&mut self, pid: u32) -> Result<(), String> {
        let mut session = self.prepare_session(true)?;
        let mut config = Config::attach(pid);
        config.source_version = self.wanted_source_version;
        session
            .configure(config)
            .map_err(|refusal| refusal.render())?;
        session.attaching().map_err(|refusal| refusal.render())?;
        self.session = Some(session);
        self.start_client()?;
        self.initialize()?;
        let mut arguments = JsonObject::new();
        arguments.insert("pid", Json::number(u64::from(pid)));
        let attach_seq = match self.send_only("attach", Json::Object(arguments)) {
            Ok(seq) => seq,
            Err(reason) => {
                self.cleanup(false);
                return Err(reason);
            }
        };
        if let Err(reason) = self.wait_for_initialized() {
            self.cleanup(false);
            return Err(reason);
        }
        self.apply_breakpoints_all()?;
        self.request("configurationDone", Json::Null)?;
        self.await_request(attach_seq, "attach")?;
        if let Some(session) = self.session.as_mut() {
            session.resumed();
        }
        println!("attached to {pid} (the process is not terminated on disconnect)");
        Ok(())
    }

    fn wait_for_initialized(&mut self) -> Result<(), String> {
        let deadline = std::time::Instant::now() + self.timeout;
        loop {
            let remaining = deadline
                .checked_duration_since(std::time::Instant::now())
                .unwrap_or(Duration::ZERO);
            if remaining.is_zero() {
                return Err("the adapter did not send initialized before the deadline".to_owned());
            }
            let client = self
                .client
                .as_mut()
                .ok_or_else(|| "the adapter is not running".to_owned())?;
            match client.event(remaining) {
                Ok(event) => {
                    if event.event == "initialized" {
                        return Ok(());
                    }
                    if event.event == "output" {
                        print_output(&event);
                    }
                }
                Err(ProtocolError::Timeout { .. }) => {
                    return Err(
                        "the adapter did not send initialized before the deadline".to_owned()
                    );
                }
                Err(error) => return Err(error.render()),
            }
        }
    }

    fn apply_breakpoints_all(&mut self) -> Result<(), String> {
        for file in self.book.files() {
            self.apply_breakpoints(&file)?;
        }
        Ok(())
    }

    fn apply_breakpoints(&mut self, file: &str) -> Result<(), String> {
        let body = self.book.set_breakpoints_body(file);
        let response = self.request("setBreakpoints", body)?;
        let entries = response
            .object()
            .and_then(|body| body.get("breakpoints"))
            .and_then(Json::as_array)
            .map(|entries| entries.to_vec())
            .unwrap_or_default();
        let ids: Vec<u32> = self
            .book
            .points()
            .iter()
            .filter(|point| point.enabled)
            .filter(|point| {
                matches!(&point.anchor, crate::app::debug::breakpoints::Anchor::Source { file: point_file, .. } if point_file == file)
            })
            .map(|point| point.id)
            .collect();
        for (id, entry) in ids.iter().zip(entries.iter()) {
            let verified = matches!(
                entry.as_object().and_then(|object| object.get("verified")),
                Some(Json::Bool(true))
            );
            let line = entry
                .as_object()
                .and_then(|object| object.get("line"))
                .and_then(Json::as_u64)
                .map(|line| line as u32);
            let message = entry
                .as_object()
                .and_then(|object| object.get("message"))
                .and_then(Json::as_str);
            if let Some(dap_id) = entry
                .as_object()
                .and_then(|object| object.get("id"))
                .and_then(Json::as_i64)
            {
                self.book.remember_dap_id(*id, dap_id);
            }
            self.book.record(*id, verified, line, message);
        }
        Ok(())
    }

    fn wait(&mut self, timeout: Duration) -> Result<(), String> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let remaining = deadline
                .checked_duration_since(std::time::Instant::now())
                .unwrap_or(Duration::ZERO);
            if remaining.is_zero() {
                println!("no debug event within {} ms", timeout.as_millis());
                return Ok(());
            }
            let client = self
                .client
                .as_mut()
                .ok_or_else(|| "the adapter is not running".to_owned())?;
            match client.event(remaining) {
                Ok(event) => match event.event.as_str() {
                    "stopped" => {
                        let object = event.object();
                        let reason_text = object
                            .and_then(|body| body.get("reason"))
                            .and_then(Json::as_str)
                            .unwrap_or("stopped");
                        let description = object
                            .and_then(|body| body.get("description"))
                            .and_then(Json::as_str);
                        let thread = object
                            .and_then(|body| body.get("threadId"))
                            .and_then(Json::as_i64);
                        let line = object
                            .and_then(|body| body.get("line"))
                            .and_then(Json::as_u64)
                            .map(|line| line as u32)
                            .unwrap_or(0);
                        let line = if line == 0 {
                            thread
                                .and_then(|thread| self.top_location(thread))
                                .unwrap_or(0)
                        } else {
                            line
                        };
                        let reason = StopReason::parse(reason_text, description, line);
                        self.last_thread = thread;
                        if let Some(session) = self.session.as_mut() {
                            session.stopped(reason.clone(), thread);
                        }
                        println!(
                            "stopped: {} (thread {}, source version {})",
                            reason.render(),
                            thread
                                .map(|thread| thread.to_string())
                                .unwrap_or_else(|| "?".to_owned()),
                            self.source_version()
                        );
                        return Ok(());
                    }
                    "exited" => {
                        let code = event
                            .object()
                            .and_then(|body| body.get("exitCode"))
                            .and_then(Json::as_i64);
                        if let Some(session) = self.session.as_mut() {
                            session.exited(code);
                        }
                        println!(
                            "exited {}",
                            code.map(|code| code.to_string())
                                .unwrap_or_else(|| "without a code".to_owned())
                        );
                        return Ok(());
                    }
                    "terminated" => {
                        if let Some(session) = self.session.as_mut() {
                            session.exited(None);
                        }
                        println!("terminated");
                        return Ok(());
                    }
                    "output" => print_output(&event),
                    "breakpoint" => {
                        let object = event.object();
                        let breakpoint = object
                            .and_then(|body| body.get("breakpoint"))
                            .and_then(Json::as_object);
                        let dap_id = breakpoint
                            .and_then(|breakpoint| breakpoint.get("id"))
                            .and_then(Json::as_i64);
                        let verified = matches!(
                            breakpoint.and_then(|breakpoint| breakpoint.get("verified")),
                            Some(Json::Bool(true))
                        );
                        let line = breakpoint
                            .and_then(|breakpoint| breakpoint.get("line"))
                            .and_then(Json::as_u64)
                            .map(|line| line as u32);
                        if let Some(dap_id) = dap_id {
                            self.book.record_event(dap_id, verified, line);
                        }
                        println!(
                            "breakpoint event: {}{}",
                            if verified { "verified" } else { "unverified" },
                            line.map(|line| format!(" at line {line}"))
                                .unwrap_or_default()
                        );
                    }
                    "initialized" | "process" | "thread" | "module" | "loadedSource"
                    | "capabilities" => {}
                    other => println!("event {other}"),
                },
                Err(ProtocolError::Timeout { .. }) => {
                    println!("no debug event within {} ms", timeout.as_millis());
                    return Ok(());
                }
                Err(error) => return Err(error.render()),
            }
        }
    }

    /// The line of the top frame on a thread, used to enrich a stop that carries no line.
    fn top_location(&mut self, thread: i64) -> Option<u32> {
        let mut arguments = JsonObject::new();
        arguments.insert("threadId", Json::number(thread.max(0) as u64));
        arguments.insert("startFrame", Json::number(0));
        arguments.insert("levels", Json::number(1));
        let response = self.request("stackTrace", Json::Object(arguments)).ok()?;
        let frame = response
            .object()?
            .get("stackFrames")?
            .as_array()?
            .first()?
            .as_object()?;
        frame
            .get("line")
            .and_then(Json::as_u64)
            .map(|line| line as u32)
    }

    fn source_version(&self) -> u64 {
        self.session
            .as_ref()
            .map(Session::source_version)
            .unwrap_or(0)
    }

    fn permit(&self, action: Action) -> Result<(), String> {
        self.session()?
            .permits(action)
            .map_err(|refusal| refusal.render())
    }

    fn resume(&mut self, action: Action, command: &str) -> Result<(), String> {
        self.permit(action)?;
        let thread = self
            .last_thread
            .or_else(|| {
                self.session().ok().and_then(|session| match session.phase {
                    Phase::Stopped { thread, .. } => thread,
                    _ => None,
                })
            })
            .unwrap_or(1);
        let mut arguments = JsonObject::new();
        arguments.insert("threadId", Json::number(thread.max(0) as u64));
        self.request(command, Json::Object(arguments))?;
        if let Some(session) = self.session.as_mut() {
            session.resumed();
        }
        println!("running");
        Ok(())
    }

    fn threads(&mut self) -> Result<(), String> {
        self.permit(Action::Inspect)?;
        let response = self.request("threads", Json::Null)?;
        let threads = response
            .object()
            .and_then(|body| body.get("threads"))
            .and_then(Json::as_array)
            .map(|entries| entries.to_vec())
            .unwrap_or_default();
        if threads.is_empty() {
            println!("no threads reported");
        }
        for entry in threads {
            let Some(object) = entry.as_object() else {
                continue;
            };
            let thread = Thread {
                id: object.get("id").and_then(Json::as_i64).unwrap_or(0),
                name: object
                    .get("name")
                    .and_then(Json::as_str)
                    .unwrap_or("unnamed")
                    .to_owned(),
            };
            println!("thread {} {}", thread.id, thread.name);
        }
        Ok(())
    }

    fn stack(&mut self, thread: Option<i64>) -> Result<(), String> {
        self.permit(Action::Inspect)?;
        let thread = thread.or(self.last_thread).unwrap_or(1);
        let mut arguments = JsonObject::new();
        arguments.insert("threadId", Json::number(thread.max(0) as u64));
        arguments.insert("startFrame", Json::number(0));
        arguments.insert("levels", Json::number(STACK_LIMIT as u64));
        let response = self.request("stackTrace", Json::Object(arguments))?;
        let total = response
            .object()
            .and_then(|body| body.get("totalFrames"))
            .and_then(Json::as_u64)
            .map(|total| total as usize);
        let frames = response
            .object()
            .and_then(|body| body.get("stackFrames"))
            .and_then(Json::as_array)
            .map(|entries| entries.to_vec())
            .unwrap_or_default();
        if frames.is_empty() {
            println!("no stack frames reported");
        }
        for entry in frames {
            let Some(object) = entry.as_object() else {
                continue;
            };
            let frame = Frame {
                id: object.get("id").and_then(Json::as_i64).unwrap_or(0),
                name: object
                    .get("name")
                    .and_then(Json::as_str)
                    .unwrap_or("anonymous")
                    .to_owned(),
                file: object
                    .get("source")
                    .and_then(|source| source.as_object())
                    .and_then(|source| source.get("path"))
                    .and_then(Json::as_str)
                    .map(str::to_owned),
                line: object.get("line").and_then(Json::as_u64).unwrap_or(0) as u32,
                column: object.get("column").and_then(Json::as_u64).unwrap_or(0) as u32,
                source_version: self.source_version(),
            };
            if self.last_frame.is_none() {
                self.last_frame = Some(frame.id);
            }
            println!("frame {} {}", frame.id, frame.render());
        }
        if total.is_some_and(|total| total > STACK_LIMIT) {
            println!("more frames: {} of {}", STACK_LIMIT, total.unwrap_or(0));
        }
        Ok(())
    }

    fn scopes(&mut self, frame: i64) -> Result<(), String> {
        self.permit(Action::Inspect)?;
        self.last_frame = Some(frame);
        let mut arguments = JsonObject::new();
        arguments.insert("frameId", Json::number(frame.max(0) as u64));
        let response = self.request("scopes", Json::Object(arguments))?;
        let scopes = response
            .object()
            .and_then(|body| body.get("scopes"))
            .and_then(Json::as_array)
            .map(|entries| entries.to_vec())
            .unwrap_or_default();
        for entry in scopes {
            let Some(object) = entry.as_object() else {
                continue;
            };
            let scope = Scope {
                name: object
                    .get("name")
                    .and_then(Json::as_str)
                    .unwrap_or("scope")
                    .to_owned(),
                variables_reference: object
                    .get("variablesReference")
                    .and_then(Json::as_i64)
                    .unwrap_or(0),
                expensive: matches!(object.get("expensive"), Some(Json::Bool(true))),
                count: object
                    .get("namedVariables")
                    .and_then(Json::as_u64)
                    .map(|count| count as usize),
            };
            println!(
                "scope {} reference {}{}{}",
                scope.name,
                scope.variables_reference,
                if scope.expensive { " (expensive)" } else { "" },
                scope
                    .count
                    .map(|count| format!(" ({count} variables)"))
                    .unwrap_or_default()
            );
            if let Some(count) = scope.count {
                self.reference_counts
                    .insert(scope.variables_reference, count);
            }
        }
        Ok(())
    }

    fn variables(&mut self, reference: i64, count: usize) -> Result<(), String> {
        self.permit(Action::Inspect)?;
        // Ask for no more than the container is known to hold. GDB's DAP `fetch_children` indexes
        // a list sized by the child count with the *requested* count, so `count` larger than the
        // container raises `list index out of range` instead of clamping. The DAP contract allows
        // a client to ask for a page, not to require the adapter to over-read.
        let known = self.reference_counts.get(&reference).copied();
        let count = match known {
            Some(known) => count.min(PAGE_LIMIT).min(known),
            None => count.min(PAGE_LIMIT),
        };
        let mut arguments = JsonObject::new();
        arguments.insert("variablesReference", Json::number(reference.max(0) as u64));
        arguments.insert("start", Json::number(0));
        arguments.insert("count", Json::number(count as u64));
        let response = self.request("variables", Json::Object(arguments))?;
        let entries = response
            .object()
            .and_then(|body| body.get("variables"))
            .and_then(Json::as_array)
            .map(|entries| entries.to_vec())
            .unwrap_or_default();
        let variables: Vec<Variable> = entries
            .iter()
            .filter_map(|entry| {
                let object = entry.as_object()?;
                Some(Variable {
                    name: object
                        .get("name")
                        .and_then(Json::as_str)
                        .unwrap_or("?")
                        .to_owned(),
                    value: object
                        .get("value")
                        .and_then(Json::as_str)
                        .unwrap_or("")
                        .to_owned(),
                    type_name: object.get("type").and_then(Json::as_str).map(str::to_owned),
                    variables_reference: object
                        .get("variablesReference")
                        .and_then(Json::as_i64)
                        .unwrap_or(0),
                    named: object
                        .get("namedVariables")
                        .and_then(Json::as_u64)
                        .map(|count| count as usize),
                    indexed: object
                        .get("indexedVariables")
                        .and_then(Json::as_u64)
                        .map(|count| count as usize),
                })
            })
            .collect();
        for variable in &variables {
            if variable.variables_reference != 0
                && let Some(count) = variable.named.or(variable.indexed)
            {
                self.reference_counts
                    .insert(variable.variables_reference, count);
            }
        }
        let page = inspect::page(variables, 0, count, None);
        println!("variables {}", page.render());
        for variable in &page.variables {
            println!("variable {}", variable.render());
        }
        Ok(())
    }

    fn evaluate(&mut self, expression: &str, watch_index: Option<usize>) -> Result<(), String> {
        self.permit(Action::Evaluate)?;
        let decision = inspect::decide(self.policy, expression);
        let evaluation = match decision {
            EvalDecision::Refuse { reason } => Evaluation::Refused { reason },
            EvalDecision::Confirm { prompt } => Evaluation::Refused { reason: prompt },
            EvalDecision::Run => {
                let mut arguments = JsonObject::new();
                arguments.insert("expression", Json::string(expression));
                // The context is an adapter fact, not a preference: GDB's `repl` context runs its
                // own commands (`n` = `next`), while `variables` evaluates the expression in the
                // selected frame. Sending `repl` to GDB silently stepped the program.
                let context = self
                    .adapter
                    .as_ref()
                    .map(|adapter| adapter.evaluate_context)
                    .unwrap_or("repl");
                arguments.insert("context", Json::string(context));
                if let Some(frame) = self.last_frame {
                    arguments.insert("frameId", Json::number(frame.max(0) as u64));
                }
                match self.request("evaluate", Json::Object(arguments)) {
                    Ok(response) => match response.object() {
                        Some(body) => Evaluation::Value {
                            value: body
                                .get("result")
                                .and_then(Json::as_str)
                                .unwrap_or("")
                                .to_owned(),
                            type_name: body.get("type").and_then(Json::as_str).map(str::to_owned),
                            variables_reference: body
                                .get("variablesReference")
                                .and_then(Json::as_i64)
                                .unwrap_or(0),
                        },
                        None => Evaluation::Error {
                            message: "the adapter returned no result".to_owned(),
                        },
                    },
                    Err(reason) if reason.contains("timed out") => Evaluation::TimedOut {
                        milliseconds: self.timeout.as_millis() as u64,
                    },
                    Err(reason) => Evaluation::Error { message: reason },
                }
            }
        };
        if let Some(index) = watch_index {
            self.watches.record(index, evaluation.clone())?;
        }
        println!("evaluate {}", evaluation.render());
        Ok(())
    }

    fn break_source(
        &mut self,
        file: &str,
        line: u32,
        condition: Option<String>,
    ) -> Result<(), String> {
        let id = self.book.add_source(file, line)?;
        if let Some(condition) = condition {
            self.book.condition(id, &condition)?;
        }
        if self.client.is_some() {
            self.apply_breakpoints(file)?;
        }
        match self.book.get(id) {
            Some(point) => println!("{}", point.render()),
            None => println!("breakpoint {id} at {file}:{line}"),
        }
        Ok(())
    }

    fn logpoint(&mut self, file: &str, line: u32, message: &str) -> Result<(), String> {
        let id = self.book.add_source(file, line)?;
        self.book.log_message(id, message)?;
        if self.client.is_some() {
            self.apply_breakpoints(file)?;
        }
        match self.book.get(id) {
            Some(point) => println!("{}", point.render()),
            None => println!("breakpoint {id} at {file}:{line}"),
        }
        Ok(())
    }

    fn hit_condition(&mut self, file: &str, line: u32, condition: &str) -> Result<(), String> {
        let id = self.book.add_source(file, line)?;
        self.book.hit_condition(id, condition)?;
        if self.client.is_some() {
            self.apply_breakpoints(file)?;
        }
        match self.book.get(id) {
            Some(point) => println!("{}", point.render()),
            None => println!("breakpoint {id} at {file}:{line}"),
        }
        Ok(())
    }

    fn resend_breakpoints(&mut self) -> Result<(), String> {
        if self.client.is_some() {
            self.apply_breakpoints_all()?;
        }
        Ok(())
    }

    fn status(&self) {
        match (&self.adapter, &self.session) {
            (Some(adapter), Some(session)) => {
                println!("adapter {}; {}", adapter.name, session.render());
            }
            (Some(adapter), None) => println!("adapter {}; no session", adapter.name),
            (None, Some(session)) => println!("no adapter selected; {}", session.render()),
            (None, None) => println!("no adapter and no session"),
        }
        println!(
            "policy {}; {} breakpoint(s); {} watch(es)",
            self.policy.name(),
            self.book.points().len(),
            self.watches.entries().len()
        );
    }

    /// Ends the session, sending the disconnect the session says is right.
    fn cleanup(&mut self, announce: bool) {
        if let Some(client) = self.client.as_mut() {
            if let Some(session) = self.session.as_ref() {
                let disconnect = session.disconnect();
                let mut arguments = JsonObject::new();
                arguments.insert(
                    "terminateDebuggee",
                    Json::bool(disconnect.terminate_debuggee),
                );
                let _ = client.request(
                    "disconnect",
                    &Json::Object(arguments),
                    Duration::from_secs(2),
                );
                if announce {
                    println!("disconnect: {}", disconnect.reason);
                }
            }
            client.terminate(Duration::from_secs(2));
        }
        self.client = None;
        if let Some(session) = self.session.as_mut() {
            session.ended();
        }
    }

    /// Reports a failure after cleaning up, so no adapter process is left behind.
    fn fail(&mut self, reason: String) -> bool {
        self.cleanup(false);
        self.failed = true;
        report(reason)
    }
}

fn print_output(event: &crate::app::debug::protocol::Event) {
    if let Some(output) = event
        .object()
        .and_then(|body| body.get("output"))
        .and_then(Json::as_str)
    {
        for line in output.lines() {
            if !line.trim().is_empty() {
                println!("output {}", line.trim_end());
            }
        }
    }
}

/// Splits `<file>:<line>[:<condition>]` without confusing a path's colons for a separator.
fn file_line(rest: &str) -> Result<(String, u32, Option<String>), String> {
    let (head, tail) = rest
        .rsplit_once(':')
        .ok_or_else(|| format!("needs <file>:<line>, got {rest:?}"))?;
    if let Ok(line) = tail.parse::<u32>() {
        return Ok((head.to_owned(), line, None));
    }
    let (file, line) = head
        .rsplit_once(':')
        .ok_or_else(|| format!("needs <file>:<line>[:<condition>], got {rest:?}"))?;
    let line = line
        .parse::<u32>()
        .map_err(|_| format!("{line:?} is not a line number"))?;
    Ok((file.to_owned(), line, Some(tail.to_owned())))
}

/// Drives a debug session (tasks T19.01–T19.08).
pub(super) fn debug(options: &crate::cli::DebugOptions) -> bool {
    let mut machine = Machine::new();
    let mut launch_program: Option<String> = None;
    let mut launch_arguments: Vec<String> = Vec::new();
    let mut directory: Option<PathBuf> = None;
    let mut check = false;

    let outcome = (|| -> Result<(), String> {
        for op in &options.ops {
            let (verb, rest) = op.split_once(':').unwrap_or((op.as_str(), ""));
            let result: Result<(), String> = match verb {
                "probe" => {
                    for adapter in adapter::table(machine.host, &machine.probe) {
                        println!("{}", adapter.render());
                    }
                    let ready = adapter::table(machine.host, &machine.probe)
                        .into_iter()
                        .filter(|adapter| adapter.state.ready())
                        .count();
                    println!("adapters: {ready} ready on {}", machine.host.name());
                    Ok(())
                }
                "host" => Os::parse(rest)
                    .map(|host| {
                        machine.host = host;
                        machine.probe = Probe::from_path();
                    })
                    .ok_or_else(|| format!("{rest:?} is not macos, linux or windows")),
                "adapter" => machine.select_adapter(rest),
                "timeout" => match rest.parse::<u64>() {
                    Ok(milliseconds) => {
                        machine.timeout = Duration::from_millis(milliseconds);
                        Ok(())
                    }
                    Err(_) => Err(format!("{rest:?} is not a timeout in milliseconds")),
                },
                "authorize" => {
                    machine.authorized = true;
                    if let Some(session) = machine.session.as_mut() {
                        session.authorize();
                    }
                    println!("authorized");
                    Ok(())
                }
                "launch" => {
                    launch_program = Some(rest.to_owned());
                    // A launch can be given after its arguments and directory, so it is deferred until
                    // the script has collected them: `launch:p arg:a arg:b` is the order people type.
                    Ok(())
                }
                "arg" => {
                    launch_arguments.push(rest.to_owned());
                    Ok(())
                }
                "directory" => {
                    directory = Some(PathBuf::from(rest));
                    Ok(())
                }
                "source-version" => match rest.parse::<u64>() {
                    Ok(version) => {
                        machine.wanted_source_version = version;
                        if let Some(session) = machine.session.as_mut()
                            && let Some(config) = session.config.as_mut()
                        {
                            config.source_version = version;
                        }
                        println!("source version {version}");
                        Ok(())
                    }
                    Err(_) => Err(format!("{rest:?} is not a source version")),
                },
                "stop-on-entry" => match rest {
                    "yes" | "true" => {
                        machine.stop_on_entry = true;
                        Ok(())
                    }
                    "no" | "false" => {
                        machine.stop_on_entry = false;
                        Ok(())
                    }
                    other => Err(format!("{other:?} is not yes or no")),
                },
                "start" => {
                    let Some(program) = launch_program.clone() else {
                        return Err("start needs launch:<program>".to_owned());
                    };
                    machine.launch(&program, launch_arguments.clone(), directory.clone())
                }
                "attach" => {
                    let pid = rest
                        .parse::<u32>()
                        .map_err(|_| format!("{rest:?} is not a process id"))?;
                    machine.attach(pid)
                }
                "break" => file_line(rest).and_then(|(file, line, condition)| {
                    machine.break_source(&file, line, condition)
                }),
                "logpoint" => {
                    let mut parts = rest.splitn(3, ':');
                    let (Some(file), Some(line), Some(message)) =
                        (parts.next(), parts.next(), parts.next())
                    else {
                        return Err(format!(
                            "logpoint needs <file>:<line>:<message>, got {rest:?}"
                        ));
                    };
                    let line = line
                        .parse::<u32>()
                        .map_err(|_| format!("{line:?} is not a line number"))?;
                    machine.logpoint(file, line, message)
                }
                "hit" => {
                    let (file, line, condition) = file_line(rest)?;
                    let condition = condition.ok_or_else(|| {
                        format!("hit needs <file>:<line>:<condition>, got {rest:?}")
                    })?;
                    machine.hit_condition(&file, line, &condition)
                }
                "breakpoints" => {
                    for point in machine.book.points() {
                        println!("{}", point.render());
                    }
                    println!("{} breakpoint(s)", machine.book.points().len());
                    Ok(())
                }
                "enable-all" => {
                    machine.book.enable_all();
                    machine.resend_breakpoints()
                }
                "disable-all" => {
                    machine.book.disable_all();
                    machine.resend_breakpoints()
                }
                "remove-breakpoint" => {
                    let id = rest
                        .parse::<u32>()
                        .map_err(|_| format!("{rest:?} is not a breakpoint id"))?;
                    machine.book.remove(id)?;
                    machine.resend_breakpoints()
                }
                "continue" => machine.resume(Action::Continue, "continue"),
                "pause" => {
                    machine.permit(Action::Pause)?;
                    let thread = machine
                        .last_thread
                        .or_else(|| {
                            machine
                                .session()
                                .ok()
                                .and_then(|session| match &session.phase {
                                    Phase::Stopped { thread, .. } => *thread,
                                    _ => None,
                                })
                        })
                        .unwrap_or(1);
                    let mut arguments = JsonObject::new();
                    arguments.insert("threadId", Json::number(thread.max(0) as u64));
                    machine.request("pause", Json::Object(arguments))?;
                    Ok(())
                }
                "step-over" => machine.resume(Action::StepOver, "next"),
                "step-in" => machine.resume(Action::StepIn, "stepIn"),
                "step-out" => machine.resume(Action::StepOut, "stepOut"),
                "run-to" => {
                    machine.permit(Action::RunToCursor)?;
                    let (file, line, _) = file_line(rest)?;
                    machine.break_source(&file, line, None)?;
                    machine.resume(Action::RunToCursor, "continue")
                }
                "wait" => {
                    let timeout = if rest.is_empty() {
                        machine.timeout
                    } else {
                        Duration::from_millis(
                            rest.parse::<u64>()
                                .map_err(|_| format!("{rest:?} is not a wait in milliseconds"))?,
                        )
                    };
                    machine.wait(timeout)
                }
                "threads" => machine.threads(),
                "stack" => {
                    let thread = if rest.is_empty() {
                        None
                    } else {
                        Some(
                            rest.parse::<i64>()
                                .map_err(|_| format!("{rest:?} is not a thread id"))?,
                        )
                    };
                    machine.stack(thread)
                }
                "scopes" => {
                    let frame = rest
                        .parse::<i64>()
                        .map_err(|_| format!("{rest:?} is not a frame id"))?;
                    machine.scopes(frame)
                }
                "variables" => {
                    let (reference, count) = rest
                        .split_once(':')
                        .map(|(reference, count)| {
                            (reference, count.parse::<usize>().unwrap_or(PAGE_LIMIT))
                        })
                        .unwrap_or((rest, PAGE_LIMIT));
                    let reference = reference
                        .parse::<i64>()
                        .map_err(|_| format!("{reference:?} is not a variables reference"))?;
                    machine.variables(reference, count)
                }
                "evaluate" => machine.evaluate(rest, None),
                "watch" => {
                    let index = machine.watches.add(rest)?;
                    machine.evaluate(rest, Some(index))
                }
                "watches" => {
                    for line in machine.watches.render() {
                        println!("{line}");
                    }
                    Ok(())
                }
                "policy" => EvalPolicy::parse(rest)
                    .map(|policy| machine.policy = policy)
                    .ok_or_else(|| format!("{rest:?} is not allow, ask or deny")),
                "status" => {
                    machine.status();
                    Ok(())
                }
                "disconnect" | "stop" => match machine.session()?.permits(Action::Disconnect) {
                    Ok(()) => {
                        machine.cleanup(true);
                        println!("disconnected");
                        Ok(())
                    }
                    Err(refusal) => Err(refusal.render()),
                },
                "hot-swap" => {
                    let confirmed = rest == "confirm";
                    let live = machine
                        .session
                        .as_ref()
                        .map(|session| session.phase.clone())
                        .map(|phase| phase.is_live())
                        .unwrap_or(false);
                    let decision =
                        hotswap::permits(hotswap::Operation::ReplaceLogic, live, false, confirmed);
                    println!("hot-swap {}", decision.render());
                    match decision {
                        hotswap::Decision::Allowed => {
                            let aftermath = hotswap::after_swap(
                                live,
                                machine
                                    .session
                                    .as_ref()
                                    .map(|session| session.attached)
                                    .unwrap_or(false),
                            );
                            if live {
                                machine.cleanup(true);
                            }
                            println!("hot-swap {}", aftermath.detail);
                            Ok(())
                        }
                        hotswap::Decision::Confirm { .. } | hotswap::Decision::Refused { .. } => {
                            machine.failed = true;
                            Ok(())
                        }
                    }
                }
                "recover" => {
                    let incident = match rest {
                        "adapter-exit" => Incident::AdapterExit { code: None },
                        other => Incident::UnsupportedTarget {
                            target: other.to_owned(),
                        },
                    };
                    let owned = machine
                        .session
                        .as_ref()
                        .map(|session| session.owns_process)
                        .unwrap_or(false);
                    let recovery = recovery::recover(incident, owned);
                    println!("recovery {}", recovery.render());
                    println!("recovery {}", recovery.detail);
                    machine.cleanup(false);
                    Ok(())
                }
                "check" => {
                    check = true;
                    Ok(())
                }
                "show" => Ok(()),
                other => Err(format!("{other:?} is not a debug operation")),
            };
            result?;
        }
        Ok(())
    })();
    if let Err(reason) = outcome {
        return machine.fail(reason);
    }

    if check {
        println!("debug: {}", if machine.failed { "failed" } else { "clean" });
    }
    !machine.failed
}
