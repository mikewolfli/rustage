// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Validating actions and keeping synchronous and high-frequency events in the UI, with a bounded queue
//! and deadlines (task T17.03).
//!
//! The acceptance line is *"drawing and IME timing are unchanged, and no event is dropped silently"*:
//!
//! * **A class says where an event runs.** [`Class::Sync`] and [`Class::HighFrequency`] stay in the UI
//!   process ([`Class::stays_in_ui`]), so a pointer move or an IME composition is handled where the
//!   frame is drawn and never becomes a round trip that would move the timing.
//! * **A queued item is beside an answer.** [`Queue::push`] answers [`Admission::StaysInUi`] for a UI
//!   class instead of enqueuing it, and [`Admission::Overflow`] carries the item it displaced, so a
//!   full queue is visible rather than a silent loss.
//! * **A synchronous event cannot be rerouted through the seam.** [`Queue::push`] refuses a
//!   [`Class::Sync`] item that needs the logic process with [`Refusal::SyncNeedsLogic`], so a
//!   synchronous event is not quietly turned into async IPC (task T13.06's rule, carried across the
//!   process boundary).
//! * **A deadline is checked where it is set and where it passes.** An item already past its deadline
//!   is answered [`Admission::Expired`], and [`Queue::expire`] returns one [`Expiry`] per item that ran
//!   out, so a timeout is an outcome and not a disappearance.

#![allow(dead_code)]

use std::collections::VecDeque;

/// Where an event is handled, and therefore whether it can cross to the logic process.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    /// Handled in the same call as the draw or the input: never leaves the UI.
    Sync,
    /// Frequent and cheap to coalesce: kept in the UI so it does not flood the channel.
    HighFrequency,
    /// A normal event the logic process handles.
    Normal,
}

impl Class {
    /// The class's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::HighFrequency => "high-frequency",
            Self::Normal => "normal",
        }
    }

    /// Reads a class's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "sync" => Some(Self::Sync),
            "high" => Some(Self::HighFrequency),
            "normal" => Some(Self::Normal),
            _ => None,
        }
    }

    /// Whether an event of this class is handled in the UI rather than the logic process.
    pub fn stays_in_ui(&self) -> bool {
        matches!(self, Self::Sync | Self::HighFrequency)
    }
}

/// One action waiting to cross to the logic process.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Item {
    /// The event's name.
    pub event: String,
    /// Where it is handled.
    pub class: Class,
    /// Whether it needs the logic process at all.
    pub needs_logic: bool,
    /// The moment it must be handled by, if any.
    pub deadline: Option<u32>,
}

impl Item {
    /// An item with no deadline.
    pub fn new(event: impl Into<String>, class: Class, needs_logic: bool) -> Self {
        Self {
            event: event.into(),
            class,
            needs_logic,
            deadline: None,
        }
    }

    /// The same item with a deadline.
    pub fn by(mut self, deadline: u32) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// The item in one line.
    pub fn render(&self) -> String {
        format!("{} ({})", self.event, self.class.name())
    }
}

/// What a push did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Admission {
    /// The item is in the queue.
    Admitted,
    /// The item's class is handled in the UI, so it was not enqueued.
    StaysInUi,
    /// The queue was full; the item it displaced is named.
    Overflow {
        /// The item that was dropped to make room.
        dropped: Item,
    },
    /// The item was already past its deadline and was not queued.
    Expired {
        /// The item that expired.
        item: Item,
    },
}

/// Why a push was refused outright.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// A synchronous event that needs the seam would move the timing.
    SyncNeedsLogic {
        /// The event.
        event: String,
    },
}

impl Refusal {
    /// The refusal in one line.
    pub fn render(&self) -> String {
        match self {
            Self::SyncNeedsLogic { event } => format!(
                "{event} is synchronous and needs the logic process; that would move the draw/IME \
                 timing"
            ),
        }
    }
}

/// One item that ran out of time.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Expiry {
    /// The event.
    pub event: String,
    /// The deadline it passed.
    pub deadline: u32,
}

impl Expiry {
    /// The expiry in one line.
    pub fn render(&self) -> String {
        format!("{} timed out at {}", self.event, self.deadline)
    }
}

/// A bounded queue of actions crossing to the logic process.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Queue {
    /// How many items the queue holds.
    pub capacity: usize,
    items: VecDeque<Item>,
}

