// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The clipboard host service: plain text in, an accurate answer out. Contract **HOST**.
//!
//! Roadmap T01.02 asks for the clipboard as a platform service. The dependency owns the OS part
//! (`NSPasteboard`, X11/Wayland, the Win32 clipboard); what this module owns is the *answer*:
//!
//! * **A write that did not happen is not a success.** `set_clipboard_text` returns `false` when
//!   the platform could not take the text, and that is mapped to [`ClipboardWrite::Refused`] with
//!   the reason named — never to a silent no-op (principle #22).
//! * **An empty read is not the same as a failed read.** The dependency's plain text accessor
//!   answers with `None` both when the clipboard is empty and when it holds something that is not
//!   text — it does not separate those two — so this module asks the *rich* backend first, which
//!   can, and only falls back to the text accessor when there is no rich backend. [`ClipboardRead`]
//!   therefore keeps `Text`, `NotText`, `Empty` and `Unavailable` apart wherever the backend lets it
//!   instead of flattening every one of them into `""`.
//!
//! Reading is not a failure when the clipboard is empty: an empty clipboard is a fact about the
//! user's session, not a defect. A read that *could not ask* the platform is
//! [`ClipboardRead::Unavailable`], and saying so is the point.
//!
//! **Scope.** Rich content (HTML, RTF, images, file lists) is T05.06's contract — it needs MIME
//! negotiation and cross-project rules that do not exist yet. This module deliberately reads rich
//! content only far enough to *classify* it, so that "the clipboard holds an image" cannot be
//! reported as "the clipboard holds no text".

use rust_widgets::platform::clipboard::ClipboardContent;

/// What happened when the host wrote to the clipboard.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ClipboardWrite {
    /// The platform took the text.
    Written,
    /// The platform refused the write. The reason names what was found.
    Refused { reason: String },
}

impl ClipboardWrite {
    /// Whether the platform took the text.
    pub fn is_written(&self) -> bool {
        matches!(self, Self::Written)
    }
}

/// What the clipboard currently holds.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ClipboardRead {
    /// The clipboard holds this text. An empty clipboard never produces this variant.
    Text(String),
    /// The clipboard holds content of another kind, named by its MIME type.
    NotText { content_type: String },
    /// The clipboard holds nothing this host can read.
    Empty,
    /// The clipboard could not be asked. Distinct from [`Self::Empty`] on purpose.
    Unavailable { reason: String },
}

/// The platform clipboard, as far as the host needs it.
///
/// A trait rather than free functions for the same reason [`super::startup`] uses one: a host test
/// must be able to produce a refusal and a non-text clipboard without owning the user's real
/// clipboard, and the clipboard is process-wide, so two tests that used it directly would race.
pub trait ClipboardHost {
    /// Replaces the clipboard's text.
    fn write_text(&self, text: &str) -> ClipboardWrite;
    /// Reports what the clipboard holds.
    fn read(&self) -> ClipboardRead;
}

/// The clipboard of the machine the host is running on.
pub struct NativeClipboard;

impl NativeClipboard {
    /// A handle. Constructing it touches no platform state.
    pub fn new() -> Self {
        Self
    }
}

impl Default for NativeClipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipboardHost for NativeClipboard {
    fn write_text(&self, text: &str) -> ClipboardWrite {
        if rust_widgets::set_clipboard_text(text) {
            return ClipboardWrite::Written;
        }
        ClipboardWrite::Refused {
            reason: "the platform clipboard backend reported that it did not take the text"
                .to_owned(),
        }
    }

