// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A real Language Server Protocol client over a child process's stdio (task T07.01).
//!
//! [`app::language::session`](crate::app::language::session) owns the *model*: one session per
//! project, the states and the capability set. This module owns the transport the model was missing:
//! a process, the `Content-Length` framing, the `initialize`/`initialized` handshake, requests with
//! ids, and a bounded shutdown.
//!
//! Deliberately small. It implements the part every language server speaks and the designer needs
//! before any completion can be real: spawn, negotiate, ask, stop. It does **not** interpret
//! `textDocument/*` results; those stay in the typed model modules until a front end consumes them.
//! That boundary is what keeps this file from becoming a second language-service implementation.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::project::record::json::{Json, JsonObject, parse};

/// How long a server may take to answer one request unless the caller says otherwise.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// One JSON-RPC message, reduced to what a client needs to match it.
#[derive(Clone, Debug)]
pub struct Message {
    /// The request id, when the message is a response.
    pub id: Option<i64>,
    /// The method, when the message is a request or notification.
    #[allow(dead_code)]
    pub method: Option<String>,
    /// The message body.
    pub body: Json,
}

impl Message {
    /// Whether this message answers `id`.
    fn answers(&self, id: i64) -> bool {
        self.id == Some(id)
    }
}

/// A running language server.
pub struct Client {
    program: String,
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Message>,
    stderr: Arc<Mutex<String>>,
    reader: Option<JoinHandle<()>>,
    next_id: i64,
}

impl Client {
    /// Starts `program` with `args` and connects to its stdio.
    pub fn start(program: &str, args: &[String]) -> Result<Self, String> {
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| format!("{program:?} could not be started: {error}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| format!("{program:?} has no standard input"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| format!("{program:?} has no standard output"))?;
        let stderr_pipe = child
            .stderr
            .take()
            .ok_or_else(|| format!("{program:?} has no standard error"))?;

        let (sender, messages) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Ok(message) = read_message(&mut reader) {
                if sender.send(message).is_err() {
                    break;
                }
            }
        });

        let stderr = Arc::new(Mutex::new(String::new()));
        let capture = Arc::clone(&stderr);
        thread::spawn(move || {
            let mut reader = BufReader::new(stderr_pipe);
            let mut line = String::new();
            while let Ok(read) = reader.read_line(&mut line) {
                if read == 0 {
                    break;
                }
                let mut text = capture.lock().expect("the language-server stderr lock");
                text.push_str(&line);
                // Keep the diagnostic bounded; a server that logs forever must not grow the host.
                if text.len() > 64 * 1024 {
                    let excess = text.len() - 64 * 1024;
                    text.drain(..excess);
                }
                line.clear();
            }
        });

        Ok(Self {
            program: program.to_owned(),
            child,
            stdin,
            messages,
            stderr,
            reader: Some(reader),
            next_id: 1,
        })
    }

    /// The child's pid.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// What the server wrote to stderr, for a failure a caller has to explain.
    pub fn stderr(&self) -> String {
        self.stderr
            .lock()
            .expect("the language-server stderr lock")
            .clone()
    }

    /// Sends `initialize`, reads the result, and sends `initialized`.
    ///
    /// `root` is the project directory; it is sent both as `rootUri` and as the single workspace
    /// folder, because a server that only understands one of the two still has an answer.
    pub fn initialize(&mut self, root: &Path, timeout: Duration) -> Result<Json, String> {
        let mut params = JsonObject::new();
        params.insert("processId", Json::Null);
        params.insert("clientInfo", info("rustage", env!("CARGO_PKG_VERSION")));
        params.insert("rootUri", Json::string(file_uri(root)));
        params.insert("capabilities", Json::Object(JsonObject::new()));
        let mut folder = JsonObject::new();
        folder.insert("uri", Json::string(file_uri(root)));
        folder.insert(
            "name",
            Json::string(
                root.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| root.display().to_string()),
            ),
        );
        params.insert("workspaceFolders", Json::Array(vec![Json::Object(folder)]));

        let result = self.request("initialize", Json::Object(params), timeout)?;
        self.notify("initialized", Json::Object(JsonObject::new()))?;
        Ok(result)
    }

    /// Sends one request and waits for the response with the same id.
    pub fn request(
        &mut self,
        method: &str,
        params: Json,
        timeout: Duration,
    ) -> Result<Json, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(request_message(id, method, params))?;
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!("{method} did not answer within {timeout:?}"));
            }
            match self.messages.recv_timeout(remaining) {
                Ok(message) if message.answers(id) => return response_body(message.body),
                Ok(_) => continue,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!("{method} did not answer within {timeout:?}"));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let stderr = self.stderr();
                    return Err(format!(
                        "the language server exited before answering {method}: {}",
                        stderr.trim()
                    ));
                }
            }
        }
    }

    /// Sends a notification.
    pub fn notify(&mut self, method: &str, params: Json) -> Result<(), String> {
        let mut message = JsonObject::new();
        message.insert("jsonrpc", Json::string("2.0"));
        message.insert("method", Json::string(method));
        message.insert("params", params);
        self.send(Json::Object(message))
    }

    /// Sends `shutdown`, then `exit`, then waits briefly and kills if the server remains.
    pub fn shutdown(&mut self) {
        let _ = self.request("shutdown", Json::Null, Duration::from_secs(2));
        let _ = self.notify("exit", Json::Null);
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }

    /// The name of the program, for a diagnostic.
    pub fn program(&self) -> &str {
        &self.program
    }

    fn send(&mut self, message: Json) -> Result<(), String> {
        let body = message.render_compact();
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.stdin
            .write_all(header.as_bytes())
            .and_then(|()| self.stdin.write_all(body.as_bytes()))
            .and_then(|()| self.stdin.flush())
            .map_err(|error| format!("the language server's input closed: {error}"))
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn info(name: &str, version: &str) -> Json {
    let mut object = JsonObject::new();
    object.insert("name", Json::string(name));
    object.insert("version", Json::string(version));
    Json::Object(object)
}

fn request_message(id: i64, method: &str, params: Json) -> Json {
    let mut message = JsonObject::new();
    message.insert("jsonrpc", Json::string("2.0"));
    message.insert("id", Json::number(id as u64));
    message.insert("method", Json::string(method));
    message.insert("params", params);
    Json::Object(message)
}

/// The `result` of a response, or the `message` of its `error`.
fn response_body(body: Json) -> Result<Json, String> {
    let Some(object) = body.as_object() else {
        return Err("the language server answered with a non-object".to_owned());
    };
    if let Some(error) = object.get("error") {
        let message = error
            .as_object()
            .and_then(|error| error.get("message"))
            .and_then(Json::as_str)
            .unwrap_or("the language server reported an error without a message");
        return Err(message.to_owned());
    }
    Ok(object.get("result").cloned().unwrap_or(Json::Null))
}

/// Reads one `Content-Length` framed message.
fn read_message<R: BufRead>(reader: &mut R) -> Result<Message, String> {
    let mut length = None;
    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| format!("the language server's output could not be read: {error}"))?;
        if read == 0 {
            return Err("the language server closed its output".to_owned());
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| format!("{value:?} is not a content length"))?,
            );
        }
    }
    let length = length.ok_or_else(|| "a message had no Content-Length header".to_owned())?;
    let mut buffer = vec![0u8; length];
    reader
        .read_exact(&mut buffer)
        .map_err(|error| format!("the language server's message was truncated: {error}"))?;
    let text = String::from_utf8(buffer)
        .map_err(|_| "the language server sent a message that is not UTF-8".to_owned())?;
    let body =
        parse(&text).map_err(|error| format!("the language server sent invalid JSON: {error}"))?;
    let (id, method) = match body.as_object() {
        Some(object) => {
            let id = object.get("id").and_then(Json::as_i64);
            let method = object
                .get("method")
                .and_then(Json::as_str)
                .map(str::to_owned);
            (id, method)
        }
        None => (None, None),
    };
    Ok(Message { id, method, body })
}

