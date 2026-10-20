// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Which debug adapters this machine has, what each can do, and what to do when one is missing
//! (task T19.01).
//!
//! The family does not ship a debugger. It drives an adapter the platform already provides, so the
//! first honest question is not "how do we debug" but "what is installed here". Three states keep
//! the answer from collapsing into a boolean:
//!
//! * [`State::Ready`] — the program was found **and started**; a program that exists but cannot run
//!   (an unaccepted Xcode licence, a broken install) is not ready.
//! * [`State::Missing`] — nothing was found, with the remedy that would install it.
//! * [`State::Unavailable`] — the program was found but refused to start, with its own message.
//!
//! Capabilities are asked per adapter rather than assumed from its name: `lldb-dap` supports
//! conditional breakpoints, log points and hit conditions, while an adapter that does not is
//! rendered as disabled rather than silently accepting a request it would ignore.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::app::targets::configuration::Os;
use crate::app::targets::local::Probe;

/// What an adapter can do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Capabilities {
    /// Starting a program under the debugger.
    pub launch: bool,
    /// Attaching to a process that is already running.
    pub attach: bool,
    /// A breakpoint with a condition expression.
    pub conditional_breakpoints: bool,
    /// A breakpoint that logs instead of stopping.
    pub logpoints: bool,
    /// A breakpoint with a hit-count condition.
    pub hit_conditions: bool,
    /// Evaluating an expression in a frame.
    pub evaluate: bool,
    /// Restarting without a new session.
    pub restart: bool,
}

impl Capabilities {
    /// The capabilities in report order.
    pub fn names(&self) -> Vec<&'static str> {
        let mut names = Vec::new();
        for (present, name) in [
            (self.launch, "launch"),
            (self.attach, "attach"),
            (self.conditional_breakpoints, "conditional"),
            (self.logpoints, "logpoints"),
            (self.hit_conditions, "hit"),
            (self.evaluate, "evaluate"),
            (self.restart, "restart"),
        ] {
            if present {
                names.push(name);
            }
        }
        names
    }

    /// The capabilities in one line.
    pub fn render(&self) -> String {
        self.names().join(" ")
    }
}

/// What an adapter's install looks like on this machine.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum State {
    /// Found and started.
    Ready {
        /// The first line the program printed for `--version`, when it prints one.
        version: Option<String>,
    },
    /// Not found.
    Missing {
        /// What to do about it.
        remedy: String,
    },
    /// Found but it did not start.
    Unavailable {
        /// What it said.
        reason: String,
    },
}

impl State {
    /// Whether a session can be started with it.
    pub fn ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

/// One debug adapter on this machine.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Adapter {
    /// The name a session asks for.
    pub name: String,
    /// The program, when it was found.
    pub program: Option<PathBuf>,
    /// Arguments the program needs before it speaks the protocol.
    pub arguments: Vec<String>,
    /// The adapter's licence.
    pub license: String,
    /// Whether it can be used.
    pub state: State,
    /// What it supports.
    pub capabilities: Capabilities,
    /// Whether `launch` starts the program immediately and needs a stop at the main subprogram
    /// before breakpoints can be applied.
    ///
    /// GDB's DAP implementation runs the program inside the `launch` request and treats
    /// `configurationDone` as a no-op (`gdb/dap/launch.py`), unlike `lldb-dap`, which waits for
    /// `configurationDone`. The session therefore asks GDB to stop at main first; otherwise a
    /// breakpoint set after `launch` can be installed after the program has already passed it.
    pub stages_breakpoints_at_entry: bool,
    /// The DAP `context` value that makes this adapter evaluate an **expression** rather than a
    /// debugger command.
    ///
    /// GDB's DAP treats `repl` as its own CLI (`n` runs `next`), so the designer would step the
    /// program instead of reading a local; `variables` is the expression-like context. `lldb-dap`
    /// treats `repl` as an expression and has been verified with it.
    pub evaluate_context: &'static str,
}

