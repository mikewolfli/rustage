// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The replaceable business-logic process: the other end of the typed channel (task T17.02, kept
//! honest by T17.06).
//!
//! The domain modules so far describe the two halves of a hot update — the shared [`Core`] that both
//! sides reduce with, and the [`Frame`] channel they speak. This module is what makes the second half
//! a *process* rather than a diagram: [`Worker`] owns one state and answers encoded requests, and
//! [`serve`] runs it over any buffered input/output, so a real child process and an in-memory test use
//! exactly the same code.
//!
//! Three rules keep it honest:
//!
//! * **The handshake comes first.** A request before the handshake is a protocol fault, not an
//!   implicitly opened session; the manifest answers with the generation the handshake named.
//! * **The worker checks its own generation.** A frame from the version being replaced is answered
//!   with `fault=...` on the worker's own generation, so the host gets a diagnosable refusal instead
//!   of an applied result. (The host checks too; a boundary only one side checks is not a boundary.)
//! * **A business refusal is not a protocol failure.** An event with no transition answers
//!   `error=...` and the process keeps serving; only a malformed or out-of-session frame ends it.
//! * **State can cross the boundary.** `snapshot` writes the state in the same line form `restore`
//!   reads, so a new process can continue from the old one's state. A malformed restore is refused
//!   before anything is assigned, leaving the old state in place.
//!
//! The payload is line-oriented text inside the hex frame, so a value with a newline would split a
//! record. Values are escaped and unescaped here rather than silently truncated.

use std::collections::BTreeMap;
use std::io::{BufRead, Write};

use super::core::{Core, Event, Refusal, State, Step, Transition, Value};
use super::ipc::{Fault, Frame, Kind, Manifest, Session};
use super::migration::{FieldType, Schema};

/// A business program: the name and version it reports, the transitions it reduces with, and the
/// state schema it accepts on `restore`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Program {
    /// The name the manifest reports.
    pub name: String,
    /// The version the manifest reports.
    pub version: String,
    /// The reduction itself.
    pub core: Core,
    /// The state schema this generation accepts, when it declares one.
    ///
    /// `None` keeps the historical behaviour (any syntactically valid field is accepted). When it
    /// is present, `restore` refuses an undeclared field or a value whose type does not match — the
    /// same two facts the migration preview classifies. This is the receiving side's own check: a
    /// generation that only trusts the sender's preview can be handed a state it cannot reduce.
    pub schema: Option<Schema>,
}

impl Program {
    /// A program that starts in `initial` and has no transitions yet.
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        initial: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            core: Core::new(initial),
            schema: None,
        }
    }

    /// The same program with one more transition.
    pub fn with(mut self, transition: Transition) -> Result<Self, Refusal> {
        self.core.add(transition)?;
        Ok(self)
    }

    /// The same program declaring the state schema it accepts on `restore`.
    pub fn with_schema(mut self, schema: Schema) -> Self {
        self.schema = Some(schema);
        self
    }

    /// Checks a restored state against the declared schema.
    ///
    /// Only fields that are present are checked. A schema field the snapshot omits stays absent, which
    /// matches [`migration::apply`](super::migration::apply): a `Kept` field the old snapshot did not
    /// hold is not invented here.
    pub fn accepts(&self, state: &State) -> Result<(), String> {
        let Some(schema) = &self.schema else {
            return Ok(());
        };
        for (name, value) in &state.fields {
            let Some(field) = schema.field(name) else {
                return Err(format!(
                    "the state field {name:?} is not declared by schema v{}",
                    schema.version
                ));
            };
            let matches = matches!(
                (field.kind, value),
                (FieldType::Flag, Value::Flag(_))
                    | (FieldType::Number, Value::Number(_))
                    | (FieldType::Text, Value::Text(_))
            );
            if !matches {
                return Err(format!(
                    "the state field {name:?} is {}, but schema v{} declares {}",
                    value.type_name(),
                    schema.version,
                    field.kind.name()
                ));
            }
        }
        Ok(())
    }

    /// The state the program starts in.
    pub fn initial(&self) -> &str {
        &self.core.initial
    }
}

