// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Local typed IPC between the stable UI process and the replaceable business process: a handshake, a
//! manifest, versions, requests, a session and a generation (task T17.02).
//!
//! The acceptance line is *"no bare handle or Rust object crosses, and the payload limit and the
//! errors are explicit"*. Each half of that is a property of the channel:
//!
//! * **The channel carries text.** [`Frame::encode`] and [`Frame::decode`] are the only way in or out,
//!   and both speak `&str`, so a pointer or a `Box` cannot survive the crossing — there is no API that
//!   could hand one over.
//! * **A frame is versioned and bounded.** Every frame names [`Protocol::VERSION`], and one past
//!   [`Protocol::MAX_PAYLOAD`] bytes is refused with [`Fault::TooLarge`], so a peer cannot grow the
//!   other side's memory with a single message.
//! * **A generation guards staleness.** A frame is stamped with the [`Generation`] it belongs to, and
//!   [`Session::send`] refuses one from another generation with [`Fault::StaleGeneration`], so a frame
//!   from the version being replaced is diagnosable rather than applied.
//! * **The handshake is checked, not assumed.** A protocol the host does not speak, or a manifest whose
//!   generation disagrees with the session, is refused before any request is accepted.

#![allow(dead_code)]

/// The protocol name and its rules.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Protocol;

impl Protocol {
    /// The frame format this build speaks.
    pub const VERSION: u32 = 1;
    /// The largest payload one frame may carry.
    pub const MAX_PAYLOAD: usize = 64 * 1024;
}

/// Which version of the logic an artifact or a frame belongs to.
pub type Generation = u64;

/// What a frame is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// The first frame: the protocol and the manifest.
    Handshake,
    /// What the business process offers.
    Manifest,
    /// The UI process asks the business process to do something.
    Request,
    /// The business process answers.
    Response,
    /// The business process reports something that happened.
    Event,
}

impl Kind {
    /// The kind's name, as written in a frame.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Handshake => "handshake",
            Self::Manifest => "manifest",
            Self::Request => "request",
            Self::Response => "response",
            Self::Event => "event",
        }
    }

    /// Reads a kind's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "handshake" => Some(Self::Handshake),
            "manifest" => Some(Self::Manifest),
            "request" => Some(Self::Request),
            "response" => Some(Self::Response),
            "event" => Some(Self::Event),
            _ => None,
        }
    }
}

/// One message across the channel: values only, encoded as text.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Frame {
    /// The protocol version the sender speaks.
    pub protocol: u32,
    /// What the frame is for.
    pub kind: Kind,
    /// Which session it belongs to.
    pub session: String,
    /// Which generation of the logic it belongs to.
    pub generation: Generation,
    /// The payload, opaque bytes to this module.
    pub payload: Vec<u8>,
}

impl Frame {
    /// A frame in the current protocol.
    pub fn new(
        kind: Kind,
        session: impl Into<String>,
        generation: Generation,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            protocol: Protocol::VERSION,
            kind,
            session: session.into(),
            generation,
            payload,
        }
    }

    /// Encodes the frame as one line of text, refusing an oversize payload or an unusable session id.
    pub fn encode(&self) -> Result<String, Fault> {
        if self.payload.len() > Protocol::MAX_PAYLOAD {
            return Err(Fault::TooLarge {
                size: self.payload.len(),
                limit: Protocol::MAX_PAYLOAD,
            });
        }
        if self.session.is_empty() || self.session.contains('|') {
            return Err(Fault::Malformed {
                reason: "the session id must be a non-empty name without '|'".to_owned(),
            });
        }
        Ok(format!(
            "v{}|{}|{}|{}|{}",
            self.protocol,
            self.kind.name(),
            self.session,
            self.generation,
            to_hex(&self.payload),
        ))
    }

    /// Decodes a frame from the text [`Frame::encode`] produced.
    pub fn decode(text: &str) -> Result<Self, Fault> {
        let fail = |reason: &str| Fault::Malformed {
            reason: reason.to_owned(),
        };
        let mut parts = text.split('|');
        let head = parts.next().ok_or_else(|| fail("no protocol field"))?;
        let protocol = head
            .strip_prefix('v')
            .and_then(|number| number.parse().ok())
            .ok_or_else(|| fail("the protocol field is not v<n>"))?;
        let kind = parts
            .next()
            .and_then(Kind::parse)
            .ok_or_else(|| fail("the kind is not a known frame kind"))?;
        let session = parts
            .next()
            .ok_or_else(|| fail("no session field"))?
            .to_owned();
        let generation = parts
            .next()
            .ok_or_else(|| fail("no generation field"))?
            .parse()
            .map_err(|_| fail("the generation is not a number"))?;
        let payload = parts
            .next()
            .ok_or_else(|| fail("no payload field"))
            .and_then(|hex| from_hex(hex).ok_or_else(|| fail("the payload is not hex")))?;
        if parts.next().is_some() {
            return Err(fail("there are more fields than a frame has"));
        }
        Ok(Self {
            protocol,
            kind,
            session,
            generation,
            payload,
        })
    }
}

