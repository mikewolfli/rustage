// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The Debug Adapter Protocol wire: framing, messages and a bounded client (task T19.01's transport).
//!
//! DAP is JSON over a byte stream with a `Content-Length` header, which is close enough to HTTP to
//! be easy to get subtly wrong: a reader that trusts the header can be made to allocate an arbitrary
//! amount, and a client that reads events and responses in one loop can hang when an event arrives
//! while it is waiting for a response. The three properties here are the answers:
//!
//! * **A frame has a bound.** [`MAX_FRAME`] is checked before the body is read; a longer frame is a
//!   protocol error, not a reason to grow memory.
//! * **Reading happens on its own thread.** [`Client`] owns a reader thread that turns bytes into
//!   [`Message`]s, so a request can wait for *its* response while events are queued for the next
//!   `wait`. Every wait has a deadline; a silent adapter becomes [`ProtocolError::Timeout`].
//! * **A failed response is not an exception.** DAP answers refusals with `success: false`; the
//!   [`Response`] keeps that field and its message so the caller can report the adapter's own words.
//!
//! Reverse requests are kept rather than dropped: a `runInTerminal` this build cannot answer is
//! visible in the queue instead of making the adapter wait forever.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use crate::project::record::json::{Json, JsonObject, parse};

/// The largest frame body this build accepts.
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

/// Why the protocol layer could not answer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ProtocolError {
    /// The permission service has not allowed this session to start a process.
    Refused(String),
    /// The adapter could not be started.
    Spawn(String),
    /// A read or write failed.
    Io(String),
    /// The adapter did not answer before the deadline.
    Timeout {
        /// What was being waited for.
        waiting_for: String,
        /// How long.
        milliseconds: u64,
    },
    /// The adapter went away.
    Closed,
    /// The bytes were not a DAP frame this build can read.
    Malformed(String),
}

impl ProtocolError {
    /// The error in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Refused(reason) => format!("the debug session was refused: {reason}"),
            Self::Spawn(reason) => format!("the debug adapter could not be started: {reason}"),
            Self::Io(reason) => format!("the debug adapter connection failed: {reason}"),
            Self::Timeout {
                waiting_for,
                milliseconds,
            } => format!("timed out after {milliseconds} ms waiting for {waiting_for}"),
            Self::Closed => "the debug adapter closed the connection".to_owned(),
            Self::Malformed(reason) => {
                format!("the debug adapter sent a frame this build cannot read: {reason}")
            }
        }
    }
}

/// One response to a request.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Response {
    /// The sequence number of the request it answers.
    pub request_seq: i64,
    /// The command it answers.
    pub command: String,
    /// Whether the adapter accepted it.
    pub success: bool,
    /// The adapter's message when it did not.
    pub message: Option<String>,
    /// The body, or `null`.
    pub body: Json,
}

impl Response {
    /// The body as an object, when it is one.
    pub fn object(&self) -> Option<&JsonObject> {
        self.body.as_object()
    }
}

/// One event from the adapter.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Event {
    /// The event name.
    pub event: String,
    /// Its body, or `null`.
    pub body: Json,
}

impl Event {
    /// The body as an object, when it is one.
    pub fn object(&self) -> Option<&JsonObject> {
        self.body.as_object()
    }
}

/// One message on the wire.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Message {
    /// An answer to a request this client sent.
    Response(Response),
    /// An event.
    Event(Event),
    /// A request the adapter sent to this client.
    Request {
        /// Its sequence number.
        seq: i64,
        /// The command.
        command: String,
        /// Its arguments.
        arguments: Json,
    },
}

/// Reads one frame, or `None` at end of input.
pub fn read_frame<R: BufRead>(reader: &mut R) -> Result<Option<String>, ProtocolError> {
    let mut length: Option<usize> = None;
    let mut saw_header = false;
    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| ProtocolError::Io(error.to_string()))?;
        if read == 0 {
            return if saw_header {
                Err(ProtocolError::Malformed(
                    "the connection ended inside a header".to_owned(),
                ))
            } else {
                Ok(None)
            };
        }
        saw_header = true;
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(ProtocolError::Malformed(format!(
                "{line:?} is not a header"
            )));
        };
        if name.eq_ignore_ascii_case("Content-Length") {
            let parsed = value
                .trim()
                .parse::<usize>()
                .map_err(|_| ProtocolError::Malformed(format!("{value:?} is not a length")))?;
            length = Some(parsed);
        }
    }
    let Some(length) = length else {
        return Err(ProtocolError::Malformed(
            "the frame has no Content-Length".to_owned(),
        ));
    };
    if length > MAX_FRAME {
        return Err(ProtocolError::Malformed(format!(
            "{length} bytes exceeds the {MAX_FRAME} byte frame limit"
        )));
    }
    let mut body = vec![0_u8; length];
    reader
        .read_exact(&mut body)
        .map_err(|error| ProtocolError::Io(error.to_string()))?;
    String::from_utf8(body)
        .map(Some)
        .map_err(|error| ProtocolError::Malformed(error.to_string()))
}

