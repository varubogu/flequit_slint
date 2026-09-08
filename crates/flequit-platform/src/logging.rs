//! The platform's own log sink.
//!
//! Desktop developers read stderr, but Android discards it and iOS only shows
//! it while a debugger is attached. Those platforms have a system log that the
//! OS tooling can read instead (`logcat`, Console.app), and picking between
//! them needs `cfg(target_os)` — which is why the choice lives here and not in
//! `flequit-app`.
//!
//! # Examples
//!
//! ```
//! use flequit_platform::SystemLogWriter;
//!
//! // `None` on platforms whose stderr is already the right sink.
//! if let Some(writer) = SystemLogWriter::current() {
//!     let _layer_writer = move || writer.clone();
//! }
//! ```

use std::io::{self, Write};

/// Writes one already-formatted log line to the platform log.
///
/// A plain `fn` pointer rather than a closure so that [`SystemLogWriter`] stays
/// `Copy` and can be handed to `tracing-subscriber`'s writer factory without an
/// allocation per event.
pub type LogLineEmitter = fn(&str);

/// A `std::io::Write` sink that forwards complete lines to the platform log.
///
/// System logs on Android and iOS are record-oriented, not byte streams: each
/// write becomes one entry with its own timestamp and level. Partial writes are
/// therefore buffered until a newline arrives, and whatever is left is emitted
/// on drop so a line without a trailing newline is not lost.
#[derive(Clone)]
pub struct SystemLogWriter {
    emit: LogLineEmitter,
    buffer: Vec<u8>,
}

impl SystemLogWriter {
    /// Returns the sink for the current platform, or `None` when stderr already
    /// reaches the platform's log tooling (all desktop targets).
    pub fn current() -> Option<Self> {
        crate::platform::log_line_emitter().map(Self::new)
    }

    /// Builds a sink around an explicit emitter. Used by backends and by tests.
    pub fn new(emit: LogLineEmitter) -> Self {
        Self {
            emit,
            buffer: Vec::new(),
        }
    }

    fn flush_complete_lines(&mut self) {
        while let Some(index) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line = self.buffer.drain(..=index).collect::<Vec<_>>();
            self.emit_slice(&line[..index]);
        }
    }

    fn emit_slice(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        (self.emit)(&String::from_utf8_lossy(bytes));
    }
}

impl Write for SystemLogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        self.flush_complete_lines();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let remainder = std::mem::take(&mut self.buffer);
        self.emit_slice(&remainder);
        Ok(())
    }
}

impl Drop for SystemLogWriter {
    fn drop(&mut self) {
        let remainder = std::mem::take(&mut self.buffer);
        self.emit_slice(&remainder);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use super::*;

    fn recorded() -> &'static Mutex<Vec<String>> {
        static LINES: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
        LINES.get_or_init(|| Mutex::new(Vec::new()))
    }

    fn record(line: &str) {
        recorded()
            .lock()
            .expect("recorder poisoned")
            .push(line.to_string());
    }

    #[test]
    fn splits_writes_into_lines_and_flushes_the_remainder_on_drop() {
        recorded().lock().unwrap().clear();

        let mut writer = SystemLogWriter::new(record);
        writer.write_all(b"first\nsec").expect("write");
        writer.write_all(b"ond\ntrailing").expect("write");

        assert_eq!(*recorded().lock().unwrap(), vec!["first", "second"]);

        drop(writer);
        assert_eq!(
            *recorded().lock().unwrap(),
            vec!["first", "second", "trailing"]
        );
    }
}