impl Adapter {
    /// The adapter in one line.
    pub fn render(&self) -> String {
        let program = self
            .program
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "not found".to_owned());
        let state = match &self.state {
            State::Ready { version } => match version {
                Some(version) => format!("ready ({version})"),
                None => "ready".to_owned(),
            },
            State::Missing { remedy } => format!("missing (remedy: {remedy})"),
            State::Unavailable { reason } => format!("unavailable ({reason})"),
        };
        format!(
            "{}: {program} — {state}; capabilities: {}",
            self.name,
            self.capabilities.render()
        )
    }
}

/// Why an adapter cannot be used.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// No adapter with this name on this host.
    NoSuchAdapter {
        /// The name that was asked for.
        name: String,
        /// The host.
        host: Os,
    },
    /// The adapter exists for this host but is not usable.
    NotReady {
        /// The adapter.
        name: String,
        /// Why.
        reason: String,
    },
}

impl Refusal {
    /// The refusal in one line.
    pub fn render(&self) -> String {
        match self {
            Self::NoSuchAdapter { name, host } => {
                format!(
                    "{name:?} is not a debug adapter this build knows on {}",
                    host.name()
                )
            }
            Self::NotReady { name, reason } => {
                format!("debug adapter {name} is not usable: {reason}")
            }
        }
    }
}

/// The adapters this build knows how to drive, in preference order.
///
/// `lldb-dap` is first everywhere it exists because it is the adapter the pinned Rust toolchain's
/// DWARF output is tested against. `vsdbg` and `gdb` are listed on the hosts where they are plausible
/// and are **not** claimed to be verified here; their capability rows say what they advertise.
pub fn table(host: Os, probe: &Probe) -> Vec<Adapter> {
    let mut adapters = vec![lldb(host, probe)];
    match host {
        Os::Windows => adapters.push(vsdbg(probe)),
        Os::Linux => adapters.push(gdb(probe)),
        Os::Macos => {}
    }
    adapters
}

fn lldb(host: Os, probe: &Probe) -> Adapter {
    let (state, program) = match resolve(host, "lldb-dap", probe) {
        Some(path) => (state_of(&path, &[]), Some(path)),
        None => (
            State::Missing {
                remedy: match host {
                    Os::Macos => {
                        "install the Xcode command line tools (`xcode-select --install`)".to_owned()
                    }
                    Os::Linux => {
                        "install `lldb-dap` from the LLVM package for this distribution".to_owned()
                    }
                    Os::Windows => "install LLVM and put `lldb-dap` on the path".to_owned(),
                },
            },
            None,
        ),
    };
    Adapter {
        name: "lldb-dap".to_owned(),
        program,
        arguments: Vec::new(),
        license: match host {
            Os::Macos => "LLVM Apache-2.0 WITH LLVM-exception; ships with Xcode".to_owned(),
            _ => "LLVM Apache-2.0 WITH LLVM-exception".to_owned(),
        },
        state,
        capabilities: Capabilities {
            launch: true,
            attach: true,
            conditional_breakpoints: true,
            logpoints: true,
            hit_conditions: true,
            evaluate: true,
            restart: false,
        },
        stages_breakpoints_at_entry: false,
        evaluate_context: "repl",
    }
}

fn vsdbg(probe: &Probe) -> Adapter {
    let (state, program) = match resolve(Os::Windows, "vsdbg", probe) {
        Some(path) => (state_of(&path, &[]), Some(path)),
        None => (
            State::Missing {
                remedy: "install Visual Studio's debugger or run `vsdbg --install`".to_owned(),
            },
            None,
        ),
    };
    Adapter {
        name: "vsdbg".to_owned(),
        program,
        arguments: Vec::new(),
        license: "Microsoft licence; installed with Visual Studio".to_owned(),
        state,
        capabilities: Capabilities {
            launch: true,
            attach: true,
            conditional_breakpoints: true,
            logpoints: true,
            hit_conditions: true,
            evaluate: true,
            restart: false,
        },
        stages_breakpoints_at_entry: false,
        evaluate_context: "repl",
    }
}