/// Writes one frame.
pub fn write_frame<W: Write>(writer: &mut W, body: &str) -> Result<(), ProtocolError> {
    write!(writer, "Content-Length: {}\r\n\r\n{body}", body.len())
        .and_then(|()| writer.flush())
        .map_err(|error| ProtocolError::Io(error.to_string()))
}

/// Parses one frame body into a message.
pub fn parse_message(text: &str) -> Result<Message, ProtocolError> {
    let value = parse(text).map_err(|error| ProtocolError::Malformed(error.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| ProtocolError::Malformed("the message is not an object".to_owned()))?;
    let kind = object
        .get("type")
        .and_then(Json::as_str)
        .ok_or_else(|| ProtocolError::Malformed("the message has no type".to_owned()))?;
    match kind {
        "response" => {
            let request_seq = object
                .get("request_seq")
                .and_then(Json::as_i64)
                .ok_or_else(|| {
                    ProtocolError::Malformed("a response has no request_seq".to_owned())
                })?;
            let command = object
                .get("command")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned();
            let success = object
                .get("success")
                .map(|value| matches!(value, Json::Bool(true)))
                .unwrap_or(false);
            let message = object
                .get("message")
                .and_then(Json::as_str)
                .map(str::to_owned);
            Ok(Message::Response(Response {
                request_seq,
                command,
                success,
                message,
                body: object.get("body").cloned().unwrap_or(Json::Null),
            }))
        }
        "event" => {
            let event = object
                .get("event")
                .and_then(Json::as_str)
                .ok_or_else(|| ProtocolError::Malformed("an event has no name".to_owned()))?
                .to_owned();
            Ok(Message::Event(Event {
                event,
                body: object.get("body").cloned().unwrap_or(Json::Null),
            }))
        }
        "request" => {
            let seq = object.get("seq").and_then(Json::as_i64).unwrap_or(0);
            let command = object
                .get("command")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned();
            Ok(Message::Request {
                seq,
                command,
                arguments: object.get("arguments").cloned().unwrap_or(Json::Null),
            })
        }
        other => Err(ProtocolError::Malformed(format!(
            "{other:?} is not a DAP message type"
        ))),
    }
}

/// Builds a request frame body.
pub fn request_body(seq: i64, command: &str, arguments: &Json) -> String {
    let mut object = JsonObject::new();
    object.insert("seq", Json::number(seq as u64));
    object.insert("type", Json::string("request"));
    object.insert("command", Json::string(command));
    if *arguments != Json::Null {
        object.insert("arguments", arguments.clone());
    }
    Json::Object(object).render_compact()
}

/// A client for one adapter process.
pub struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    messages: Receiver<Result<Message, ProtocolError>>,
    queued: VecDeque<Message>,
    next_seq: i64,
    exited: bool,
}

