// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The MCP stdio transport for an authorized tool call (task T23.11).
//!
//! [`super::ToolCall`] is data and `ToolPolicy` is a grant; this module is the part that actually
//! talks to a server. It is deliberately small, because each thing it does is one of the properties
//! the task's acceptance line asks for:
//!
//! * **A process is owned, not borrowed.** [`McpServer::start`] goes through
//!   [`crate::host::subprocess::DuplexProcess`], which requires a permission permit, gives the child
//!   its own process group and labels it for the log. This module never spawns a program itself.
//! * **Only an authorized call is sent.** [`McpServer::call_tool`] takes the decision, not just the
//!   call: a `ToolDecision::Refused` stops before a byte reaches the child.
//! * **One body definition.** The `params` object is [`super::ToolCall::mcp_params`], the same one the
//!   preview and `--ai tool-mcp` render, so the bytes a person approved are the bytes a server reads.
//! * **Every wait is bounded and every refusal names itself.** A missing answer, an answer for another
//!   request, a JSON-RPC error and a server request this client cannot serve are distinct failures
//!   ([`McpFailure`]), never an empty result (principle #22).
//! * **Shutdown is explicit.** [`McpServer::shutdown`] closes the child's input, waits the caller's
//!   grace period and escalates; the caller keeps the `Termination` as the evidence of what happened.
//!
//! The transport is newline-delimited JSON-RPC as MCP's stdio transport defines it, which is also
//! exactly what `DuplexProcess` exchanges. Nothing here speaks HTTP or opens a socket.

use std::time::{Duration, Instant};

use crate::host::permission::ExecutionPermit;
use crate::host::subprocess::{DuplexProcess, ProcessSpec, Termination};
use crate::project::record::json::{self, Json, JsonObject};

use super::{ToolCall, ToolDecision};

/// The protocol version this client advertises in `initialize`.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// How long one request may wait for its answer unless the caller says otherwise.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Why the transport could not do what was asked.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum McpFailure {
    /// The server process could not be started.
    Spawn {
        /// The host's own reason.
        reason: String,
    },
    /// The call was not authorized by a person.
    Refused {
        /// Why it was refused.
        reason: String,
    },
    /// The call's body could not be built, so nothing was sent.
    Unsendable {
        /// What is wrong with it.
        reason: String,
    },
    /// The child could not be written to or read from.
    Transport {
        /// What went wrong.
        reason: String,
    },
    /// The child answered in a shape this client cannot accept.
    Protocol {
        /// What was wrong with the answer.
        reason: String,
    },
    /// The child answered with a JSON-RPC error.
    Server {
        /// The error body the server sent, so the reason survives the transport.
        reason: String,
    },
}

impl McpFailure {
    /// A sentence for a log line, a panel or a terminal.
    pub fn message(&self) -> String {
        match self {
            Self::Spawn { reason } => format!("the MCP server could not be started: {reason}"),
            Self::Refused { reason } => format!("the tool call was not sent: {reason}"),
            Self::Unsendable { reason } => format!("the tool call is not sendable: {reason}"),
            Self::Transport { reason } => format!("the MCP transport failed: {reason}"),
            Self::Protocol { reason } => format!("the MCP server answered unexpectedly: {reason}"),
            Self::Server { reason } => format!("the MCP server reported an error: {reason}"),
        }
    }
}

impl std::fmt::Display for McpFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for McpFailure {}

/// A started MCP server over its standard streams.
pub struct McpServer {
    process: DuplexProcess,
    next_id: u64,
    timeout: Duration,
    initialized: bool,
}

impl McpServer {
    /// Starts a server through the host's process seam.
    ///
    /// The caller supplies the [`ProcessSpec`] (program, arguments, working directory) and the
    /// [`ExecutionPermit`] the host's permission service minted for running a command. This module
    /// cannot start anything without both.
    pub fn start(
        spec: ProcessSpec,
        timeout: Duration,
        permit: &ExecutionPermit,
    ) -> Result<Self, McpFailure> {
        let process = DuplexProcess::spawn(spec, permit).map_err(|error| McpFailure::Spawn {
            reason: error.message(),
        })?;
        Ok(Self {
            process,
            next_id: 1,
            timeout,
            initialized: false,
        })
    }

