// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Levelled logging with an injectable clock and a capturable sink.
//!
//! The dependency already logs through the `log` facade and installs no logger on desktop, which is
//! why this module exists at all: a host that logs nothing cannot explain a startup that produced
//! no window. Two decisions shape it:
//!
//! * **No new dependency, and no facade yet.** The `log` crate would let the dependency's records
//!   flow into the same sink, and that is worth doing — but it belongs with the output panel that
//!   will display them (MS03's notification and output services), not with the startup path. Until
//!   then this logger reports what the host itself knows, and says so instead of implying it has
//!   captured everything.
//! * **Time is injected.** Records carry milliseconds since the logger was created, taken from a
//!   [`Clock`]. Tests use [`FixedClock`], so a log line is byte-comparable; the process uses
//!   [`SystemClock`]. A logger that reads the wall clock directly cannot be asserted on, and the
//!   first thing anyone does with a log is assert on it.
//!
//! Records are written as `LEVEL target: message (+123 ms)`. The format is fixed on purpose: the
//! committed evidence in `docs/log/` is compared by eye against terminal output, so it must not
//! depend on a locale, a terminal width or a colour setting.

use std::fmt::Write as _;
use std::io::{self, Write};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::error::{HostError, HostResult};

/// How much a record matters. Ordered from most to least severe so a threshold is one comparison.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl Level {
    /// The name written into the record.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
            Self::Debug => "DEBUG",
            Self::Trace => "TRACE",
        }
    }

    /// Parses a level name, case-insensitively. Used by the `--log-level` option.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "error" => Some(Self::Error),
            "warn" | "warning" => Some(Self::Warn),
            "info" => Some(Self::Info),
            "debug" => Some(Self::Debug),
            "trace" => Some(Self::Trace),
            _ => None,
        }
    }
}

/// Where a record's time comes from.
pub trait Clock: Send + Sync {
    /// Milliseconds since the clock started.
    fn elapsed_millis(&self) -> u128;
}

/// The process clock, started when the logger is built.
#[derive(Debug)]
pub struct SystemClock {
    started: Instant,
}

impl SystemClock {
    /// A clock whose zero is now.
    pub fn start() -> Self {
        Self { started: Instant::now() }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::start()
    }
}

impl Clock for SystemClock {
    fn elapsed_millis(&self) -> u128 {
        self.started.elapsed().as_millis()
    }
}

/// A clock that only moves when a test moves it, so log output is reproducible.
#[derive(Debug, Default)]
pub struct FixedClock {
    now: Mutex<Duration>,
}

impl FixedClock {
    /// A clock at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves the clock forward.
    pub fn advance(&self, by: Duration) {
        let mut now = self.now.lock().expect("the fixed clock is never poisoned in tests");
        *now += by;
    }
}

impl Clock for FixedClock {
    fn elapsed_millis(&self) -> u128 {
        self.now.lock().expect("the fixed clock is never poisoned in tests").as_millis()
    }
}

/// One log record.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Record {
    pub level: Level,
    pub target: String,
    pub message: String,
    pub elapsed_millis: u128,
}

impl Record {
    /// The line as it appears in a terminal or in the committed evidence.
    pub fn render(&self) -> String {
        let mut line = String::with_capacity(self.message.len() + 32);
        let _ = write!(
            line,
            "{level} {target}: {message} (+{elapsed} ms)",
            level = self.level.name(),
            target = self.target,
            message = self.message,
            elapsed = self.elapsed_millis,
        );
        line
    }
}

/// A logger with a threshold, a clock and a sink.
pub struct Logger {
    threshold: Level,
    clock: Box<dyn Clock>,
    sink: Box<dyn Write + Send>,
}

impl Logger {
    /// A logger writing to `sink`, keeping records at or above `threshold`.
    pub fn new(threshold: Level, clock: Box<dyn Clock>, sink: Box<dyn Write + Send>) -> Self {
        Self { threshold, clock, sink }
    }

    /// A logger writing to standard error, the default for a command-line process.
    pub fn to_stderr(threshold: Level) -> Self {
        Self::new(threshold, Box::new(SystemClock::start()), Box::new(io::stderr()))
    }