impl Client {
    /// Starts the adapter and its reader thread.
    ///
    /// `permitted` is the permission decision, passed explicitly rather than read from the
    /// environment: starting a debug adapter is starting a process, and the same rule as the
    /// terminal and the build applies.
    pub fn spawn(
        program: &Path,
        arguments: &[String],
        directory: Option<&Path>,
        environment: &[(String, String)],
        permitted: bool,
    ) -> Result<Self, ProtocolError> {
        if !permitted {
            return Err(ProtocolError::Refused(
                "starting a debug adapter needs an authorization decision".to_owned(),
            ));
        }
        let mut command = Command::new(program);
        command
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(directory) = directory {
            command.current_dir(directory);
        }
        for (name, value) in environment {
            command.env(name, value);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // A group of its own, so terminating the adapter cannot reach a process this session
            // did not start (the same rule as `host::subprocess`).
            command.process_group(0);
        }
        let mut child = command
            .spawn()
            .map_err(|error| ProtocolError::Spawn(format!("{}: {error}", program.display())))?;
        let stdin = child.stdin.take();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ProtocolError::Spawn("the adapter has no stdout".to_owned()))?;
        let stderr = child.stderr.take();
        if let Some(stderr) = stderr {
            // The adapter's diagnostics must not fill a pipe and block it. They are drained to a
            // bounded buffer and are only read when a start fails; a chatty adapter cannot grow
            // the host's memory.
            thread::spawn(move || {
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap_or(0) > 0 {
                    line.clear();
                }
            });
        }
        let (sender, messages) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                match read_frame(&mut reader) {
                    Ok(Some(text)) => match parse_message(&text) {
                        Ok(message) => {
                            if sender.send(Ok(message)).is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            let _ = sender.send(Err(error));
                            break;
                        }
                    },
                    Ok(None) => {
                        let _ = sender.send(Err(ProtocolError::Closed));
                        break;
                    }
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        break;
                    }
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            messages,
            queued: VecDeque::new(),
            next_seq: 1,
            exited: false,
        })
    }

    /// The adapter's process id.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    fn send(&mut self, command: &str, arguments: &Json) -> Result<i64, ProtocolError> {
        let seq = self.next_seq;
        self.next_seq += 1;
        let Some(stdin) = self.stdin.as_mut() else {
            return Err(ProtocolError::Closed);
        };
        write_frame(stdin, &request_body(seq, command, arguments))?;
        Ok(seq)
    }

    /// Sends a request without waiting for its answer.
    ///
    /// A launch is the reason this exists: `lldb-dap` sends `initialized` while handling `launch` and
    /// only answers `launch` after `configurationDone`, so a client that waits for the response
    /// first deadlocks against the event it needs to send that request.
    pub fn send_request(&mut self, command: &str, arguments: &Json) -> Result<i64, ProtocolError> {
        self.send(command, arguments)
    }

    /// Waits for the response to a request that was already sent, queueing events that arrive first.
    pub fn await_response(
        &mut self,
        seq: i64,
        command: &str,
        timeout: Duration,
    ) -> Result<Response, ProtocolError> {
        // A previous `event` may have queued the response already.
        if let Some(position) = self.queued.iter().position(
            |message| matches!(message, Message::Response(response) if response.request_seq == seq),
        ) && let Some(Message::Response(response)) = self.queued.remove(position)
        {
            return Ok(response);
        }
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::ZERO);
            if remaining.is_zero() {
                return Err(ProtocolError::Timeout {
                    waiting_for: format!("the {command} response"),
                    milliseconds: timeout.as_millis() as u64,
                });
            }
            match self.messages.recv_timeout(remaining) {
                Ok(Ok(Message::Response(response))) if response.request_seq == seq => {
                    return Ok(response);
                }
                Ok(Ok(other)) => self.queued.push_back(other),
                Ok(Err(error)) => return Err(error),
                Err(RecvTimeoutError::Timeout) => {
                    return Err(ProtocolError::Timeout {
                        waiting_for: format!("the {command} response"),
                        milliseconds: timeout.as_millis() as u64,
                    });
                }
                Err(RecvTimeoutError::Disconnected) => return Err(ProtocolError::Closed),
            }
        }
    }

    /// Sends a request and waits for its own response, queueing events that arrive first.
    pub fn request(
        &mut self,
        command: &str,
        arguments: &Json,
        timeout: Duration,
    ) -> Result<Response, ProtocolError> {
        let seq = self.send(command, arguments)?;
        self.await_response(seq, command, timeout)
    }

    /// Waits for the next event, skipping queued responses.
    pub fn event(&mut self, timeout: Duration) -> Result<Event, ProtocolError> {
        if let Some(event) = self.take_queued_event() {
            return Ok(event);
        }
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::ZERO);
            if remaining.is_zero() {
                return Err(ProtocolError::Timeout {
                    waiting_for: "the next event".to_owned(),
                    milliseconds: timeout.as_millis() as u64,
                });
            }
            match self.messages.recv_timeout(remaining) {
                Ok(Ok(Message::Event(event))) => return Ok(event),
                Ok(Ok(other)) => self.queued.push_back(other),
                Ok(Err(ProtocolError::Closed)) => return Err(ProtocolError::Closed),
                Ok(Err(error)) => return Err(error),
                Err(RecvTimeoutError::Timeout) => {
                    return Err(ProtocolError::Timeout {
                        waiting_for: "the next event".to_owned(),
                        milliseconds: timeout.as_millis() as u64,
                    });
                }
                Err(RecvTimeoutError::Disconnected) => return Err(ProtocolError::Closed),
            }
        }
    }

    /// Whether an event is already waiting, without blocking.
    pub fn has_queued_event(&self) -> bool {
        self.queued
            .iter()
            .any(|message| matches!(message, Message::Event(_)))
    }

    fn take_queued_event(&mut self) -> Option<Event> {
        let position = self
            .queued
            .iter()
            .position(|message| matches!(message, Message::Event(_)))?;
        match self.queued.remove(position) {
            Some(Message::Event(event)) => Some(event),
            _ => None,
        }
    }

    /// One queued reverse request, when the adapter sent one.
    pub fn reverse_request(&mut self) -> Option<(i64, String)> {
        let position = self
            .queued
            .iter()
            .position(|message| matches!(message, Message::Request { .. }))?;
        match self.queued.remove(position) {
            Some(Message::Request { seq, command, .. }) => Some((seq, command)),
            _ => None,
        }
    }

    /// Terminates the adapter process and reaps it.
    ///
    /// Terminating the adapter is not terminating the debuggee: an attached process is owned by
    /// whoever started it, and a launched process is told to end by the `disconnect` request the
    /// session sends first. This only guarantees that *this* process does not survive the session.
    pub fn terminate(&mut self, grace: Duration) -> bool {
        self.stdin.take();
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => {
                    self.exited = true;
                    return true;
                }
                Ok(None) => thread::sleep(Duration::from_millis(10)),
                Err(_) => break,
            }
        }
        let killed = self.child.kill().is_ok();
        let reaped = self.child.wait().is_ok();
        self.exited = true;
        killed || reaped
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if !self.exited {
            self.stdin.take();
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_FRAME, Message, ProtocolError, parse_message, read_frame, request_body, write_frame,
    };
    use crate::project::record::json::{Json, JsonObject};
    use std::io::Cursor;

    #[test]
    fn a_frame_round_trips_through_the_wire_format() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, "{\"type\":\"event\",\"event\":\"initialized\"}").expect("a frame");
        let text = String::from_utf8(bytes.clone()).expect("utf-8");
        assert!(text.starts_with("Content-Length: "));
        assert!(text.contains("\r\n\r\n"));
        let mut cursor = Cursor::new(bytes);
        let body = read_frame(&mut cursor).expect("a read").expect("a body");
        assert_eq!(body, "{\"type\":\"event\",\"event\":\"initialized\"}");
        assert!(read_frame(&mut cursor).expect("end of input").is_none());
    }

    #[test]
    fn an_oversized_frame_is_refused_before_it_is_read() {
        let text = format!("Content-Length: {}\r\n\r\n", MAX_FRAME + 1);
        let mut cursor = Cursor::new(text.into_bytes());
        let error = read_frame(&mut cursor).expect_err("an oversized frame");
        assert!(matches!(error, ProtocolError::Malformed(_)));
        assert!(error.render().contains("frame limit"));
    }

    #[test]
    fn a_frame_without_a_length_is_refused() {
        let mut cursor = Cursor::new(b"X-Other: 1\r\n\r\n".to_vec());
        let error = read_frame(&mut cursor).expect_err("no length");
        assert!(error.render().contains("no Content-Length"));
    }

    #[test]
    fn a_response_carries_success_and_failure_alike() {
        let failed = parse_message(
            "{\"type\":\"response\",\"request_seq\":7,\"command\":\"attach\",\"success\":false,\
             \"message\":\"attach is not permitted\"}",
        )
        .expect("a message");
        match failed {
            Message::Response(response) => {
                assert_eq!(response.request_seq, 7);
                assert!(!response.success);
                assert_eq!(response.message.as_deref(), Some("attach is not permitted"));
            }
            other => panic!("expected a response, got {other:?}"),
        }
    }

    #[test]
    fn an_event_keeps_its_body_and_a_reverse_request_is_kept() {
        let event = parse_message(
            "{\"type\":\"event\",\"event\":\"stopped\",\"body\":{\"reason\":\"breakpoint\"}}",
        )
        .expect("a message");
        match event {
            Message::Event(event) => {
                assert_eq!(event.event, "stopped");
                assert_eq!(
                    event
                        .object()
                        .and_then(|body| body.get("reason"))
                        .and_then(Json::as_str),
                    Some("breakpoint")
                );
            }
            other => panic!("expected an event, got {other:?}"),
        }
        let reverse = parse_message(
            "{\"type\":\"request\",\"seq\":9,\"command\":\"runInTerminal\",\"arguments\":{}}",
        )
        .expect("a message");
        assert!(matches!(reverse, Message::Request { seq: 9, .. }));
    }

    #[test]
    fn a_request_body_has_the_fields_dap_expects() {
        let mut arguments = JsonObject::new();
        arguments.insert("program", Json::string("/tmp/example"));
        let body = request_body(4, "launch", &Json::Object(arguments));
        let value = crate::project::record::json::parse(&body).expect("json");
        let object = value.as_object().expect("an object");
        assert_eq!(object.get("type").and_then(Json::as_str), Some("request"));
        assert_eq!(object.get("seq").and_then(Json::as_i64), Some(4));
        assert_eq!(object.get("command").and_then(Json::as_str), Some("launch"));
    }
}