/// A `file://` URI for a local path, percent-encoding what a URI cannot carry.
pub fn file_uri(path: &Path) -> String {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .unwrap_or_else(|_| PathBuf::from(path))
    };
    let mut out = String::from("file://");
    for byte in absolute.to_string_lossy().bytes() {
        let unreserved =
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/');
        if unreserved {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// A map of project to running client, so a caller that only has a project name can stop it.
#[derive(Default)]
pub struct Clients {
    running: BTreeMap<String, Client>,
}

impl Clients {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a client for a project, replacing (and stopping) an older one.
    pub fn insert(&mut self, project: impl Into<String>, client: Client) {
        let project = project.into();
        if let Some(mut old) = self.running.remove(&project) {
            old.shutdown();
        }
        self.running.insert(project, client);
    }

    /// Stops and removes a project's client.
    pub fn stop(&mut self, project: &str) -> bool {
        match self.running.remove(project) {
            Some(mut client) => {
                client.shutdown();
                true
            }
            None => false,
        }
    }

    /// A project's pid, when it is running.
    pub fn pid(&self, project: &str) -> Option<u32> {
        self.running.get(project).map(Client::pid)
    }

    /// A project's client, when it is running.
    pub fn client_mut(&mut self, project: &str) -> Option<&mut Client> {
        self.running.get_mut(project)
    }

    /// The number of running clients.
    pub fn len(&self) -> usize {
        self.running.len()
    }

    /// Whether none is running.
    pub fn is_empty(&self) -> bool {
        self.running.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::{file_uri, read_message, response_body};
    use crate::project::record::json::parse;
    use std::path::Path;

    #[test]
    fn a_file_uri_escapes_spaces_and_keeps_the_root() {
        let uri = file_uri(Path::new("/tmp/a folder/üi.rs"));
        assert!(uri.starts_with("file:///tmp/a%20folder/"));
        assert!(!uri.contains(' '));
        assert!(uri.contains("%C3%BC"));
    }

    #[test]
    fn a_response_with_an_error_answers_with_its_message() {
        let body =
            parse(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"no such method"}}"#)
                .expect("json");
        assert_eq!(response_body(body).expect_err("an error"), "no such method");
    }

    #[test]
    fn a_response_without_a_result_is_null_rather_than_a_guess() {
        let body = parse(r#"{"jsonrpc":"2.0","id":1}"#).expect("json");
        assert!(matches!(
            response_body(body),
            Ok(crate::project::record::json::Json::Null)
        ));
    }

    #[test]
    fn a_content_length_framed_message_is_read() {
        let body = r#"{"jsonrpc":"2.0","id":7,"result":{"ok":true}}"#;
        let framed = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        let mut reader = std::io::Cursor::new(framed.into_bytes());
        let message = read_message(&mut reader).expect("a framed message");
        assert_eq!(message.id, Some(7));
        assert!(
            message
                .body
                .as_object()
                .and_then(|object| object.get("result"))
                .is_some()
        );
    }

    #[test]
    fn a_message_without_a_content_length_is_refused() {
        let mut reader = std::io::Cursor::new(b"X-Thing: 1\r\n\r\n".to_vec());
        let error = read_message(&mut reader).expect_err("a refusal");
        assert!(error.contains("Content-Length"), "{error}");
    }
}