    fn read(&self) -> ClipboardRead {
        match rust_widgets::platform_clipboard() {
            Some(backend) => match backend.get_contents() {
                Some(ClipboardContent::Text(text)) => ClipboardRead::Text(text),
                Some(other) => ClipboardRead::NotText {
                    content_type: other.content_type().to_owned(),
                },
                None => ClipboardRead::Empty,
            },
            // No rich backend: the text accessor is all there is. It answers `None` for "no text"
            // and for "not text" alike, so those two collapse into `Empty` — and the accessor's
            // `Option` says so, rather than the empty string a `String` accessor would have made
            // indistinguishable from a genuine empty read.
            None => match rust_widgets::clipboard_text() {
                Some(text) => ClipboardRead::Text(text),
                None => ClipboardRead::Empty,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ClipboardHost, ClipboardRead, ClipboardWrite, NativeClipboard};
    use rust_widgets::platform::clipboard::ClipboardContent;

    /// An in-memory clipboard, so classification is tested without the user's real one.
    struct FakeClipboard {
        content: std::cell::RefCell<Option<ClipboardContent>>,
        refuse_writes: bool,
        break_reads: bool,
    }

    impl FakeClipboard {
        fn holding(content: Option<ClipboardContent>) -> Self {
            Self {
                content: std::cell::RefCell::new(content),
                refuse_writes: false,
                break_reads: false,
            }
        }
    }

    impl ClipboardHost for FakeClipboard {
        fn write_text(&self, text: &str) -> ClipboardWrite {
            if self.refuse_writes {
                return ClipboardWrite::Refused {
                    reason: "the session's clipboard is owned by another process".to_owned(),
                };
            }
            *self.content.borrow_mut() = Some(ClipboardContent::Text(text.to_owned()));
            ClipboardWrite::Written
        }

        fn read(&self) -> ClipboardRead {
            if self.break_reads {
                return ClipboardRead::Unavailable {
                    reason: "the display server refused the selection request".to_owned(),
                };
            }
            match self.content.borrow().clone() {
                Some(ClipboardContent::Text(text)) => ClipboardRead::Text(text),
                Some(other) => ClipboardRead::NotText {
                    content_type: other.content_type().to_owned(),
                },
                None => ClipboardRead::Empty,
            }
        }
    }

    #[test]
    fn a_refused_write_is_not_a_success() {
        let clipboard = FakeClipboard {
            refuse_writes: true,
            ..FakeClipboard::holding(None)
        };

        let outcome = clipboard.write_text("anything");

        assert!(!outcome.is_written());
        match outcome {
            ClipboardWrite::Refused { reason } => assert!(reason.contains("another process")),
            ClipboardWrite::Written => unreachable!("the fake refuses writes"),
        }
    }

    #[test]
    fn a_non_text_clipboard_is_not_reported_as_no_text() {
        let clipboard = FakeClipboard::holding(Some(ClipboardContent::Image {
            width: 2,
            height: 2,
            data: vec![0; 16],
        }));

        assert_eq!(
            clipboard.read(),
            ClipboardRead::NotText {
                content_type: "image/png".to_owned()
            }
        );
        assert_ne!(clipboard.read(), ClipboardRead::Empty);
    }

    #[test]
    fn an_empty_clipboard_is_a_fact_and_an_unreadable_one_is_not() {
        let empty = FakeClipboard::holding(None);
        assert_eq!(empty.read(), ClipboardRead::Empty);

        let broken = FakeClipboard {
            break_reads: true,
            ..FakeClipboard::holding(None)
        };
        match broken.read() {
            ClipboardRead::Unavailable { reason } => {
                assert!(reason.contains("display server"), "{reason}");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    #[test]
    fn unicode_text_survives_the_write_and_the_read_unchanged() {
        let clipboard = FakeClipboard::holding(None);
        let text = "剪贴板 · clipboard · 😀 · e\u{301}";

        assert_eq!(clipboard.write_text(text), ClipboardWrite::Written);
        assert_eq!(clipboard.read(), ClipboardRead::Text(text.to_owned()));
    }

    #[test]
    fn the_native_handle_asks_the_platform_only_when_used() {
        // Constructing it must not touch the platform: `--help` builds no clipboard, and a test
        // process that never uses the clipboard must not disturb the user's.
        let clipboard = NativeClipboard::new();
        assert!(std::mem::size_of_val(&clipboard) == 0);
    }
}