    /// What the caller called the child.
    pub fn label(&self) -> &str {
        self.process.label()
    }

    /// The child's process id, for a log line.
    pub fn pid(&self) -> u32 {
        self.process.pid()
    }

    /// What the child wrote to standard error, bounded by the host's capture limit.
    pub fn stderr(&self) -> String {
        self.process.stderr()
    }

    /// Performs the `initialize` handshake and sends `notifications/initialized`.
    ///
    /// The returned value is the server's `result`, so the caller can show the server's own name and
    /// protocol version instead of assuming the one it asked for.
    pub fn initialize(&mut self) -> Result<Json, McpFailure> {
        let mut client_info = JsonObject::new();
        client_info.insert("name", Json::string("rustage"));
        client_info.insert("version", Json::string(env!("CARGO_PKG_VERSION")));
        let mut params = JsonObject::new();
        params.insert("protocolVersion", Json::string(PROTOCOL_VERSION));
        params.insert("capabilities", Json::Object(JsonObject::new()));
        params.insert("clientInfo", Json::Object(client_info));
        let result = self.request("initialize", Json::Object(params))?;
        if !matches!(result, Json::Object(_)) {
            return Err(McpFailure::Protocol {
                reason: format!(
                    "the initialize result is {}, not an object",
                    json_kind(&result)
                ),
            });
        }
        self.notify("notifications/initialized", Json::Object(JsonObject::new()))?;
        self.initialized = true;
        Ok(result)
    }

    /// The tools the server advertises (`tools/list`).
    ///
    /// The result is returned as the server sent it: the host does not keep a second list of tool
    /// names, because a name this build invented would be a permission the server never offered.
    /// The caller reads `tools` from it; a server that paginates advertises that with `nextCursor`,
    /// and this method deliberately asks for one page rather than pretending the list is complete.
    pub fn list_tools(&mut self) -> Result<Json, McpFailure> {
        if !self.initialized {
            return Err(McpFailure::Protocol {
                reason: "the server has not completed initialize".to_owned(),
            });
        }
        self.request("tools/list", Json::Object(JsonObject::new()))
    }

    /// Sends one authorized `tools/call` and returns the server's `result`.
    ///
    /// The decision is required rather than inferred: a caller that has not asked a person (or its
    /// own policy) gets a refusal here even if it built the [`ToolCall`] itself.
    pub fn call_tool(
        &mut self,
        call: &ToolCall,
        decision: &ToolDecision,
    ) -> Result<Json, McpFailure> {
        if let ToolDecision::Refused { reason } = decision {
            return Err(McpFailure::Refused {
                reason: reason.clone(),
            });
        }
        if !self.initialized {
            return Err(McpFailure::Protocol {
                reason: "the server has not completed initialize".to_owned(),
            });
        }
        let params = call
            .mcp_params()
            .map_err(|reason| McpFailure::Unsendable { reason })?;
        self.request("tools/call", Json::Object(params))
    }

    /// Closes the child's input, waits `grace`, and escalates if it does not end.
    pub fn shutdown(&mut self, grace: Duration) -> Termination {
        self.process.finish(grace)
    }

