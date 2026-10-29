// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The workbench's debug panel (tasks T19.03–T19.06).
//!
//! The panel is a view over a **real** DAP session: a worker thread compiles a debuggee, starts
//! the adapter the probe found, binds a breakpoint, stops, and asks the adapter for threads,
//! stack frames, scopes and variables. The window never blocks on the protocol; it polls the
//! snapshot channel and mirrors the last report into a list.
//!
//! Nothing here invents a session. If no adapter is installed, the panel says which one is
//! missing. A breakpoint is `bound` only after the adapter answered `verified: true`. A stop is
//! reported with the adapter's reason and line. The debuggee is generated into
//! `target/rustage-debug/` and compiled by the real `rustc` on this host.
//!
//! The panel does not claim two-way source navigation: it shows the adapter's raw source position
//! and says that no generated [`SourceMap`] is loaded for
//! this debuggee, rather than guessing a node from a nearby line.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

use crate::app::debug::adapter::{self, Adapter};
use crate::app::debug::breakpoints::{Anchor, Book};
use crate::app::debug::inspect::{PAGE_LIMIT, STACK_LIMIT};
use crate::app::debug::protocol::{Client, ProtocolError, Response};
use crate::app::targets::configuration::Os;
use crate::app::targets::local::Probe;
use crate::contracts::identity::NodeId;
use crate::contracts::revision::{GeneratorVersion, Revision};
use crate::contracts::source_map::{SourceMap, SourceSpan};
use crate::project::record::json::{Json, JsonObject};

/// How long one protocol request may take.
const TIMEOUT: Duration = Duration::from_secs(10);

/// One stack row: frame id, function, file, line and column.
type FrameRow = (i64, String, Option<String>, u32, u32);

/// One scope row: its name and the variables the panel shows for it.
type LocalPage = (String, Vec<crate::app::debug::inspect::Variable>);

/// A command the panel sends to the worker.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Command {
    /// Start a new session (handled by the panel, never sent to a worker).
    Start,
    /// Continue to the next breakpoint or the end.
    Continue,
    /// Step over one line.
    Step,
    /// End the session and terminate the adapter.
    Stop,
}

impl Command {
    /// The command's name, for the status line.
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Continue => "continue",
            Self::Step => "step over",
            Self::Stop => "stop",
        }
    }
}

/// One report from the worker: what phase it reached, the one-line status and the panel rows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Update {
    /// `starting`, `stopped`, `exited`, `ended` or `failed`.
    pub phase: &'static str,
    /// The one-line status.
    pub status: String,
    /// The rows the panel shows.
    pub rows: Vec<String>,
}

/// The panel's view of one session and the channels that connect it to the worker.
pub(super) struct DebugState {
    rows: Vec<String>,
    commands: Option<Sender<Command>>,
    updates: Receiver<Update>,
    live: bool,
}

impl DebugState {
    /// No session yet.
    pub(super) fn new() -> Self {
        let (_sender, updates) = channel();
        Self {
            rows: vec!["no debug session; press Start".to_owned()],
            commands: None,
            updates,
            live: false,
        }
    }

    /// Starts a worker unless one is already live.
    ///
    /// `nodes` are the design nodes the debuggee is generated from and `break_node` is the node
    /// whose generated statement the first breakpoint anchors on. They come from the same scene
    /// the canvas and Inspector show, so the source map is about real nodes rather than names
    /// invented by the panel.
    pub(super) fn start(&mut self, nodes: Vec<(NodeId, String)>, break_node: NodeId) -> bool {
        if self.live {
            return false;
        }
        let (commands, commands_rx) = channel();
        let (updates, updates_rx) = channel();
        self.commands = Some(commands);
        self.updates = updates_rx;
        self.live = true;
        self.rows = vec!["starting a real debug session…".to_owned()];
        std::thread::spawn(move || worker(commands_rx, updates, nodes, break_node));
        true
    }

    /// Sends a command; `false` when no worker is running.
    pub(super) fn send(&mut self, command: Command) -> bool {
        self.commands
            .as_ref()
            .is_some_and(|sender| sender.send(command).is_ok())
    }

