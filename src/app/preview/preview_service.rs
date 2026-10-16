// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The preview service: its lifecycle, the session it owns, and the instances and process that must
//! all be gone once it is stopped (task T14.01).
//!
//! The acceptance line is *"after stopping there is no dangling instance or process"*. A dangling
//! thing is one nobody can reach any more, so the state machine is built so stopping cannot leave one:
//!
//! * **One session at a time.** [`Preview::start`] refuses a second session while one is live, so the
//!   service never owns two.
//! * **An instance is a [`Generation`], and a rebuild is a new one.** [`Preview::rebuild`] bumps the
//!   generation, so a handle made for the old instance cannot read the new one — the same rule the
//!   design scene uses.
//! * **Stopping clears everything.** [`Preview::stop`] takes the process and the instances away and
//!   answers what it released, and [`Preview::is_clean`] is the honest check that nothing is left.
//! * **A transition out of order is refused.** Ready before starting, stopping before running and
//!   finishing a stop that never began are each `Err`, so the lifecycle cannot be half-entered.

#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::contracts::identity::NodeId;

/// Where the service is in its life.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum State {
    /// Not running.
    #[default]
    Stopped,
    /// Starting, not yet able to hold instances.
    Starting,
    /// Running.
    Running,
    /// Stopping: still holding what it must release.
    Stopping,
}

impl State {
    /// The state's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
        }
    }
}

/// A render instance of one node.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Instance {
    /// The node it renders.
    pub node: NodeId,
    /// Which build of it this is.
    pub generation: u32,
}

/// What stopping released.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Stopped {
    /// Whether a process was terminated.
    pub process: bool,
    /// The instances that were destroyed.
    pub instances: Vec<Instance>,
    /// The session that was released.
    pub session: Option<String>,
}

impl Stopped {
    /// The release in one line.
    pub fn render(&self) -> String {
        format!(
            "stopped: {} instance(s), process {}, session {}",
            self.instances.len(),
            if self.process { "terminated" } else { "none" },
            self.session.as_deref().unwrap_or("(none)")
        )
    }
}

/// The preview service.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Preview {
    /// Where it is in its life.
    pub state: State,
    /// The session it owns.
    pub session: Option<String>,
    /// The instances it is holding.
    pub instances: Vec<Instance>,
    /// The child process it started, if any.
    pub process: Option<u32>,
    generations: BTreeMap<NodeId, u32>,
}

impl Preview {
    /// A stopped service.
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a session, refusing a second one while the first is live.
    pub fn start(&mut self, session: impl Into<String>) -> Result<(), String> {
        match self.state {
            State::Stopped => {
                self.state = State::Starting;
                self.session = Some(session.into());
                Ok(())
            }
            other => Err(format!(
                "the service is {}, so a session cannot start",
                other.name()
            )),
        }
    }

    /// Moves a starting service to running.
    pub fn ready(&mut self) -> Result<(), String> {
        if self.state == State::Starting {
            self.state = State::Running;
            Ok(())
        } else {
            Err(format!(
                "the service is {}, not starting",
                self.state.name()
            ))
        }
    }

    /// Records the child process the preview runs in.
    pub fn spawn(&mut self, process: u32) -> Result<(), String> {
        if self.state != State::Running {
            return Err(format!(
                "the service is {}, so nothing can be spawned",
                self.state.name()
            ));
        }
        self.process = Some(process);
        Ok(())
    }

    /// Installs a render instance for a node, only while running.
    pub fn install(&mut self, node: NodeId) -> Result<Instance, String> {
        if self.state != State::Running {
            return Err(format!(
                "the service is {}, so no instance can be installed",
                self.state.name()
            ));
        }
        let generation = self.generations.entry(node).or_insert(0);
        let instance = Instance {
            node,
            generation: *generation,
        };
        match self.instances.iter_mut().find(|held| held.node == node) {
            Some(held) => *held = instance,
            None => self.instances.push(instance),
        }
        Ok(instance)
    }

    /// Rebuilds a node's instance as a new generation.
    pub fn rebuild(&mut self, node: NodeId) -> Result<Instance, String> {
        let generation = self.generations.entry(node).or_insert(0);
        *generation += 1;
        self.install(node)
    }

    /// Stops the service, releasing the process and the instances.
    pub fn stop(&mut self) -> Result<Stopped, String> {
        match self.state {
            State::Running | State::Starting => {
                self.state = State::Stopping;
                let released = Stopped {
                    process: self.process.is_some(),
                    instances: std::mem::take(&mut self.instances),
                    session: self.session.clone(),
                };
                self.process = None;
                Ok(released)
            }
            other => Err(format!(
                "the service is {}, so it cannot stop",
                other.name()
            )),
        }
    }

    /// Finishes a stop, releasing the session.
    pub fn finish_stop(&mut self) -> Result<(), String> {
        if self.state == State::Stopping {
            self.state = State::Stopped;
            self.session = None;
            Ok(())
        } else {
            Err(format!(
                "the service is {}, not stopping",
                self.state.name()
            ))
        }
    }