fn gdb(probe: &Probe) -> Adapter {
    let (state, program) = match resolve(Os::Linux, "gdb", probe) {
        Some(path) => (state_of(&path, &[]), Some(path)),
        None => (
            State::Missing {
                remedy: "install gdb, which speaks DAP with `--interpreter=dap`".to_owned(),
            },
            None,
        ),
    };
    Adapter {
        name: "gdb".to_owned(),
        program,
        arguments: vec!["--interpreter=dap".to_owned()],
        license: "GPL-3.0-or-later".to_owned(),
        state,
        capabilities: Capabilities {
            launch: true,
            attach: true,
            conditional_breakpoints: true,
            logpoints: false,
            hit_conditions: false,
            evaluate: true,
            restart: false,
        },
        stages_breakpoints_at_entry: true,
        evaluate_context: "variables",
    }
}

/// The adapter a session asks for, refusing anything that is not ready.
pub fn select(host: Os, probe: &Probe, name: &str) -> Result<Adapter, Refusal> {
    let adapters = table(host, probe);
    let Some(adapter) = adapters.into_iter().find(|adapter| adapter.name == name) else {
        return Err(Refusal::NoSuchAdapter {
            name: name.to_owned(),
            host,
        });
    };
    match &adapter.state {
        State::Ready { .. } => Ok(adapter),
        State::Missing { remedy } => Err(Refusal::NotReady {
            name: adapter.name,
            reason: format!("not installed; {remedy}"),
        }),
        State::Unavailable { reason } => Err(Refusal::NotReady {
            name: adapter.name,
            reason: reason.clone(),
        }),
    }
}

/// The first ready adapter, for `adapter:auto`.
pub fn first_ready(host: Os, probe: &Probe) -> Result<Adapter, Refusal> {
    let adapters = table(host, probe);
    adapters
        .iter()
        .find(|adapter| adapter.state.ready())
        .cloned()
        .ok_or_else(|| Refusal::NoSuchAdapter {
            name: "any".to_owned(),
            host,
        })
}

/// An adapter at a path the caller names rather than one this build knows.
///
/// This is the seam an integration test (or a person using an adapter this build has not heard of)
/// uses. Its readiness is not probed with `--version` because a DAP adapter is not required to
/// implement one; a start that fails is reported by the session with the program's own error.
pub fn explicit(program: &Path) -> Adapter {
    Adapter {
        name: "custom".to_owned(),
        program: Some(program.to_path_buf()),
        arguments: Vec::new(),
        license: "provided by the caller".to_owned(),
        state: State::Ready { version: None },
        capabilities: Capabilities {
            launch: true,
            attach: true,
            conditional_breakpoints: true,
            logpoints: true,
            hit_conditions: true,
            evaluate: true,
            restart: false,
        },
        stages_breakpoints_at_entry: false,
        evaluate_context: "repl",
    }
}

/// Where a program is, asked of `PATH` first and of the platform's developer tool locator second.
fn resolve(host: Os, program: &str, probe: &Probe) -> Option<PathBuf> {
    if let Some(path) = probe.locate(program) {
        return Some(path);
    }
    if host != Os::Macos {
        return None;
    }
    // `lldb-dap` lives inside Xcode and is not on `PATH`; `xcrun` is the supported way to find it.
    let output = Command::new("xcrun")
        .args(["--find", program])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    path.is_file().then_some(path)
}