    /// The latest update, if one arrived since the last poll.
    pub(super) fn poll(&mut self) -> Option<Update> {
        let mut last = None;
        loop {
            match self.updates.try_recv() {
                Ok(update) => {
                    if matches!(update.phase, "exited" | "ended" | "failed") {
                        self.live = false;
                    }
                    last = Some(update);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.live = false;
                    break;
                }
            }
        }
        if let Some(update) = &last {
            self.rows = update.rows.clone();
        }
        last
    }

    /// The rows the list currently shows.
    pub(super) fn rows(&self) -> Vec<String> {
        self.rows.clone()
    }

    /// Whether a worker is still running.
    pub(super) fn is_live(&self) -> bool {
        self.live
    }
}

impl Default for DebugState {
    fn default() -> Self {
        Self::new()
    }
}

/// The worker loop: start a session, then serve commands until Stop or the channel closes.
fn worker(
    commands: Receiver<Command>,
    updates: Sender<Update>,
    nodes: Vec<(NodeId, String)>,
    break_node: NodeId,
) {
    let mut worker = match Worker::start(updates.clone(), nodes, break_node) {
        Ok(worker) => worker,
        Err(reason) => {
            let _ = updates.send(Update {
                phase: "failed",
                status: format!("session failed: {reason}"),
                rows: vec![format!("failed: {reason}")],
            });
            return;
        }
    };
    loop {
        match commands.recv() {
            Ok(Command::Start) => {}
            Ok(Command::Continue) => {
                if let Err(reason) = worker.resume("continue") {
                    let _ = worker.updates.send(Update {
                        phase: "failed",
                        status: format!("continue failed: {reason}"),
                        rows: vec![format!("failed: {reason}")],
                    });
                    worker.stop();
                    return;
                }
                if !worker.is_live() {
                    return;
                }
            }
            Ok(Command::Step) => {
                if let Err(reason) = worker.resume("next") {
                    let _ = worker.updates.send(Update {
                        phase: "failed",
                        status: format!("step failed: {reason}"),
                        rows: vec![format!("failed: {reason}")],
                    });
                    worker.stop();
                    return;
                }
                if !worker.is_live() {
                    return;
                }
            }
            Ok(Command::Stop) => {
                worker.stop();
                let _ = worker.updates.send(Update {
                    phase: "ended",
                    status: "session ended".to_owned(),
                    rows: vec!["session ended".to_owned()],
                });
                return;
            }
            Err(_) => {
                worker.stop();
                return;
            }
        }
    }
}

/// One live session and the book keeping the panel reports.
struct Worker {
    client: Client,
    book: Book,
    updates: Sender<Update>,
    source: PathBuf,
    source_file: String,
    map: SourceMap,
    line_offsets: Vec<u32>,
    break_line: u32,
    adapter: Adapter,
    thread: Option<i64>,
    live: bool,
}

impl Worker {
    /// Compiles the generated debuggee, launches it and stops at the mapped breakpoint.
    fn start(
        updates: Sender<Update>,
        nodes: Vec<(NodeId, String)>,
        break_node: NodeId,
    ) -> Result<Self, String> {
        let prepared = prepare_debuggee(&nodes, break_node)?;
        let source = prepared.source;
        let program = prepared.program;
        let adapter = adapter::first_ready(Os::host(), &Probe::from_path())
            .map_err(|refusal| refusal.render())?;
        let adapter_program = adapter
            .program
            .clone()
            .ok_or_else(|| format!("{} has no program on this machine", adapter.name))?;
        let client = Client::spawn(&adapter_program, &adapter.arguments, None, &[], true)
            .map_err(|error| error.render())?;
        let mut worker = Self {
            client,
            book: Book::new(),
            updates,
            source,
            source_file: prepared.source_file,
            map: prepared.map,
            line_offsets: prepared.line_offsets,
            break_line: prepared.break_line,
            adapter,
            thread: None,
            live: true,
        };
        if let Err(reason) = worker.launch(&program) {
            worker.stop();
            return Err(reason);
        }
        // GDB answers `launch` while staged at main and does not list a thread until the first
        // resume, but DAP's `continue` requires a `threadId`. The staged entry is the program's
        // first thread (the same convention the CLI session uses, not a guess about a user
        // thread). An adapter that starts directly is already running, so it only has to wait.
        let outcome = if worker.adapter.stages_breakpoints_at_entry {
            worker.thread = Some(1);
            worker.resume("continue")
        } else {
            worker.wait_stopped()
        };
        if let Err(reason) = outcome {
            worker.stop();
            return Err(reason);
        }
        Ok(worker)
    }