    /// One request and the answer whose `id` matches it.
    ///
    /// Notifications the server sends in between are skipped; a response for another `id`, a server
    /// request this client cannot serve, and an error body are each reported instead of being
    /// mistaken for the answer.
    fn request(&mut self, method: &str, params: Json) -> Result<Json, McpFailure> {
        let id = self.next_id;
        self.next_id += 1;
        let body = envelope(Some(id), method, params);
        self.process
            .send(&body.render_compact())
            .map_err(|reason| McpFailure::Transport { reason })?;

        let deadline = Instant::now() + self.timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(McpFailure::Transport {
                    reason: format!(
                        "{} did not answer {method} within {:?}",
                        self.process.label(),
                        self.timeout
                    ),
                });
            }
            let line = self
                .process
                .receive(remaining)
                .map_err(|reason| McpFailure::Transport { reason })?;
            let value = json::parse(&line).map_err(|error| McpFailure::Protocol {
                reason: format!(
                    "a line from {} is not JSON: {} (line {}, column {})",
                    self.process.label(),
                    error.kind().message(),
                    error.line(),
                    error.column()
                ),
            })?;
            let Some(object) = value.as_object() else {
                return Err(McpFailure::Protocol {
                    reason: format!(
                        "{} sent {}, not a JSON-RPC object",
                        self.process.label(),
                        json_kind(&value)
                    ),
                });
            };
            if object.contains("method") {
                // A server notification carries no id and is safe to skip; a server request needs a
                // method this client does not implement, and pretending it was answered would be a
                // false claim about the session.
                if object.contains("id") {
                    return Err(McpFailure::Protocol {
                        reason: format!(
                            "{} sent a request ({}) this client cannot serve",
                            self.process.label(),
                            object
                                .get("method")
                                .and_then(|value| value.as_str())
                                .unwrap_or("(unnamed)")
                        ),
                    });
                }
                continue;
            }
            let answer_id = object.get("id").and_then(|value| value.as_u64());
            if answer_id != Some(id) {
                // An answer to a request this client did not send, or a late answer after a timeout.
                // Skipping is deliberate: it is not this request's result.
                continue;
            }
            if let Some(error) = object.get("error") {
                return Err(McpFailure::Server {
                    reason: error.render_compact(),
                });
            }
            return object
                .get("result")
                .cloned()
                .ok_or_else(|| McpFailure::Protocol {
                    reason: format!("the answer to {method} has neither result nor error"),
                });
        }
    }

    /// A notification: no id, and no answer is expected.
    fn notify(&mut self, method: &str, params: Json) -> Result<(), McpFailure> {
        let body = envelope(None, method, params);
        self.process
            .send(&body.render_compact())
            .map_err(|reason| McpFailure::Transport { reason })
    }
}

/// A JSON-RPC 2.0 envelope. `id` is `None` for a notification.
fn envelope(id: Option<u64>, method: &str, params: Json) -> Json {
    let mut root = JsonObject::new();
    root.insert("jsonrpc", Json::string("2.0"));
    if let Some(id) = id {
        root.insert("id", Json::number(id));
    }
    root.insert("method", Json::string(method));
    root.insert("params", params);
    Json::Object(root)
}