/// What the business process offers, declared at the handshake.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Manifest {
    /// The process's name.
    pub name: String,
    /// The process's version, as a text form it chooses.
    pub version: String,
    /// Which generation of the logic this manifest describes.
    pub generation: Generation,
    /// What it can do.
    pub capabilities: Vec<String>,
}

impl Manifest {
    /// A manifest with a name, a version and a generation.
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        generation: Generation,
    ) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            generation,
            capabilities: Vec::new(),
        }
    }

    /// The same manifest offering one more capability.
    pub fn with(mut self, capability: impl Into<String>) -> Self {
        self.capabilities.push(capability.into());
        self
    }

    /// The manifest as the text a manifest frame carries, one field per line.
    ///
    /// A value containing a newline or an `=` would make the message ambiguous, so it is refused
    /// rather than escaped: the words in a manifest (a process name, a version) have no need for
    /// either, and a refusal a caller can see beats a parse that quietly changes a name.
    pub fn encode(&self) -> Result<String, Fault> {
        for (what, value) in [
            ("name", self.name.as_str()),
            ("version", self.version.as_str()),
        ] {
            if value.is_empty() || value.contains(['\n', '=']) {
                return Err(Fault::Malformed {
                    reason: format!(
                        "the manifest {what} must be non-empty and contain no '=' or newline"
                    ),
                });
            }
        }
        for capability in &self.capabilities {
            if capability.is_empty() || capability.contains(['\n', '=']) {
                return Err(Fault::Malformed {
                    reason: "a capability must be non-empty and contain no '=' or newline"
                        .to_owned(),
                });
            }
        }
        let mut text = format!(
            "name={}\nversion={}\ngeneration={}",
            self.name, self.version, self.generation
        );
        for capability in &self.capabilities {
            text.push_str("\ncapability=");
            text.push_str(capability);
        }
        Ok(text)
    }

    /// Reads the text [`Manifest::encode`] wrote.
    pub fn parse(text: &str) -> Result<Self, Fault> {
        let mut name = None;
        let mut version = None;
        let mut generation = None;
        let mut capabilities = Vec::new();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                return Err(Fault::Malformed {
                    reason: format!("the manifest line {line:?} is not <key>=<value>"),
                });
            };
            match key {
                "name" if name.is_none() => name = Some(value.to_owned()),
                "version" if version.is_none() => version = Some(value.to_owned()),
                "generation" if generation.is_none() => {
                    generation = Some(value.parse().map_err(|_| Fault::Malformed {
                        reason: format!("the manifest generation {value:?} is not a number"),
                    })?);
                }
                "capability" => capabilities.push(value.to_owned()),
                "name" | "version" | "generation" => {
                    return Err(Fault::Malformed {
                        reason: format!("the manifest names {key} more than once"),
                    });
                }
                other => {
                    return Err(Fault::Malformed {
                        reason: format!("the manifest has no field {other:?}"),
                    });
                }
            }
        }
        let name = name.ok_or_else(|| Fault::Malformed {
            reason: "the manifest has no name".to_owned(),
        })?;
        let version = version.ok_or_else(|| Fault::Malformed {
            reason: "the manifest has no version".to_owned(),
        })?;
        let generation = generation.ok_or_else(|| Fault::Malformed {
            reason: "the manifest has no generation".to_owned(),
        })?;
        if name.is_empty() || version.is_empty() {
            return Err(Fault::Malformed {
                reason: "the manifest name and version must be non-empty".to_owned(),
            });
        }
        if capabilities.iter().any(String::is_empty) {
            return Err(Fault::Malformed {
                reason: "a manifest capability must be non-empty".to_owned(),
            });
        }
        Ok(Self {
            name,
            version,
            generation,
            capabilities,
        })
    }
}

