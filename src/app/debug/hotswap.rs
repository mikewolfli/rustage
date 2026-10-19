// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The boundary between an active debug session and the logic swap (task T19.07).
//!
//! The acceptance line is *"the debugged process is not silently replaced, and the session does not
//! claim its stack survived"*. Replacing logic starts a new generation of the business process; a
//! debugger attached to the old generation cannot follow it. Three statements keep that honest:
//!
//! * **The two operations exclude each other.** [`permits`] refuses a logic replacement while a
//!   debug session is live, and refuses starting a debug session while a replacement is paused
//!   mid-swap. A confirmation does not grant permission to keep debugging through it.
//! * **Confirmation is about ending the session, not about preserving it.** With `confirmed = true`
//!   the replacement is allowed, and [`after_swap`] says exactly what was lost: the session ended,
//!   a re-attach is required, and neither the call stack nor the breakpoint bindings are retained.
//! * **A launched process is stopped; an attached one is left alone.** The aftermath carries which
//!   one it was, so a person is told whether re-attaching means "start it again" or "the program is
//!   still running elsewhere".

#![allow(dead_code)]

/// An operation that touches the running business process.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Operation {
    /// Replacing the running logic with a freshly generated generation.
    ReplaceLogic,
    /// Starting a debug session.
    StartDebugging,
    /// Attaching to a process that already runs.
    Attach,
}

impl Operation {
    /// The operation's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::ReplaceLogic => "replace logic",
            Self::StartDebugging => "start debugging",
            Self::Attach => "attach",
        }
    }
}

/// What the request may do.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Decision {
    /// It may proceed.
    Allowed,
    /// It may proceed only after the person confirms, with this question.
    Confirm {
        /// The question to ask.
        prompt: String,
    },
    /// It may not proceed, with this reason.
    Refused {
        /// Why.
        reason: String,
    },
}

impl Decision {
    /// Whether the operation may run now.
    pub fn allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }

    /// The decision in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Allowed => "allowed".to_owned(),
            Self::Confirm { prompt } => format!("confirm: {prompt}"),
            Self::Refused { reason } => format!("refused: {reason}"),
        }
    }
}

/// Decides whether an operation may run.
///
/// `debug_live` is a session that is starting, running or stopped; `swap_running` is a replacement
/// that has paused the old generation and not finished. `confirmed` is a person's answer to the
/// question this function would have asked.
pub fn permits(
    operation: Operation,
    debug_live: bool,
    swap_running: bool,
    confirmed: bool,
) -> Decision {
    match operation {
        Operation::ReplaceLogic => {
            if swap_running {
                return Decision::Refused {
                    reason: "a logic replacement is already in progress".to_owned(),
                };
            }
            if debug_live {
                if confirmed {
                    return Decision::Allowed;
                }
                return Decision::Confirm {
                    prompt:
                        "replacing logic ends the debug session; the call stack and breakpoint \
                             bindings will not survive. Replace anyway?"
                            .to_owned(),
                };
            }
            Decision::Allowed
        }
        Operation::StartDebugging | Operation::Attach => {
            if swap_running {
                return Decision::Refused {
                    reason:
                        "the running logic is being replaced; finish or cancel the replacement first"
                            .to_owned(),
                };
            }
            Decision::Allowed
        }
    }
}

/// What a replacement did to a live debug session.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Aftermath {
    /// Whether a session ended because of the replacement.
    pub session_ended: bool,
    /// Whether a new debug session has to be started or attached.
    pub must_reattach: bool,
    /// Whether the old call stack survived. Always `false` by construction.
    pub stack_retained: bool,
    /// Whether breakpoint bindings survived. Always `false` by construction.
    pub breakpoints_retained: bool,
    /// Whether the program this session launched was stopped.
    pub launched_process_stopped: bool,
    /// What the person has to do next.
    pub detail: String,
}

/// The consequence of a replacement for a session that was or was not live.
pub fn after_swap(debug_live: bool, was_attached: bool) -> Aftermath {
    if !debug_live {
        return Aftermath {
            session_ended: false,
            must_reattach: false,
            stack_retained: false,
            breakpoints_retained: false,
            launched_process_stopped: false,
            detail: "no debug session was running".to_owned(),
        };
    }
    Aftermath {
        session_ended: true,
        must_reattach: true,
        stack_retained: false,
        breakpoints_retained: false,
        launched_process_stopped: !was_attached,
        detail: if was_attached {
            "the debug session ended; the process was attached to and is still running, so \
             re-attach to the new generation and re-create the breakpoints"
                .to_owned()
        } else {
            "the debug session ended and the process it launched was stopped; start a new \
             session for the new generation and re-create the breakpoints"
                .to_owned()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{Decision, Operation, after_swap, permits};

    #[test]
    fn replacing_logic_while_debugging_requires_confirmation() {
        let decision = permits(Operation::ReplaceLogic, true, false, false);
        assert!(matches!(decision, Decision::Confirm { .. }));
        assert!(decision.render().contains("call stack"));
        assert!(permits(Operation::ReplaceLogic, true, false, true).allowed());
        assert!(permits(Operation::ReplaceLogic, false, false, false).allowed());
    }

    #[test]
    fn a_swap_that_is_already_running_refuses_another_one() {
        let decision = permits(Operation::ReplaceLogic, false, true, false);
        assert!(matches!(decision, Decision::Refused { .. }));
        assert!(decision.render().contains("already in progress"));
    }

    #[test]
    fn debugging_cannot_start_while_the_logic_is_being_replaced() {
        for operation in [Operation::StartDebugging, Operation::Attach] {
            let decision = permits(operation, false, true, false);
            assert!(matches!(decision, Decision::Refused { .. }));
            assert!(permits(operation, false, false, false).allowed());
        }
    }

    #[test]
    fn a_replacement_never_claims_the_stack_or_breakpoints_survived() {
        let aftermath = after_swap(true, false);
        assert!(aftermath.session_ended);
        assert!(aftermath.must_reattach);
        assert!(!aftermath.stack_retained);
        assert!(!aftermath.breakpoints_retained);
        assert!(aftermath.launched_process_stopped);
        assert!(aftermath.detail.contains("start a new session"));
    }

    #[test]
    fn an_attached_process_is_not_reported_as_stopped() {
        let aftermath = after_swap(true, true);
        assert!(!aftermath.launched_process_stopped);
        assert!(aftermath.detail.contains("still running"));
    }

    #[test]
    fn nothing_is_claimed_when_no_session_was_live() {
        let aftermath = after_swap(false, false);
        assert!(!aftermath.session_ended);
        assert!(!aftermath.must_reattach);
        assert_eq!(aftermath.detail, "no debug session was running");
    }
}