/// The JSON kind, for a message that has to say what arrived instead.
fn json_kind(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "a boolean",
        Json::Number(_) => "a number",
        Json::String(_) => "a string",
        Json::Array(_) => "an array",
        Json::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_REQUEST_TIMEOUT, McpFailure, McpServer};
    use crate::app::ai::{ToolCall, ToolPolicy};
    use crate::contracts::permission::{EffectClass, Principal};
    use crate::host::permission::{PermissionService, Scope};
    use crate::host::subprocess::ProcessSpec;
    use std::time::Duration;

    #[cfg(unix)]
    fn permit() -> crate::host::permission::ExecutionPermit {
        PermissionService::new()
            .permit(EffectClass::Command, Scope::Session, Principal::Human)
            .expect("the session may run a command")
    }

    #[cfg(unix)]
    fn scripted_server(script: &str) -> ProcessSpec {
        ProcessSpec::new("mcp-test-server", "sh").args(["-c", script])
    }

    #[cfg(unix)]
    #[test]
    fn the_handshake_and_an_authorized_call_reach_the_server_and_come_back() {
        // Reads initialize, the initialized notification, then tools/call, and answers both requests
        // in order. The ids are the transport's, so the test also proves the client pairs them.
        let script = concat!(
            "read a\n",
            "printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{},\"serverInfo\":{\"name\":\"fake\",\"version\":\"1\"}}}'\n",
            "read b\n",
            "read c\n",
            "printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]}}'\n",
        );
        let mut server =
            McpServer::start(scripted_server(script), DEFAULT_REQUEST_TIMEOUT, &permit())
                .expect("the server starts");

        let initialized = server.initialize().expect("the handshake answers");
        let server_info = initialized
            .as_object()
            .and_then(|object| object.get("serverInfo"))
            .and_then(|value| value.as_object())
            .and_then(|object| object.get("name"))
            .and_then(|value| value.as_str());
        assert_eq!(
            server_info,
            Some("fake"),
            "the server's own identity comes back"
        );

        let call = ToolCall {
            role: crate::app::ai::Role::Coder,
            name: "read_file".to_owned(),
            arguments: "{\"path\":\"a.rs\"}".to_owned(),
        };
        let mut policy = ToolPolicy::default();
        let refused = server
            .call_tool(&call, &policy.decide(&call))
            .expect_err("an unauthorized call is not sent");
        assert!(matches!(refused, McpFailure::Refused { .. }), "{refused:?}");

        policy.authorize(&call);
        let result = server
            .call_tool(&call, &policy.decide(&call))
            .expect("the authorized call answers");
        assert!(result.render_compact().contains("ok"), "{result:?}");

        server.shutdown(Duration::from_millis(500));
    }

    #[cfg(unix)]
    #[test]
    fn the_tool_list_comes_from_the_server_rather_than_a_host_table() {
        let script = concat!(
            "read a\n",
            "printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{},\"serverInfo\":{\"name\":\"fake\",\"version\":\"1\"}}}'\n",
            "read b\n",
            "read c\n",
            "printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[{\"name\":\"read_file\",\"description\":\"read one file\",\"inputSchema\":{\"type\":\"object\"}}]}}'\n",
        );
        let mut server =
            McpServer::start(scripted_server(script), DEFAULT_REQUEST_TIMEOUT, &permit())
                .expect("the server starts");
        server.initialize().expect("the handshake answers");

        let tools = server.list_tools().expect("the list answers");
        let names: Vec<&str> = tools
            .as_object()
            .and_then(|object| object.get("tools"))
            .and_then(|value| value.as_array())
            .unwrap_or_default()
            .iter()
            .filter_map(|tool| tool.as_object())
            .filter_map(|tool| tool.get("name"))
            .filter_map(|name| name.as_str())
            .collect();
        assert_eq!(names, vec!["read_file"]);

        server.shutdown(Duration::from_millis(500));
    }

    #[cfg(unix)]
    #[test]
    fn a_server_error_is_reported_with_its_body_not_as_an_empty_result() {
        let script = concat!(
            "read a\n",
            "printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{},\"serverInfo\":{\"name\":\"fake\",\"version\":\"1\"}}}'\n",
            "read b\n",
            "read c\n",
            "printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":2,\"error\":{\"code\":-32602,\"message\":\"unknown tool\"}}'\n",
        );
        let mut server =
            McpServer::start(scripted_server(script), DEFAULT_REQUEST_TIMEOUT, &permit())
                .expect("the server starts");
        server.initialize().expect("the handshake answers");

        let call = ToolCall {
            role: crate::app::ai::Role::Coder,
            name: "missing".to_owned(),
            arguments: "{}".to_owned(),
        };
        let mut policy = ToolPolicy::default();
        policy.authorize(&call);
        let failure = server
            .call_tool(&call, &policy.decide(&call))
            .expect_err("the error body is not a result");
        match failure {
            McpFailure::Server { reason } => {
                assert!(reason.contains("unknown tool"), "{reason}");
            }
            other => panic!("expected a server error, got {other:?}"),
        }

        server.shutdown(Duration::from_millis(500));
    }

    #[cfg(unix)]
    #[test]
    fn a_call_that_is_not_a_json_object_is_refused_before_it_is_sent() {
        let mut server = McpServer::start(
            scripted_server("read a\nprintf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{},\"serverInfo\":{\"name\":\"fake\",\"version\":\"1\"}}}'\nread b\nread c\n"),
            DEFAULT_REQUEST_TIMEOUT,
            &permit(),
        )
        .expect("the server starts");
        server.initialize().expect("the handshake answers");

        let call = ToolCall {
            role: crate::app::ai::Role::Coder,
            name: "read_file".to_owned(),
            arguments: "path=a.rs".to_owned(),
        };
        let mut policy = ToolPolicy::default();
        policy.authorize(&call);
        let failure = server
            .call_tool(&call, &policy.decide(&call))
            .expect_err("non-JSON arguments are not sendable");
        assert!(
            matches!(failure, McpFailure::Unsendable { .. }),
            "{failure:?}"
        );

        server.shutdown(Duration::from_millis(500));
    }
}