/// Why a frame or a handshake was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Fault {
    /// The payload is larger than the channel allows.
    TooLarge {
        /// The size that arrived.
        size: usize,
        /// The limit.
        limit: usize,
    },
    /// The text is not a frame.
    Malformed {
        /// Why not.
        reason: String,
    },
    /// The two sides speak different protocol versions.
    VersionMismatch {
        /// The version the host speaks.
        host: u32,
        /// The version the peer speaks.
        peer: u32,
    },
    /// The frame belongs to another session.
    SessionMismatch {
        /// The frame's session.
        frame: String,
        /// This session's id.
        session: String,
    },
    /// The frame belongs to another generation.
    StaleGeneration {
        /// The frame's generation.
        frame: Generation,
        /// This session's generation.
        session: Generation,
    },
}

impl Fault {
    /// The refusal in one line.
    pub fn render(&self) -> String {
        match self {
            Self::TooLarge { size, limit } => {
                format!("the payload is {size} bytes; the channel allows {limit}")
            }
            Self::Malformed { reason } => format!("the frame is malformed: {reason}"),
            Self::VersionMismatch { host, peer } => {
                format!("protocol v{host} and v{peer} do not match")
            }
            Self::SessionMismatch { frame, session } => {
                format!("the frame is for session {frame:?}, not {session:?}")
            }
            Self::StaleGeneration { frame, session } => {
                format!("the frame is from generation {frame}, not {session}")
            }
        }
    }
}

/// One handshake's outcome: the session the two sides share.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Session {
    /// The negotiated protocol version.
    pub protocol: u32,
    /// The session's id.
    pub id: String,
    /// The generation the session runs at.
    pub generation: Generation,
    /// What the business process offers.
    pub manifest: Manifest,
}

impl Session {
    /// Opens a session, checking the protocol and the manifest's generation first.
    pub fn open(
        id: impl Into<String>,
        host_protocol: u32,
        manifest: Manifest,
        current: Generation,
    ) -> Result<Self, Fault> {
        if host_protocol != Protocol::VERSION {
            return Err(Fault::VersionMismatch {
                host: host_protocol,
                peer: Protocol::VERSION,
            });
        }
        if manifest.generation != current {
            return Err(Fault::StaleGeneration {
                frame: manifest.generation,
                session: current,
            });
        }
        Ok(Self {
            protocol: host_protocol,
            id: id.into(),
            generation: current,
            manifest,
        })
    }

    /// Checks a frame belongs to this session and encodes it.
    pub fn send(&self, frame: &Frame) -> Result<String, Fault> {
        self.accept(frame)?;
        frame.encode()
    }

    /// Checks a frame and decodes it.
    pub fn receive(&self, text: &str) -> Result<Frame, Fault> {
        let frame = Frame::decode(text)?;
        self.accept(&frame)?;
        Ok(frame)
    }

    /// Whether a frame belongs to this session, for a side that decoded it already.
    pub fn accept(&self, frame: &Frame) -> Result<(), Fault> {
        if frame.protocol != self.protocol {
            return Err(Fault::VersionMismatch {
                host: self.protocol,
                peer: frame.protocol,
            });
        }
        if frame.session != self.id {
            return Err(Fault::SessionMismatch {
                frame: frame.session.clone(),
                session: self.id.clone(),
            });
        }
        if frame.generation != self.generation {
            return Err(Fault::StaleGeneration {
                frame: frame.generation,
                session: self.generation,
            });
        }
        Ok(())
    }
}

/// The payload written as lowercase hex, so any byte round-trips through text.
fn to_hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