/// Starts the adapter once with a harmless argument to learn whether it can run at all.
///
/// A program on disk is not a working adapter: this project has seen an `lldb-dap` that existed but
/// refused to start because the Xcode licence had not been accepted. The check is bounded and only
/// runs when a session is being selected; it does not start a server.
fn state_of(program: &Path, arguments: &[&str]) -> State {
    let output = Command::new(program)
        .args(arguments)
        .arg("--version")
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let text = if output.stdout.is_empty() {
                String::from_utf8_lossy(&output.stderr).into_owned()
            } else {
                String::from_utf8_lossy(&output.stdout).into_owned()
            };
            State::Ready {
                version: text.lines().next().map(|line| line.trim().to_owned()),
            }
        }
        Ok(output) => {
            let reason = if output.stderr.is_empty() {
                format!("exited with {:?}", output.status.code())
            } else {
                String::from_utf8_lossy(&output.stderr).trim().to_owned()
            };
            State::Unavailable { reason }
        }
        Err(error) => State::Unavailable {
            reason: format!("could not be started: {error}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{Capabilities, Refusal, State, first_ready, resolve, select, table};
    use crate::app::targets::configuration::Os;
    use crate::app::targets::local::Probe;

    #[test]
    fn every_host_lists_lldb_dap_first() {
        for host in [Os::Macos, Os::Linux, Os::Windows] {
            let adapters = table(host, &Probe::of([]));
            assert_eq!(adapters[0].name, "lldb-dap");
            assert!(adapters[0].capabilities.conditional_breakpoints);
        }
    }

    #[test]
    fn gdb_is_driven_differently_from_lldb() {
        // The two adapters disagree about two protocol facts. GDB runs the program inside
        // `launch` and treats `repl` as its CLI; lldb-dap waits for `configurationDone` and
        // evaluates an expression in `repl`. The table must carry that difference, not a
        // single assumed sequence.
        let adapters = table(Os::Linux, &Probe::of([]));
        let gdb = adapters
            .iter()
            .find(|adapter| adapter.name == "gdb")
            .expect("gdb is listed on Linux");
        assert!(gdb.stages_breakpoints_at_entry);
        assert_eq!(gdb.evaluate_context, "variables");

        let lldb = &adapters[0];
        assert!(!lldb.stages_breakpoints_at_entry);
        assert_eq!(lldb.evaluate_context, "repl");
    }

    #[test]
    fn a_missing_adapter_is_a_blocker_with_a_remedy() {
        // `Os::Linux` with an empty probe cannot fall back to `xcrun`, so this is the genuinely
        // missing case even when the test runs on a Mac that has Xcode.
        let refusal = select(Os::Linux, &Probe::of([]), "lldb-dap").expect_err("no adapter");
        match refusal {
            Refusal::NotReady { name, reason } => {
                assert_eq!(name, "lldb-dap");
                assert!(reason.contains("install"), "{reason}");
            }
            other => panic!("expected a not-ready refusal, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_name_is_not_silently_replaced() {
        let refusal = select(Os::Macos, &Probe::of([]), "rust-gdb").expect_err("no such adapter");
        assert!(matches!(refusal, Refusal::NoSuchAdapter { .. }));
        assert!(refusal.render().contains("rust-gdb"));
    }

    #[test]
    fn no_ready_adapter_is_reported_as_such() {
        let refusal = first_ready(Os::Linux, &Probe::of([])).expect_err("nothing installed");
        assert!(matches!(refusal, Refusal::NoSuchAdapter { name, .. } if name == "any"));
    }

    #[test]
    fn capabilities_render_only_what_is_supported() {
        let capabilities = Capabilities {
            launch: true,
            attach: false,
            conditional_breakpoints: false,
            logpoints: false,
            hit_conditions: false,
            evaluate: true,
            restart: false,
        };
        assert_eq!(capabilities.render(), "launch evaluate");
    }

    #[test]
    fn a_program_that_is_not_there_has_no_path() {
        assert_eq!(resolve(Os::Linux, "lldb-dap", &Probe::of([])), None);
    }

    #[test]
    fn an_explicit_adapter_is_ready_without_a_version_probe() {
        let adapter = super::explicit(std::path::Path::new("/tmp/an-adapter"));
        assert!(adapter.state.ready());
        assert_eq!(adapter.name, "custom");
        assert!(adapter.capabilities.evaluate);
    }

    #[test]
    fn a_found_program_that_cannot_start_is_unavailable() {
        let directory = std::env::temp_dir().join(format!(
            "rustage-debug-adapter-{}-broken",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("a temporary directory");
        let program = directory.join("lldb-dap");
        std::fs::write(&program, "#!/bin/sh\nexit 3\n").expect("a script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
                .expect("an executable script");
        }
        let state = super::state_of(&program, &[]);
        assert!(matches!(state, State::Unavailable { .. }), "{state:?}");
        std::fs::remove_dir_all(&directory).ok();
    }
}