/// What handling one line produced.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Handling {
    /// The encoded reply to write back.
    Reply(String),
    /// The encoded reply to write back, after which the process should stop.
    Finished(String),
}

/// One running business program: a state and the session it answers on.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Worker {
    program: Program,
    state: State,
    session: Option<Session>,
    finished: bool,
}

impl Worker {
    /// A worker that has not been handed a session yet.
    pub fn new(program: Program) -> Self {
        let state = program.core.start();
        Self {
            program,
            state,
            session: None,
            finished: false,
        }
    }

    /// The state the worker currently holds (a test and a driver both need to ask).
    #[cfg(test)]
    pub fn state(&self) -> &State {
        &self.state
    }

    /// Whether the worker was asked to stop.
    #[cfg(test)]
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// Handles one encoded frame.
    pub fn handle(&mut self, line: &str) -> Result<Handling, Fault> {
        let frame = Frame::decode(line)?;
        match frame.kind {
            Kind::Handshake => self.open(&frame),
            Kind::Request => {
                let Some(session) = self.session.clone() else {
                    return Err(Fault::Malformed {
                        reason: "a request arrived before the handshake".to_owned(),
                    });
                };
                if let Err(fault) = session.accept(&frame) {
                    let reply = format!("fault={}", escape(&fault.render()));
                    return Ok(Handling::Reply(self.reply(&session, reply)?));
                }
                self.request(&frame, &session)
            }
            other => Err(Fault::Malformed {
                reason: format!(
                    "a {} frame is not accepted by the logic process",
                    other.name()
                ),
            }),
        }
    }

    /// Opens the session and answers with the manifest.
    fn open(&mut self, frame: &Frame) -> Result<Handling, Fault> {
        if self.session.is_some() {
            return Err(Fault::Malformed {
                reason: "the session is already open".to_owned(),
            });
        }
        let manifest = Manifest::new(
            self.program.name.clone(),
            self.program.version.clone(),
            frame.generation,
        )
        .with("reduce")
        .with("state");
        let session = Session::open(
            frame.session.clone(),
            frame.protocol,
            manifest.clone(),
            frame.generation,
        )?;
        let payload = manifest.encode()?;
        let reply = Frame::new(
            Kind::Manifest,
            session.id.clone(),
            session.generation,
            payload.into_bytes(),
        );
        let text = session.send(&reply)?;
        self.session = Some(session);
        Ok(Handling::Reply(text))
    }

    /// Reduces one request and answers with the resulting state or a diagnosable refusal.
    fn request(&mut self, frame: &Frame, session: &Session) -> Result<Handling, Fault> {
        let text = String::from_utf8(frame.payload.clone()).map_err(|_| Fault::Malformed {
            reason: "the request payload is not UTF-8".to_owned(),
        })?;
        let mut lines = text.lines();
        let first = lines.next().unwrap_or_default();
        let reply = if first == "restore" {
            match restore(lines) {
                Ok(state) => match self.program.accepts(&state) {
                    Ok(()) => {
                        self.state = state;
                        describe(&self.state)
                    }
                    Err(reason) => format!("error={}", escape(&reason)),
                },
                Err(reason) => format!("error={}", escape(&reason)),
            }
        } else if lines.next().is_some() {
            format!("error={}", escape("a request carries exactly one line"))
        } else if first == "state" || first == "snapshot" {
            describe(&self.state)
        } else if first == "quit" {
            self.finished = true;
            "bye".to_owned()
        } else if let Some(event) = parse_event(first) {
            match self.program.core.reduce(&self.state, &event) {
                Ok(step) => {
                    self.state = step.state.clone();
                    describe_step(&step)
                }
                Err(refusal) => format!("error={}", escape(&refusal.render())),
            }
        } else {
            format!(
                "error={}",
                escape(&format!("{first:?} is not a business request"))
            )
        };
        let reply = if self.finished {
            Handling::Finished(self.reply(session, reply)?)
        } else {
            Handling::Reply(self.reply(session, reply)?)
        };
        Ok(reply)
    }