/// Reads lowercase hex back into bytes.
fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let characters: Vec<char> = text.chars().collect();
    let mut bytes = Vec::with_capacity(characters.len() / 2);
    for pair in characters.chunks(2) {
        let high = pair[0].to_digit(16)?;
        let low = pair[1].to_digit(16)?;
        bytes.push((high * 16 + low) as u8);
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::{Fault, Frame, Kind, Manifest, Protocol, Session, from_hex, to_hex};

    fn session() -> Session {
        Session::open(
            "logic-1",
            Protocol::VERSION,
            Manifest::new("orders", "0.3.0", 4).with("place-order"),
            4,
        )
        .expect("a session")
    }

    #[test]
    fn a_frame_round_trips_any_bytes_through_text() {
        let frame = Frame::new(
            Kind::Request,
            "logic-1",
            4,
            vec![0, 1, 254, 255, b'|', b'\n'],
        );
        let text = frame.encode().expect("an encoded frame");
        assert_eq!(Frame::decode(&text).expect("a decoded frame"), frame);
    }

    #[test]
    fn an_oversize_payload_is_refused_with_both_numbers() {
        let frame = Frame::new(
            Kind::Request,
            "logic-1",
            4,
            vec![0; Protocol::MAX_PAYLOAD + 1],
        );
        let fault = frame.encode().expect_err("a refusal");
        assert_eq!(
            fault,
            Fault::TooLarge {
                size: Protocol::MAX_PAYLOAD + 1,
                limit: Protocol::MAX_PAYLOAD,
            }
        );
        assert!(fault.render().contains("allows"));
    }

    #[test]
    fn malformed_text_is_refused_not_guessed() {
        assert!(matches!(
            Frame::decode("nonsense"),
            Err(Fault::Malformed { .. })
        ));
        assert!(matches!(
            Frame::decode("v1|request|logic-1|4|zz"),
            Err(Fault::Malformed { .. })
        ));
    }

    #[test]
    fn a_handshake_with_another_protocol_is_refused() {
        let fault = Session::open("logic-1", 99, Manifest::new("orders", "0.3.0", 4), 4)
            .expect_err("a refusal");
        assert_eq!(
            fault,
            Fault::VersionMismatch {
                host: 99,
                peer: Protocol::VERSION,
            }
        );
    }

    #[test]
    fn a_manifest_from_another_generation_is_refused_at_the_handshake() {
        let fault = Session::open(
            "logic-1",
            Protocol::VERSION,
            Manifest::new("orders", "0.3.0", 2),
            4,
        )
        .expect_err("a refusal");
        assert_eq!(
            fault,
            Fault::StaleGeneration {
                frame: 2,
                session: 4,
            }
        );
    }

    #[test]
    fn a_frame_from_another_generation_is_refused() {
        let session = session();
        let stale = Frame::new(Kind::Request, "logic-1", 3, Vec::new());
        let fault = session.send(&stale).expect_err("a refusal");
        assert_eq!(
            fault,
            Fault::StaleGeneration {
                frame: 3,
                session: 4,
            }
        );
        assert!(fault.render().contains("generation"));
    }

    #[test]
    fn a_frame_for_another_session_is_refused() {
        let session = session();
        let other = Frame::new(Kind::Request, "logic-2", 4, Vec::new());
        assert!(matches!(
            session.send(&other),
            Err(Fault::SessionMismatch { .. })
        ));
    }

    #[test]
    fn a_session_round_trips_a_frame_it_owns() {
        let session = session();
        let frame = Frame::new(Kind::Event, "logic-1", 4, b"clicked".to_vec());
        let text = session.send(&frame).expect("a sent frame");
        assert_eq!(session.receive(&text).expect("a received frame"), frame);
    }

    #[test]
    fn a_session_id_with_the_separator_is_refused() {
        let frame = Frame::new(Kind::Request, "a|b", 4, Vec::new());
        assert!(matches!(frame.encode(), Err(Fault::Malformed { .. })));
    }

    #[test]
    fn a_manifest_round_trips_and_refuses_ambiguous_text() {
        let manifest = Manifest::new("orders", "0.3.0", 7)
            .with("reduce")
            .with("state");
        let text = manifest.encode().expect("manifest text");
        assert_eq!(Manifest::parse(&text).expect("a manifest"), manifest);
        assert!(
            Manifest::new("a=b", "0.1.0", 1).encode().is_err(),
            "an '=' in a name must be refused, not escaped"
        );
        assert!(Manifest::parse("name=orders\nversion=0.3.0\ngeneration=7\ncapability=").is_err());
        assert!(Manifest::parse("name=orders\nversion=0.3.0\nunknown=7").is_err());
    }

    #[test]
    fn hex_round_trips_and_rejects_odd_input() {
        let bytes = vec![0u8, 16, 255];
        assert_eq!(from_hex(&to_hex(&bytes)), Some(bytes));
        assert_eq!(from_hex("abc"), None);
    }
}