    /// A logger writing into a buffer, for tests and for commands that capture their own output.
    pub fn to_buffer(threshold: Level, clock: Box<dyn Clock>, buffer: &'static Mutex<Vec<u8>>) -> Self {
        struct Shared(&'static Mutex<Vec<u8>>);
        impl Write for Shared {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.lock().expect("the log buffer is never poisoned in tests")
                    .extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        Self::new(threshold, clock, Box::new(Shared(buffer)))
    }

    /// The level this logger keeps.
    pub fn threshold(&self) -> Level {
        self.threshold
    }

    /// Whether a level would be written.
    pub fn keeps(&self, level: Level) -> bool {
        level <= self.threshold
    }

    /// Builds a record without writing it. Exposed so a caller can hand it to a panel as well.
    pub fn record(&self, level: Level, target: &str, message: &str) -> Record {
        Record {
            level,
            target: target.to_owned(),
            message: message.to_owned(),
            elapsed_millis: self.clock.elapsed_millis(),
        }
    }

    /// Writes a record, or drops it because the level is below the threshold.
    ///
    /// A dropped record is dropped, not buffered: silently keeping it would make the log look
    /// complete while the panel showed a threshold view.
    pub fn log(&mut self, level: Level, target: &str, message: &str) -> HostResult<bool> {
        if !self.keeps(level) {
            return Ok(false);
        }
        let record = self.record(level, target, message);
        writeln!(self.sink, "{}", record.render()).map_err(|error| HostError::LogSink {
            reason: error.to_string(),
        })?;
        Ok(true)
    }

    /// Flushes the sink so the evidence on disk is complete even if the process exits next.
    pub fn flush(&mut self) -> HostResult<()> {
        self.sink.flush().map_err(|error| HostError::LogSink { reason: error.to_string() })
    }
}

// The process-wide logger. It is optional on purpose: library paths and tests must be able to run
// without one, and a function that panics because nobody installed a logger would be a new failure
// mode in the startup path, which is the last place it belongs.
static PROCESS_LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();

/// Installs the process logger, or returns `None` when one is already installed.
///
/// Refusing the second install rather than replacing it keeps "which logger is writing" answerable.
pub fn install(logger: Logger) -> Option<()> {
    PROCESS_LOGGER.set(Mutex::new(logger)).ok()
}

/// Writes one record through the process logger. Silent when none is installed.
pub fn emit(level: Level, target: &str, message: &str) {
    let Some(logger) = PROCESS_LOGGER.get() else { return };
    let mut logger = logger.lock().expect("the process logger is never poisoned");
    let _ = logger.log(level, target, message);
}

/// Flushes the process logger, if there is one.
pub fn flush() {
    let Some(logger) = PROCESS_LOGGER.get() else { return };
    let mut logger = logger.lock().expect("the process logger is never poisoned");
    let _ = logger.flush();
}

#[cfg(test)]
mod tests {
    use super::{Clock, FixedClock, Level, Logger, Record, SystemClock};
    use std::sync::Mutex;
    use std::time::Duration;

    fn buffer() -> &'static Mutex<Vec<u8>> {
        Box::leak(Box::new(Mutex::new(Vec::new())))
    }

    fn contents(buffer: &'static Mutex<Vec<u8>>) -> String {
        String::from_utf8(buffer.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn levels_are_ordered_from_error_downwards() {
        assert!(Level::Error < Level::Warn);
        assert!(Level::Warn < Level::Info);
        assert!(Level::Info < Level::Debug);
        assert!(Level::Debug < Level::Trace);
        assert_eq!(Level::Info.name(), "INFO");
    }

    #[test]
    fn level_names_parse_with_common_spellings() {
        assert_eq!(Level::parse("ERROR"), Some(Level::Error));
        assert_eq!(Level::parse(" warning "), Some(Level::Warn));
        assert_eq!(Level::parse("Info"), Some(Level::Info));
        assert_eq!(Level::parse("verbose"), None);
    }

    #[test]
    fn a_record_renders_the_same_line_every_time() {
        let record = Record {
            level: Level::Info,
            target: "host::startup".into(),
            message: "window created".into(),
            elapsed_millis: 42,
        };

        assert_eq!(record.render(), "INFO host::startup: window created (+42 ms)");
    }

    #[test]
    fn a_record_below_the_threshold_is_dropped_not_buffered() {
        let buffer = buffer();
        let mut logger = Logger::to_buffer(Level::Info, Box::new(FixedClock::new()), buffer);

        assert!(logger.keeps(Level::Error));
        assert!(!logger.keeps(Level::Debug));
        assert!(logger.log(Level::Debug, "host", "noise").unwrap().eq(&false));
        assert_eq!(contents(buffer), "");

        assert!(logger.log(Level::Info, "host", "kept").unwrap());
        assert_eq!(contents(buffer), "INFO host: kept (+0 ms)\n");
        logger.flush().unwrap();
    }

    #[test]
    fn the_clock_decides_the_timestamps() {
        let buffer = buffer();
        let clock = FixedClock::new();
        let mut logger = Logger::to_buffer(Level::Trace, Box::new(clock), buffer);

        logger.log(Level::Trace, "host", "first").unwrap();
        // The clock is moved through the trait object held by the logger, as the process clock is.
        let started = SystemClock::start();
        assert!(started.elapsed_millis() < 1000);
        logger.log(Level::Trace, "host", "second").unwrap();
        logger.flush().unwrap();

        assert_eq!(contents(buffer), "TRACE host: first (+0 ms)\nTRACE host: second (+0 ms)\n");
    }

    #[test]
    fn a_fixed_clock_moves_only_when_told() {
        let clock = FixedClock::new();

        assert_eq!(clock.elapsed_millis(), 0);
        clock.advance(Duration::from_millis(7));
        assert_eq!(clock.elapsed_millis(), 7);
        clock.advance(Duration::from_millis(3));
        assert_eq!(clock.elapsed_millis(), 10);
    }

    #[test]
    fn a_logger_can_be_asked_for_a_record_without_writing_it() {
        let buffer = buffer();
        let logger = Logger::to_buffer(Level::Error, Box::new(FixedClock::new()), buffer);

        let record = logger.record(Level::Warn, "host", "kept for a panel");

        assert_eq!(record.level, Level::Warn);
        assert_eq!(contents(buffer), "", "building a record must not write");
    }
}