    /// Encodes one response on the open session.
    fn reply(&self, session: &Session, payload: String) -> Result<String, Fault> {
        let frame = Frame::new(
            Kind::Response,
            session.id.clone(),
            session.generation,
            payload.into_bytes(),
        );
        session.send(&frame)
    }
}

/// Why serving stopped.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ServeError {
    /// The input or output failed.
    Io(String),
    /// The peer broke the protocol.
    Fault(Fault),
}

impl ServeError {
    /// The reason in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Io(reason) => format!("the channel failed: {reason}"),
            Self::Fault(fault) => fault.render(),
        }
    }
}

impl std::fmt::Display for ServeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.render())
    }
}

/// Serves one program over `input`, writing each reply to `output`.
///
/// It stops at `quit`, at end-of-input, or at the first protocol fault; a business refusal is a reply
/// and does not stop the process (see the module docs).
pub fn serve<R: BufRead, W: Write>(
    mut worker: Worker,
    input: &mut R,
    output: &mut W,
) -> Result<(), ServeError> {
    for line in input.lines() {
        let line = line.map_err(|error| ServeError::Io(error.to_string()))?;
        if line.trim().is_empty() {
            continue;
        }
        match worker.handle(&line) {
            Ok(Handling::Reply(reply)) => {
                writeln!(output, "{reply}").map_err(|error| ServeError::Io(error.to_string()))?;
                output
                    .flush()
                    .map_err(|error| ServeError::Io(error.to_string()))?;
            }
            Ok(Handling::Finished(reply)) => {
                writeln!(output, "{reply}").map_err(|error| ServeError::Io(error.to_string()))?;
                output
                    .flush()
                    .map_err(|error| ServeError::Io(error.to_string()))?;
                return Ok(());
            }
            Err(fault) => return Err(ServeError::Fault(fault)),
        }
    }
    Ok(())
}

/// Reads the lines after `restore` into a state, refusing anything that does not round-trip.
///
/// The shape is the one [`describe`] writes: one `state=` line and zero or more
/// `field=<name>=<type>:<value>` lines. Nothing is assigned until every line parsed, so a malformed
/// restore leaves the worker's state exactly where it was.
fn restore<'a>(lines: impl Iterator<Item = &'a str>) -> Result<State, String> {
    let mut name = None;
    let mut fields = BTreeMap::new();
    for line in lines {
        if let Some(rest) = line.strip_prefix("state=") {
            if name.is_some() {
                return Err("a restore names the state more than once".to_owned());
            }
            name = Some(
                unescape(rest)
                    .ok_or_else(|| "the state name has an unfinished escape".to_owned())?,
            );
        } else if let Some(rest) = line.strip_prefix("field=") {
            let Some((field, typed)) = rest.split_once('=') else {
                return Err(format!(
                    "the restore field {line:?} is not field=<name>=<type>:<value>"
                ));
            };
            let field = unescape(field)
                .ok_or_else(|| "a field name has an unfinished escape".to_owned())?;
            let typed = unescape(typed)
                .ok_or_else(|| "a field value has an unfinished escape".to_owned())?;
            let value =
                Value::parse(&typed).ok_or_else(|| format!("{typed:?} is not <type>:<value>"))?;
            if fields.insert(field.clone(), value).is_some() {
                return Err(format!("the restore names field {field:?} more than once"));
            }
        } else if line.is_empty() {
            continue;
        } else {
            return Err(format!("the restore line {line:?} is not state= or field="));
        }
    }
    let name = name.ok_or_else(|| "a restore needs a state= line".to_owned())?;
    if name.is_empty() {
        return Err("a restore needs a non-empty state name".to_owned());
    }
    Ok(State { name, fields })
}

/// Reads `event <name>` or `event <name> <type>:<value>`.
fn parse_event(line: &str) -> Option<Event> {
    let rest = line.strip_prefix("event ")?;
    let (name, payload) = match rest.split_once(' ') {
        Some((name, value)) => (name, Value::parse(&unescape(value)?)?),
        None => (rest, Value::Flag(true)),
    };
    if name.is_empty() {
        return None;
    }
    Some(Event::new(name, payload))
}