    /// Whether the program can still be inspected.
    fn is_live(&self) -> bool {
        self.live
    }

    fn launch(&mut self, program: &Path) -> Result<(), String> {
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
        self.request("initialize", &Json::Object(arguments))?;

        // `prepare_debuggee` compiles with an absolute source path and launches with its
        // directory, so this is the path GDB recorded in the compilation unit. A relative name
        // stays `pending` because GDB's source lookup does not find it.
        let file = self.source.display().to_string();
        self.book.add_source(file.clone(), self.break_line)?;

        let mut launch = JsonObject::new();
        launch.insert("program", Json::string(program.display().to_string()));
        launch.insert("args", Json::Array(Vec::new()));
        let directory = program
            .parent()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        if !directory.is_empty() {
            launch.insert("cwd", Json::string(directory));
        }
        // GDB's DAP launch runs the program and treats `configurationDone` as a no-op, so it must
        // stop at main before the breakpoint is installed. The adapter flag says which behaviour
        // this installation has; the flag is not guessed from its name.
        if self.adapter.stages_breakpoints_at_entry {
            launch.insert("stopAtBeginningOfMainSubprogram", Json::bool(true));
        } else {
            launch.insert("stopOnEntry", Json::bool(false));
        }
        launch.insert("console", Json::string("internalConsole"));
        launch.insert("sourceLanguages", Json::array([Json::string("rust")]));
        let launch_seq = self
            .client
            .send_request("launch", &Json::Object(launch))
            .map_err(|error| error.render())?;
        self.wait_for_initialized()?;
        self.request("configurationDone", &Json::Null)?;
        let response = self
            .client
            .await_response(launch_seq, "launch", TIMEOUT)
            .map_err(|error| error.render())?;
        if !response.success {
            return Err(format!(
                "launch was refused by the adapter: {}",
                response.message.unwrap_or_else(|| "no message".to_owned())
            ));
        }
        // GDB stages at main and only then has the source loaded. Setting the breakpoint after
        // the launch answer is the order its DAP implementation expects; setting it before the
        // load answers `verified: false` and the program can pass the line.
        self.apply_breakpoints(&file)?;
        Ok(())
    }