impl Queue {
    /// A queue that holds `capacity` items.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            items: VecDeque::new(),
        }
    }

    /// How many items are waiting.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether nothing is waiting.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Offers an item at time `now`.
    pub fn push(&mut self, item: Item, now: u32) -> Result<Admission, Refusal> {
        if item.class == Class::Sync && item.needs_logic {
            return Err(Refusal::SyncNeedsLogic { event: item.event });
        }
        if item.class.stays_in_ui() {
            return Ok(Admission::StaysInUi);
        }
        if item.deadline.is_some_and(|deadline| deadline <= now) {
            return Ok(Admission::Expired { item });
        }
        if self.items.len() >= self.capacity {
            let dropped = self.items.pop_front().expect("a full queue has a front");
            self.items.push_back(item);
            return Ok(Admission::Overflow { dropped });
        }
        self.items.push_back(item);
        Ok(Admission::Admitted)
    }

    /// Takes the oldest waiting item.
    pub fn take(&mut self) -> Option<Item> {
        self.items.pop_front()
    }

    /// Removes and reports every item past its deadline at time `now`.
    pub fn expire(&mut self, now: u32) -> Vec<Expiry> {
        let mut expired = Vec::new();
        let mut kept = VecDeque::with_capacity(self.items.len());
        while let Some(item) = self.items.pop_front() {
            match item.deadline {
                Some(deadline) if deadline <= now => expired.push(Expiry {
                    event: item.event,
                    deadline,
                }),
                _ => kept.push_back(item),
            }
        }
        self.items = kept;
        expired
    }
}

#[cfg(test)]
mod tests {
    use super::{Admission, Class, Expiry, Item, Queue, Refusal};

    #[test]
    fn a_synchronous_event_stays_in_the_ui() {
        let mut queue = Queue::new(4);
        let admission = queue
            .push(Item::new("pointer-move", Class::Sync, false), 0)
            .expect("an admission");
        assert_eq!(admission, Admission::StaysInUi);
        assert!(queue.is_empty());
    }

    #[test]
    fn a_high_frequency_event_stays_in_the_ui() {
        let mut queue = Queue::new(4);
        assert_eq!(
            queue
                .push(Item::new("ime-compose", Class::HighFrequency, false), 0)
                .expect("an admission"),
            Admission::StaysInUi
        );
    }

    #[test]
    fn a_synchronous_event_that_needs_the_seam_is_refused() {
        let mut queue = Queue::new(4);
        let refusal = queue
            .push(Item::new("save", Class::Sync, true), 0)
            .expect_err("a refusal");
        assert_eq!(
            refusal,
            Refusal::SyncNeedsLogic {
                event: "save".to_owned(),
            }
        );
        assert!(refusal.render().contains("timing"));
    }

    #[test]
    fn a_full_queue_drops_the_oldest_and_names_it() {
        let mut queue = Queue::new(2);
        queue
            .push(Item::new("first", Class::Normal, true), 0)
            .expect("an admission");
        queue
            .push(Item::new("second", Class::Normal, true), 0)
            .expect("an admission");
        let admission = queue
            .push(Item::new("third", Class::Normal, true), 0)
            .expect("an admission");
        assert_eq!(
            admission,
            Admission::Overflow {
                dropped: Item::new("first", Class::Normal, true),
            }
        );
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.take().expect("an item").event, "second");
    }

    #[test]
    fn an_item_already_past_its_deadline_is_reported_not_queued() {
        let mut queue = Queue::new(4);
        let admission = queue
            .push(Item::new("late", Class::Normal, true).by(3), 5)
            .expect("an admission");
        assert_eq!(
            admission,
            Admission::Expired {
                item: Item::new("late", Class::Normal, true).by(3),
            }
        );
        assert!(queue.is_empty());
    }

    #[test]
    fn expire_reports_one_entry_per_timed_out_item() {
        let mut queue = Queue::new(4);
        queue
            .push(Item::new("a", Class::Normal, true).by(10), 0)
            .expect("an admission");
        queue
            .push(Item::new("b", Class::Normal, true).by(20), 0)
            .expect("an admission");
        let expired = queue.expire(15);
        assert_eq!(
            expired,
            vec![Expiry {
                event: "a".to_owned(),
                deadline: 10,
            }]
        );
        assert_eq!(queue.len(), 1);
        assert!(expired[0].render().contains("timed out"));
    }

    #[test]
    fn normal_items_are_taken_in_order() {
        let mut queue = Queue::new(4);
        queue
            .push(Item::new("a", Class::Normal, true), 0)
            .expect("an admission");
        queue
            .push(Item::new("b", Class::Normal, true), 0)
            .expect("an admission");
        assert_eq!(queue.take().expect("an item").event, "a");
        assert_eq!(queue.take().expect("an item").event, "b");
        assert!(queue.take().is_none());
    }

    #[test]
    fn classes_and_items_render_what_they_are() {
        assert_eq!(Class::parse("high"), Some(Class::HighFrequency));
        assert_eq!(Class::parse("other"), None);
        assert!(Class::HighFrequency.stays_in_ui());
        assert!(!Class::Normal.stays_in_ui());
        assert_eq!(Item::new("x", Class::Normal, true).render(), "x (normal)");
    }
}