    /// Whether nothing is left over: no instances, no process and no session.
    pub fn is_clean(&self) -> bool {
        self.instances.is_empty() && self.process.is_none() && self.session.is_none()
    }

    /// The service in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![
            format!("state: {}", self.state.name()),
            format!("session: {}", self.session.as_deref().unwrap_or("(none)")),
            format!(
                "process: {}",
                self.process
                    .map(|process| process.to_string())
                    .unwrap_or_else(|| "(none)".to_owned())
            ),
        ];
        lines.push(format!("instances: {}", self.instances.len()));
        for instance in &self.instances {
            lines.push(format!(
                "  node {} (generation {})",
                instance.node.get(),
                instance.generation
            ));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{Preview, State};
    use crate::contracts::identity::NodeId;

    fn node(value: u64) -> NodeId {
        NodeId::from_raw(value)
    }

    fn running() -> Preview {
        let mut preview = Preview::new();
        preview.start("session-1").expect("a start");
        preview.ready().expect("a ready");
        preview
    }

    #[test]
    fn a_stopped_service_holds_nothing() {
        let preview = Preview::new();
        assert_eq!(preview.state, State::Stopped);
        assert!(preview.is_clean());
        assert!(preview.instances.is_empty());
        assert!(preview.process.is_none());
    }

    #[test]
    fn a_second_session_cannot_start_while_one_is_live() {
        let mut preview = running();
        let outcome = preview.start("session-2");
        assert!(outcome.unwrap_err().contains("cannot start"));
        assert_eq!(preview.session.as_deref(), Some("session-1"));
    }

    #[test]
    fn an_instance_cannot_be_installed_while_stopped() {
        let mut preview = Preview::new();
        assert!(preview.install(node(1)).unwrap_err().contains("stopped"));
    }

    #[test]
    fn installing_a_node_twice_does_not_duplicate_it() {
        let mut preview = running();
        preview.install(node(1)).expect("an install");
        preview.install(node(1)).expect("an install");
        assert_eq!(preview.instances.len(), 1);
    }

    #[test]
    fn a_rebuild_is_a_new_generation() {
        let mut preview = running();
        let first = preview.install(node(1)).expect("an install");
        let second = preview.rebuild(node(1)).expect("a rebuild");
        assert_eq!(first.generation, 0);
        assert_eq!(second.generation, 1);
        assert_eq!(preview.instances.len(), 1);
        assert_eq!(preview.instances[0].generation, 1);
    }

    #[test]
    fn stopping_releases_the_instances_and_the_process() {
        let mut preview = running();
        preview.spawn(4242).expect("a spawn");
        preview.install(node(1)).expect("an install");
        preview.install(node(2)).expect("an install");
        let released = preview.stop().expect("a stop");
        assert!(released.process);
        assert_eq!(released.instances.len(), 2);
        assert_eq!(released.session.as_deref(), Some("session-1"));
        assert!(preview.instances.is_empty());
        assert!(preview.process.is_none());
    }

    #[test]
    fn finishing_a_stop_releases_the_session_and_leaves_nothing_behind() {
        let mut preview = running();
        preview.spawn(7).expect("a spawn");
        preview.install(node(1)).expect("an install");
        preview.stop().expect("a stop");
        preview.finish_stop().expect("a finish");
        assert_eq!(preview.state, State::Stopped);
        assert!(preview.is_clean());
    }

    #[test]
    fn a_stop_cannot_be_finished_when_it_never_began() {
        let mut preview = running();
        assert!(preview.finish_stop().unwrap_err().contains("not stopping"));
        let mut preview = Preview::new();
        assert!(preview.stop().unwrap_err().contains("cannot stop"));
    }

    #[test]
    fn transitions_out_of_order_are_refused() {
        let mut preview = Preview::new();
        assert!(preview.ready().unwrap_err().contains("not starting"));
        let mut preview = running();
        assert!(preview.start("again").is_err());
        let mut stopped = running();
        stopped.stop().expect("a stop");
        stopped.finish_stop().expect("a finish");
        assert!(stopped.stop().unwrap_err().contains("cannot stop"));
    }

    #[test]
    fn nothing_can_be_spawned_before_the_service_is_running() {
        let mut preview = Preview::new();
        preview.start("s").expect("a start");
        assert!(preview.spawn(1).unwrap_err().contains("starting"));
        preview.ready().expect("a ready");
        assert!(preview.spawn(1).is_ok());
    }

    #[test]
    fn the_release_says_what_it_released() {
        let mut preview = running();
        preview.spawn(9).expect("a spawn");
        preview.install(node(1)).expect("an install");
        let released = preview.stop().expect("a stop");
        let line = released.render();
        assert!(line.contains("1 instance(s)"));
        assert!(line.contains("process terminated"));
        assert!(line.contains("session-1"));
    }

    #[test]
    fn the_service_renders_its_state_and_instances() {
        let mut preview = running();
        preview.install(node(3)).expect("an install");
        let lines = preview.render();
        assert_eq!(lines[0], "state: running");
        assert_eq!(lines[1], "session: session-1");
        assert_eq!(lines[3], "instances: 1");
        assert_eq!(lines[4], "  node 3 (generation 0)");
    }
}