    fn wait_for_initialized(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::ZERO);
            if remaining.is_zero() {
                return Err("the adapter did not send initialized before the deadline".to_owned());
            }
            match self.client.event(remaining) {
                Ok(event) if event.event == "initialized" => return Ok(()),
                Ok(_) => {}
                Err(ProtocolError::Timeout { .. }) => {
                    return Err(
                        "the adapter did not send initialized before the deadline".to_owned()
                    );
                }
                Err(error) => return Err(error.render()),
            }
        }
    }

    /// Installs the book's breakpoints and records the adapter's own binding answer.
    ///
    /// GDB answers `setBreakpoints` with `notStopped` while it is still finishing the launch. The
    /// CLI session waits before asking; this retries the same request instead, so the panel does
    /// not depend on a fixed sleep being long enough on a slower machine.
    fn apply_breakpoints(&mut self, file: &str) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let body = self.book.set_breakpoints_body(file);
        let response = loop {
            match self.request("setBreakpoints", &body) {
                Ok(response) => break response,
                Err(reason) if reason.contains("notStopped") && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(200));
                }
                Err(reason) => return Err(reason),
            }
        };
        let entries = response
            .object()
            .and_then(|body| body.get("breakpoints"))
            .and_then(Json::as_array)
            .map(|entries| entries.to_vec())
            .unwrap_or_default();
        let ids: Vec<u32> = self.book.points().iter().map(|point| point.id).collect();
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

    /// An inspection request, retrying while the adapter still says `notStopped`.
    ///
    /// GDB reports the `stopped` event just before its own state machine settles; a `threads`
    /// request in that window is refused with `notStopped`. Retrying a bounded number of times
    /// reads the stop the event announced without inventing one.
    fn inspect_request(&mut self, command: &str, arguments: &Json) -> Result<Response, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match self.request(command, arguments) {
                Ok(response) => return Ok(response),
                Err(reason) if reason.contains("notStopped") && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(reason) => return Err(reason),
            }
        }
    }

    fn request(&mut self, command: &str, arguments: &Json) -> Result<Response, String> {
        let response = self
            .client
            .request(command, arguments, TIMEOUT)
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

    /// Sends a resume command, then waits for the next stop or the program's exit.
    fn resume(&mut self, command: &str) -> Result<(), String> {
        let mut arguments = JsonObject::new();
        if let Some(thread) = self.thread {
            arguments.insert("threadId", Json::number(thread.max(0) as u64));
        }
        if command == "continue" {
            arguments.insert("singleThread", Json::bool(false));
        }
        self.request(command, &Json::Object(arguments))?;
        self.wait_stopped()
    }

    fn wait_stopped(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::ZERO);
            if remaining.is_zero() {
                return Err("no stop event arrived before the deadline".to_owned());
            }
            match self.client.event(remaining) {
                Ok(event) => {
                    match event.event.as_str() {
                        "stopped" => {
                            let body = event.object();
                            let reason = body
                                .and_then(|body| body.get("reason"))
                                .and_then(Json::as_str)
                                .unwrap_or("stopped")
                                .to_owned();
                            let thread = body
                                .and_then(|body| body.get("threadId"))
                                .and_then(Json::as_i64);
                            let line = body
                                .and_then(|body| body.get("line"))
                                .and_then(Json::as_u64)
                                .map(|line| line as u32);
                            self.thread = thread;
                            crate::host::log::emit(
                                crate::host::log::Level::Info,
                                "app::run::workbench-debug",
                                &format!("stop event: {reason} line {line:?} thread {thread:?}"),
                            );
                            // A stop event can be queued just before the adapter resumes the program
                            // (GDB re-resolves a pending breakpoint during a step). If inspection is
                            // refused because the program is no longer stopped, keep waiting rather
                            // than reporting a stop that is already over.
                            match self.refresh(&reason, line) {
                                Ok(()) => return Ok(()),
                                Err(reason) => {
                                    crate::host::log::emit(
                                        crate::host::log::Level::Info,
                                        "app::run::workbench-debug",
                                        &format!("stale stop event ignored: {reason}"),
                                    );
                                }
                            }
                        }
                        "exited" => {
                            let code = event
                                .object()
                                .and_then(|body| body.get("exitCode"))
                                .and_then(Json::as_i64);
                            self.live = false;
                            let status = code
                                .map(|code| format!("exited with code {code}"))
                                .unwrap_or_else(|| "exited without a code".to_owned());
                            let _ = self.updates.send(Update {
                                phase: "exited",
                                status,
                                rows: self.base_rows(),
                            });
                            return Ok(());
                        }
                        "terminated" => {
                            self.live = false;
                            let _ = self.updates.send(Update {
                                phase: "exited",
                                status: "terminated".to_owned(),
                                rows: self.base_rows(),
                            });
                            return Ok(());
                        }
                        "breakpoint" => {
                            // A request is not a binding: this event is where the adapter reports
                            // that a staged breakpoint really resolved to a line.
                            let breakpoint = event
                                .object()
                                .and_then(|body| body.get("breakpoint"))
                                .and_then(Json::as_object);
                            if let Some(dap_id) = breakpoint
                                .and_then(|breakpoint| breakpoint.get("id"))
                                .and_then(Json::as_i64)
                            {
                                let verified = matches!(
                                    breakpoint.and_then(|breakpoint| breakpoint.get("verified")),
                                    Some(Json::Bool(true))
                                );
                                let line = breakpoint
                                    .and_then(|breakpoint| breakpoint.get("line"))
                                    .and_then(Json::as_u64)
                                    .map(|line| line as u32);
                                self.book.record_event(dap_id, verified, line);
                            }
                        }
                        _ => {}
                    }
                }
                Err(ProtocolError::Timeout { .. }) => {
                    return Err("no stop event arrived before the deadline".to_owned());
                }
                Err(error) => return Err(error.render()),
            }
        }
    }

    /// Reads threads, the top frame, its first scope and that scope's variables, then reports.
    fn refresh(&mut self, reason: &str, line: Option<u32>) -> Result<(), String> {
        let mut rows = self.base_rows();
        let threads = self.threads()?;
        let thread = self
            .thread
            .or_else(|| threads.first().map(|(id, _)| *id))
            .unwrap_or(1);
        for (id, name) in &threads {
            rows.push(format!("thread {id} {name}"));
        }
        let frames = self.stack(thread)?;
        for (index, (_id, name, file, frame_line, column)) in frames.iter().enumerate() {
            rows.push(format!(
                "frame {index} {name} at {}:{frame_line}:{column}",
                file.as_deref().map(short).unwrap_or("no source")
            ));
        }
        if let Some((frame_id, _name, file, frame_line, column)) = frames.first() {
            let (scope, variables) = self.locals(*frame_id)?;
            rows.push(scope);
            // The panel shows one representative local so the source row still fits; the log
            // carries every row the adapter returned.
            if let Some(variable) = variables.first() {
                rows.push(format!("variable {}", variable.render()));
            }
            rows.push(format!(
                "source {}:{frame_line}:{column}{}",
                file.as_deref().map(short).unwrap_or("no source"),
                self.describe_position(file.as_deref(), *frame_line)
            ));
            let _ = self.updates.send(Update {
                phase: "stopped",
                status: format!(
                    "stopped: {reason} at line {} (thread {thread})",
                    line.or(Some(*frame_line)).unwrap_or(0)
                ),
                rows,
            });
        } else {
            rows.push("no stack frames reported".to_owned());
            let _ = self.updates.send(Update {
                phase: "stopped",
                status: format!("stopped: {reason} (thread {thread})"),
                rows,
            });
        }
        Ok(())
    }

    fn base_rows(&self) -> Vec<String> {
        let mut rows = vec![format!(
            "adapter {}: {}",
            self.adapter.name,
            self.adapter.capabilities.render()
        )];
        for point in self.book.points() {
            match &point.anchor {
                Anchor::Source { file, line } => rows.push(format!(
                    "breakpoint {} at {}:{line}: {}{}",
                    point.id,
                    short(file),
                    point.binding.render(),
                    self.describe_position(Some(file), *line)
                )),
                Anchor::Node { .. } => rows.push(point.render()),
            }
        }
        rows
    }

    /// What the source map says about a stopped position: the node whose generated construct
    /// contains the line, or an explicit `unmapped` answer when no entry covers it.
    fn describe_position(&self, file: Option<&str>, line: u32) -> String {
        let Some(file) = file else {
            return " (unmapped: the frame has no source)".to_owned();
        };
        if file != self.source_file {
            return format!(" (unmapped: no generated source map for {})", short(file));
        }
        let node = self
            .line_offsets
            .get(line.saturating_sub(1) as usize)
            .and_then(|offset| self.map.node_at(file, *offset));
        match node {
            Some(node) => format!(" -> node {}", node.get()),
            None => " (unmapped: no node covers this line)".to_owned(),
        }
    }

    fn threads(&mut self) -> Result<Vec<(i64, String)>, String> {
        let response = self.inspect_request("threads", &Json::Null)?;
        let entries = response
            .object()
            .and_then(|body| body.get("threads"))
            .and_then(Json::as_array)
            .map(|entries| entries.to_vec())
            .unwrap_or_default();
        Ok(entries
            .iter()
            .filter_map(|entry| {
                let object = entry.as_object()?;
                Some((
                    object.get("id").and_then(Json::as_i64).unwrap_or(0),
                    object
                        .get("name")
                        .and_then(Json::as_str)
                        .unwrap_or("unnamed")
                        .to_owned(),
                ))
            })
            .collect())
    }

    /// Returns `(frame id, function, file, line, column)` for the bounded stack page.
    fn stack(&mut self, thread: i64) -> Result<Vec<FrameRow>, String> {
        let mut arguments = JsonObject::new();
        arguments.insert("threadId", Json::number(thread.max(0) as u64));
        arguments.insert("startFrame", Json::number(0));
        arguments.insert("levels", Json::number(STACK_LIMIT as u64));
        let response = self.inspect_request("stackTrace", &Json::Object(arguments))?;
        let frames = response
            .object()
            .and_then(|body| body.get("stackFrames"))
            .and_then(Json::as_array)
            .map(|entries| entries.to_vec())
            .unwrap_or_default();
        Ok(frames
            .iter()
            .filter_map(|entry| {
                let object = entry.as_object()?;
                Some((
                    object.get("id").and_then(Json::as_i64).unwrap_or(0),
                    object
                        .get("name")
                        .and_then(Json::as_str)
                        .unwrap_or("anonymous")
                        .to_owned(),
                    object
                        .get("source")
                        .and_then(|source| source.as_object())
                        .and_then(|source| source.get("path"))
                        .and_then(Json::as_str)
                        .map(str::to_owned),
                    object.get("line").and_then(Json::as_u64).unwrap_or(0) as u32,
                    object.get("column").and_then(Json::as_u64).unwrap_or(0) as u32,
                ))
            })
            .collect())
    }

    /// The first frame's first scope and its bounded variable page.
    fn locals(&mut self, frame: i64) -> Result<LocalPage, String> {
        let mut arguments = JsonObject::new();
        arguments.insert("frameId", Json::number(frame.max(0) as u64));
        let response = self.inspect_request("scopes", &Json::Object(arguments))?;
        let scopes = response
            .object()
            .and_then(|body| body.get("scopes"))
            .and_then(Json::as_array)
            .map(|entries| entries.to_vec())
            .unwrap_or_default();
        let parsed: Vec<(String, i64, Option<usize>)> = scopes
            .iter()
            .filter_map(|entry| {
                let object = entry.as_object()?;
                Some((
                    object
                        .get("name")
                        .and_then(Json::as_str)
                        .unwrap_or("scope")
                        .to_owned(),
                    object
                        .get("variablesReference")
                        .and_then(Json::as_i64)
                        .unwrap_or(0),
                    object
                        .get("namedVariables")
                        .and_then(Json::as_u64)
                        .map(|count| count as usize),
                ))
            })
            .collect();
        // GDB orders `Registers` before `Locals` on some lines. The panel is about the program's
        // locals, so that scope is preferred by name rather than by position.
        let scope = parsed
            .iter()
            .find(|(name, _, _)| {
                name.eq_ignore_ascii_case("locals") || name.eq_ignore_ascii_case("local")
            })
            .or_else(|| parsed.first())
            .cloned();
        let Some((name, reference, count)) = scope else {
            return Ok(("no scopes reported".to_owned(), Vec::new()));
        };
        // GDB's DAP indexes its child list with the requested count, so asking for more than the
        // adapter said it holds raises instead of clamping.
        let want = count.unwrap_or(20).clamp(1, PAGE_LIMIT);
        let mut arguments = JsonObject::new();
        arguments.insert("variablesReference", Json::number(reference.max(0) as u64));
        arguments.insert("start", Json::number(0));
        arguments.insert("count", Json::number(want as u64));
        let response = self.inspect_request("variables", &Json::Object(arguments))?;
        let entries = response
            .object()
            .and_then(|body| body.get("variables"))
            .and_then(Json::as_array)
            .map(|entries| entries.to_vec())
            .unwrap_or_default();
        let variables: Vec<crate::app::debug::inspect::Variable> = entries
            .iter()
            .filter_map(|entry| {
                let object = entry.as_object()?;
                Some(crate::app::debug::inspect::Variable {
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
        Ok((
            format!("scope {name} ({} variable(s))", variables.len()),
            variables,
        ))
    }

    fn stop(&mut self) {
        if !self.live {
            return;
        }
        let mut arguments = JsonObject::new();
        arguments.insert("terminateDebuggee", Json::bool(true));
        let _ = self.client.request(
            "disconnect",
            &Json::Object(arguments),
            Duration::from_secs(3),
        );
        self.client.terminate(Duration::from_secs(2));
        self.live = false;
    }
}

/// The file name of a path, for a panel row that a narrow column can actually show.
fn short(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
}

/// What `prepare_debuggee` produced: the compiled source, the map back to design nodes, and the
/// breakpoint line the generated statement occupies.
struct PreparedDebuggee {
    source: PathBuf,
    program: PathBuf,
    source_file: String,
    map: SourceMap,
    line_offsets: Vec<u32>,
    break_line: u32,
}

/// Writes and compiles the debuggee generated from the design nodes with the host's own `rustc`.
fn prepare_debuggee(
    nodes: &[(NodeId, String)],
    break_node: NodeId,
) -> Result<PreparedDebuggee, String> {
    let directory = std::env::current_dir()
        .map_err(|error| format!("cannot read the working directory: {error}"))?
        .join("target/rustage-debug");
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
    let (text, spans, break_line) = generated_source(nodes, break_node);
    let source = directory.join("main.rs");
    // Writing the source unconditionally would update its mtime on every Start and make the
    // binary permanently "stale", so each session would pay for a compile it does not need.
    let unchanged = std::fs::read_to_string(&source)
        .map(|existing| existing == text)
        .unwrap_or(false);
    if !unchanged {
        std::fs::write(&source, &text)
            .map_err(|error| format!("cannot write {}: {error}", source.display()))?;
    }
    let program = directory.join("app");
    let stamp = directory.join("build.txt");
    let command = format!(
        "rustc -g -C opt-level=0 {} -o {}",
        source.display(),
        program.display()
    );
    let same_command = std::fs::read_to_string(&stamp)
        .map(|previous| previous.trim() == command)
        .unwrap_or(false);
    let newer_source = match (std::fs::metadata(&program), std::fs::metadata(&source)) {
        (Ok(program), Ok(source)) => match (program.modified(), source.modified()) {
            (Ok(program), Ok(source)) => program < source,
            _ => true,
        },
        _ => true,
    };
    if !same_command || newer_source {
        let status = std::process::Command::new("rustc")
            .args(["-g", "-C", "opt-level=0"])
            .arg(&source)
            .arg("-o")
            .arg(&program)
            .status()
            .map_err(|error| format!("cannot run rustc: {error}"))?;
        if !status.success() {
            return Err(format!("rustc exited with {status}"));
        }
        std::fs::write(&stamp, &command)
            .map_err(|error| format!("cannot write {}: {error}", stamp.display()))?;
    }
    let source_file = source.display().to_string();
    let mut map = SourceMap::new(GeneratorVersion::CURRENT, Revision::INITIAL);
    for (node, start, end) in spans {
        if let Some(span) = SourceSpan::new(&source_file, start, end) {
            map.insert(node, span);
        }
    }
    Ok(PreparedDebuggee {
        source,
        program,
        source_file,
        map,
        line_offsets: line_offsets(&text),
        break_line,
    })
}

/// Generates a Rust program from the design nodes and the spans that map it back.
///
/// Each node becomes one `#[inline(never)]` function that returns its title; `main` calls every
/// function so a breakpoint inside one is reached. The span covers the whole function, so a stop
/// on its `let` line maps to that node, and the node maps back to the same range.
fn generated_source(
    nodes: &[(NodeId, String)],
    break_node: NodeId,
) -> (String, Vec<(NodeId, u32, u32)>, u32) {
    let mut nodes = nodes.to_vec();
    if nodes.is_empty() {
        nodes.push((NodeId::from_raw(0), "window".to_owned()));
    }
    let mut text = String::new();
    let mut spans = Vec::new();
    let mut first_let = None;
    let mut break_line = None;
    for (node, name) in &nodes {
        let start = text.len() as u32;
        let start_line = line_of_offset(&text, start);
        let literal = format!("{name:?}");
        text.push_str(&format!(
            "#[inline(never)]\nfn node_{}() -> &'static str {{\n    let n = 41;\n    let title: &'static str = {literal};\n    let _ = n;\n    title\n}}\n\n",
            node.get()
        ));
        // The `title` line runs after `n` is initialized, so the stopped frame carries both the
        // demo local and the node's own title.
        let break_at = start_line + 5;
        first_let.get_or_insert(break_at);
        if *node == break_node {
            break_line = Some(break_at);
        }
        spans.push((*node, start, text.len() as u32));
    }
    text.push_str("fn main() {\n    let n = 41;\n    println!(\"value {}\", n + 1);\n");
    for (node, _) in &nodes {
        text.push_str(&format!("    println!(\"{{}}\", node_{}());\n", node.get()));
    }
    text.push_str("}\n");
    (text, spans, break_line.or(first_let).unwrap_or(1))
}

/// The 1-based line a byte offset falls on.
fn line_of_offset(text: &str, offset: u32) -> u32 {
    text.bytes()
        .take(offset as usize)
        .filter(|byte| *byte == b'\n')
        .count() as u32
        + 1
}

/// The byte offset of each 1-based line, for turning a DAP line into a map lookup.
fn line_offsets(text: &str) -> Vec<u32> {
    let mut offsets = vec![0];
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            offsets.push((index + 1) as u32);
        }
    }
    offsets
}

#[cfg(test)]
mod tests {
    use super::DebugState;
    use std::time::{Duration, Instant};

    #[test]
    fn the_generated_source_maps_the_break_line_back_to_its_node() {
        let nodes = vec![
            (
                crate::contracts::identity::NodeId::from_raw(0),
                "window".to_owned(),
            ),
            (
                crate::contracts::identity::NodeId::from_raw(1),
                "Demo".to_owned(),
            ),
            (
                crate::contracts::identity::NodeId::from_raw(2),
                "button".to_owned(),
            ),
        ];
        let (text, spans, break_line) =
            super::generated_source(&nodes, crate::contracts::identity::NodeId::from_raw(2));
        assert!(text.contains("let n = 41;"), "{text}");
        assert!(text.contains("fn node_2()"), "{text}");
        let offsets = super::line_offsets(&text);
        let break_offset = offsets[(break_line - 1) as usize];
        let (node, start, end) = spans
            .iter()
            .find(|(node, _, _)| node.get() == 2)
            .copied()
            .expect("node 2 is mapped");
        assert_eq!(node.get(), 2);
        assert!(start <= break_offset && break_offset < end);
        // The reverse direction: node 2's span names the lines its function occupies.
        let first_line = super::line_of_offset(&text, start);
        let last_line = super::line_of_offset(&text, end);
        assert!(first_line < break_line && break_line <= last_line);
    }

    /// Drives the worker without a window: a real `rustc`, adapter, breakpoint and locals.
    ///
    /// Ignored by default because it starts `rustc` and gdb (and so is not a unit-test cost);
    /// the GUI probe is the window-level evidence, this is the worker-level one.
    #[test]
    #[ignore = "starts rustc and gdb; run with --ignored"]
    fn a_real_session_reports_the_breakpoint_and_locals() {
        let mut state = DebugState::new();
        let nodes = vec![
            (
                crate::contracts::identity::NodeId::from_raw(0),
                "window".to_owned(),
            ),
            (
                crate::contracts::identity::NodeId::from_raw(1),
                "Demo".to_owned(),
            ),
            (
                crate::contracts::identity::NodeId::from_raw(2),
                "button".to_owned(),
            ),
        ];
        assert!(
            state.start(nodes, crate::contracts::identity::NodeId::from_raw(2)),
            "the worker starts"
        );
        let deadline = Instant::now() + Duration::from_secs(45);
        let mut last = None;
        while Instant::now() < deadline {
            if let Some(update) = state.poll() {
                let stopped = update.phase == "stopped";
                last = Some(update);
                if stopped {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let update = last.expect("the worker reports at least one update");
        assert_eq!(update.phase, "stopped", "{:?}", update.rows);
        assert!(
            update.rows.iter().any(|row| row.contains("bound at")),
            "{:?}",
            update.rows
        );
        assert!(
            update.rows.iter().any(|row| row.contains("-> node 2")),
            "{:?}",
            update.rows
        );
        assert!(
            update
                .rows
                .iter()
                .any(|row| row.contains("variable n: i32 = 41")),
            "{:?}",
            update.rows
        );
        assert!(state.send(super::Command::Stop));
    }
}