/// The state as response lines.
fn describe(state: &State) -> String {
    let mut text = format!("state={}", escape(&state.name));
    for (field, value) in &state.fields {
        text.push_str(&format!(
            "\nfield={}={}:{}",
            escape(field),
            value.type_name(),
            escape(&value.render())
        ));
    }
    text
}

/// One step as response lines.
fn describe_step(step: &Step) -> String {
    let mut text = describe(&step.state);
    for action in &step.actions {
        text.push_str(&format!("\naction={}", escape(&action.render())));
    }
    if let Some(need) = &step.need {
        text.push_str(&format!("\nneed={}", escape(&need.render())));
    }
    for event in &step.emitted {
        text.push_str(&format!(
            "\nemitted={}={}",
            escape(&event.name),
            escape(&event.render())
        ));
    }
    text
}

/// Escapes the characters that would otherwise split a record.
///
/// The host's request builder uses the same function, so the two sides of the channel cannot drift
/// into different escape rules.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out
}

/// Reads what [`escape`] wrote, answering `None` for an unfinished escape.
pub fn unescape(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match characters.next()? {
            '\\' => out.push('\\'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{Handling, Program, ServeError, Worker, escape, parse_event, serve, unescape};
    use crate::app::logic::core::{Action, State, Transition, Value};
    use crate::app::logic::ipc::{Fault, Frame, Kind, Manifest, Session};
    use crate::app::logic::migration::{FieldType, Schema};
    use std::io::Cursor;

    fn program() -> Program {
        Program::new("orders", "0.3.0", "idle")
            .with(Transition::new("idle", "go", "running"))
            .expect("a transition")
    }

    fn handshake(generation: u64) -> String {
        Frame::new(Kind::Handshake, "logic-1", generation, Vec::new())
            .encode()
            .expect("an encoded handshake")
    }

    fn request(generation: u64, payload: &str) -> String {
        Frame::new(
            Kind::Request,
            "logic-1",
            generation,
            payload.as_bytes().to_vec(),
        )
        .encode()
        .expect("an encoded request")
    }

    fn answer(handling: Handling) -> String {
        match handling {
            Handling::Reply(text) | Handling::Finished(text) => text,
        }
    }

    fn payload(encoded: &str) -> String {
        let frame = Frame::decode(encoded).expect("a frame");
        String::from_utf8(frame.payload).expect("text")
    }

    #[test]
    fn a_handshake_answers_with_the_manifest_and_the_same_generation() {
        let mut worker = Worker::new(program());
        let reply = answer(worker.handle(&handshake(7)).expect("a reply"));
        let frame = Frame::decode(&reply).expect("a manifest frame");
        assert_eq!(frame.kind, Kind::Manifest);
        assert_eq!(frame.generation, 7);
        let manifest =
            Manifest::parse(&String::from_utf8(frame.payload).expect("text")).expect("a manifest");
        assert_eq!(manifest.name, "orders");
        assert_eq!(manifest.version, "0.3.0");
        assert_eq!(manifest.generation, 7);
        assert_eq!(manifest.capabilities, vec!["reduce", "state"]);
    }

    #[test]
    fn an_event_moves_the_state_and_the_next_request_sees_it() {
        let mut worker = Worker::new(program());
        worker.handle(&handshake(7)).expect("the handshake");
        let reply = answer(worker.handle(&request(7, "event go")).expect("a response"));
        assert!(payload(&reply).starts_with("state=running"));

        let reply = answer(worker.handle(&request(7, "state")).expect("a response"));
        assert!(payload(&reply).starts_with("state=running"));
        assert_eq!(worker.state(), &State::new("running"));
    }

    #[test]
    fn an_event_with_no_transition_is_a_reply_not_a_dead_process() {
        let mut worker = Worker::new(program());
        worker.handle(&handshake(7)).expect("the handshake");
        let reply = answer(worker.handle(&request(7, "event stop")).expect("a reply"));
        let text = payload(&reply);
        assert!(text.starts_with("error="), "{text}");
        assert!(text.contains("no transition"));
        // Still serving.
        assert!(worker.handle(&request(7, "state")).is_ok());
    }

    #[test]
    fn a_frame_from_the_replaced_generation_is_refused_on_the_new_one() {
        let mut worker = Worker::new(program());
        worker.handle(&handshake(8)).expect("the handshake");
        let reply = answer(
            worker
                .handle(&request(7, "event go"))
                .expect("a refusal reply"),
        );
        let frame = Frame::decode(&reply).expect("a frame");
        assert_eq!(frame.generation, 8);
        let text = String::from_utf8(frame.payload).expect("text");
        assert!(text.starts_with("fault="), "{text}");
        assert!(text.contains("generation"), "{text}");
        // The wrong-generation event did not move the state.
        assert_eq!(worker.state().name, "idle");
    }

    #[test]
    fn a_request_before_the_handshake_is_a_protocol_fault() {
        let mut worker = Worker::new(program());
        let fault = worker
            .handle(&request(7, "state"))
            .expect_err("a protocol fault");
        assert!(matches!(fault, Fault::Malformed { .. }));
        assert!(fault.render().contains("before the handshake"));
    }

    #[test]
    fn a_snapshot_restores_into_a_fresh_worker_with_its_fields() {
        let mut transition = Transition::new("idle", "go", "running");
        transition.actions.push(Action::Set {
            field: "count".to_owned(),
            value: Value::Number(3),
        });
        let program = Program::new("orders", "0.3.0", "idle")
            .with(transition)
            .expect("a transition");

        let mut old = Worker::new(program.clone());
        old.handle(&handshake(1)).expect("the handshake");
        old.handle(&request(1, "event go")).expect("the event");
        let snapshot = payload(&answer(
            old.handle(&request(1, "snapshot")).expect("a snapshot"),
        ));
        assert!(snapshot.starts_with("state=running"));
        assert!(snapshot.contains("field=count=number:3"), "{snapshot}");

        let mut new = Worker::new(Program::new("orders", "0.4.0", "idle"));
        new.handle(&handshake(2)).expect("the handshake");
        let restored = payload(&answer(
            new.handle(&request(2, &format!("restore\n{snapshot}")))
                .expect("a restore"),
        ));
        assert_eq!(restored, snapshot);
        assert_eq!(new.state().name, "running");
        assert_eq!(new.state().fields.get("count"), Some(&Value::Number(3)));
    }

    #[test]
    fn a_declared_schema_accepts_its_fields_and_refuses_an_undeclared_one() {
        let program = program().with_schema(
            Schema::new(2)
                .with("count", FieldType::Number, 1)
                .with("note", FieldType::Text, 2),
        );
        let mut worker = Worker::new(program);
        worker.handle(&handshake(2)).expect("the handshake");

        let accepted = payload(&answer(
            worker
                .handle(&request(2, "restore\nstate=running\nfield=count=number:3"))
                .expect("a restore"),
        ));
        assert!(accepted.starts_with("state=running"), "{accepted}");
        assert_eq!(worker.state().fields.get("count"), Some(&Value::Number(3)));

        let refused = payload(&answer(
            worker
                .handle(&request(2, "restore\nstate=running\nfield=other=text:x"))
                .expect("a refusal reply"),
        ));
        assert!(refused.starts_with("error="), "{refused}");
        assert!(refused.contains("not declared by schema v2"), "{refused}");
        // The refused restore changed nothing.
        assert_eq!(worker.state().fields.get("count"), Some(&Value::Number(3)));
    }

    #[test]
    fn a_declared_schema_refuses_a_value_of_the_wrong_type() {
        let program = program().with_schema(Schema::new(2).with("count", FieldType::Number, 1));
        let mut worker = Worker::new(program);
        worker.handle(&handshake(2)).expect("the handshake");
        let refused = payload(&answer(
            worker
                .handle(&request(2, "restore\nstate=running\nfield=count=text:3"))
                .expect("a refusal reply"),
        ));
        assert!(refused.starts_with("error="), "{refused}");
        assert!(
            refused.contains("is text, but schema v2 declares number"),
            "{refused}"
        );
        assert_eq!(worker.state().name, "idle");
    }

    #[test]
    fn a_malformed_restore_leaves_the_state_untouched() {
        let mut worker = Worker::new(program());
        worker.handle(&handshake(1)).expect("the handshake");
        let reply = answer(
            worker
                .handle(&request(1, "restore\nstate=running\nfield=count=wat"))
                .expect("a reply"),
        );
        let text = payload(&reply);
        assert!(text.starts_with("error="), "{text}");
        assert!(text.contains("not <type>:<value>"), "{text}");
        assert_eq!(worker.state().name, "idle");

        let reply = answer(
            worker
                .handle(&request(
                    1,
                    "restore\nstate=a\nfield=x=number:1\nfield=x=number:2",
                ))
                .expect("a reply"),
        );
        assert!(payload(&reply).contains("more than once"));
        assert_eq!(worker.state().name, "idle");
    }

    #[test]
    fn quit_answers_and_asks_the_process_to_stop() {
        let mut worker = Worker::new(program());
        worker.handle(&handshake(1)).expect("the handshake");
        let handling = worker.handle(&request(1, "quit")).expect("a reply");
        assert!(matches!(handling, Handling::Finished(_)));
        assert!(worker.finished());
    }

    #[test]
    fn serving_reads_frames_writes_frames_and_stops_at_quit() {
        let input = [
            handshake(3),
            request(3, "event go"),
            request(3, "quit"),
            request(3, "state"),
        ]
        .join("\n");
        let mut reader = Cursor::new(input);
        let mut output = Vec::new();
        serve(Worker::new(program()), &mut reader, &mut output).expect("a served program");
        let lines: Vec<String> = String::from_utf8(output)
            .expect("text")
            .lines()
            .map(str::to_owned)
            .collect();
        assert_eq!(lines.len(), 3, "the request after quit was not read");
        assert_eq!(
            Frame::decode(&lines[0]).expect("a frame").kind,
            Kind::Manifest
        );
        assert!(payload(&lines[1]).starts_with("state=running"));
        assert_eq!(payload(&lines[2]), "bye");
    }

    #[test]
    fn a_malformed_frame_stops_serving_with_the_reason() {
        let mut reader = Cursor::new(String::from("not a frame"));
        let mut output = Vec::new();
        let error =
            serve(Worker::new(program()), &mut reader, &mut output).expect_err("a protocol fault");
        assert!(matches!(error, ServeError::Fault(Fault::Malformed { .. })));
        assert!(error.render().contains("malformed"));
    }

    #[test]
    fn a_newline_in_a_value_survives_the_record_format() {
        let value = "two\nlines\\and a slash";
        assert_eq!(unescape(&escape(value)).as_deref(), Some(value));
        let event = parse_event("event note text:two\nlines").expect("an event");
        assert_eq!(event.payload, Value::Text("two\nlines".to_owned()));
    }

    #[test]
    fn the_manifest_text_reports_its_generation() {
        let manifest = Manifest::new("orders", "0.3.0", 12);
        let text = manifest.encode().expect("text");
        assert_eq!(Manifest::parse(&text).expect("a manifest").generation, 12);
        assert!(Manifest::parse("name=a\nversion=b\nname=c\ngeneration=1").is_err());
        assert!(Manifest::parse("name=a\nversion=b\nunknown=1\ngeneration=1").is_err());
    }

    #[test]
    fn the_session_shape_is_the_one_the_host_opens() {
        // A driver parses the manifest and opens the same session; this is the join between the two
        // halves of the boundary, exercised without a process.
        let mut worker = Worker::new(program());
        let reply = answer(worker.handle(&handshake(5)).expect("a reply"));
        let frame = Frame::decode(&reply).expect("a frame");
        let manifest =
            Manifest::parse(&String::from_utf8(frame.payload).expect("text")).expect("a manifest");
        let session = Session::open("logic-1", frame.protocol, manifest, 5).expect("a session");
        let reply = answer(worker.handle(&request(5, "event go")).expect("a reply"));
        let response = session.receive(&reply).expect("a response in session");
        assert!(
            String::from_utf8(response.payload)
                .expect("text")
                .starts_with("state=running")
        );
    }
}
