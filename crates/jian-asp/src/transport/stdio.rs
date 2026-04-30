//! Stdio transport — reads NDJSON requests from stdin, writes
//! responses to stdout. The dev-tools agent CLI in `bin/` uses
//! this to drive a running app from the same terminal that
//! launched it.
//!
//! Buffered reads via [`std::io::BufReader::read_line`] so we
//! don't allocate per byte; flushed writes after every response so
//! a downstream pipe sees the line immediately rather than after
//! stdio's default block-buffering. The `Box<dyn BufRead>` /
//! `Box<dyn Write>` indirection lets tests substitute in-memory
//! cursors without spinning a real process.

use super::{Transport, TransportError};
use std::io::{BufRead, BufReader, Stdin, Stdout, Write};

/// Stdio transport. Use [`StdioTransport::on_stdio`] for the
/// production case; `from_streams` exposes the constructor that
/// tests reuse.
pub struct StdioTransport {
    reader: Box<dyn BufRead>,
    writer: Box<dyn Write>,
}

impl StdioTransport {
    /// Build the production transport reading `std::io::stdin()` and
    /// writing `std::io::stdout()`.
    pub fn on_stdio() -> Self {
        Self::from_streams(
            BufReader::new(StdinReader(std::io::stdin())),
            Box::new(std::io::stdout()),
        )
    }

    /// Build from arbitrary boxed streams. Tests pair an
    /// `&[u8]`-backed `BufReader` with a `Vec<u8>` writer; the
    /// production caller hands in stdin / stdout.
    pub fn from_streams(reader: impl BufRead + 'static, writer: Box<dyn Write>) -> Self {
        Self {
            reader: Box::new(reader),
            writer,
        }
    }
}

/// Tiny wrapper because `Stdin` itself doesn't implement `BufRead`
/// (it owns a per-call mutex `StdinLock`). Wrapping in
/// `BufReader` would borrow it on each call, so we hold the
/// `Stdin` and rebuild a `lock()` per `read_line`. This trades
/// a tiny lock-acquire cost per line for a `'static` reader.
struct StdinReader(Stdin);

impl std::io::Read for StdinReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.lock().read(buf)
    }
}

#[allow(dead_code)]
fn _stdout_writer_helper(_: Stdout) {} // keep import tidy

impl Transport for StdioTransport {
    fn read_line(&mut self) -> Result<String, TransportError> {
        let mut buf = String::new();
        match self.reader.read_line(&mut buf) {
            Ok(0) => Err(TransportError::Eof),
            Ok(_) => {
                // `read_line` includes the terminating `\n` when
                // present; strip it so the caller only sees the
                // payload bytes. Also strip a preceding `\r` so
                // CRLF lines from a Windows-side agent work.
                if buf.ends_with('\n') {
                    buf.pop();
                    if buf.ends_with('\r') {
                        buf.pop();
                    }
                }
                Ok(buf)
            }
            Err(e) => Err(TransportError::Io(e.to_string())),
        }
    }

    fn write_line(&mut self, line: &str) -> Result<(), TransportError> {
        self.writer
            .write_all(line.as_bytes())
            .and_then(|_| self.writer.write_all(b"\n"))
            .and_then(|_| self.writer.flush())
            .map_err(|e| TransportError::Io(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Build an in-memory transport over fixed input bytes; capture
    /// output via the returned `Vec<u8>` once dropped. The
    /// `Rc<RefCell<Vec<u8>>>` lets the test inspect what the
    /// transport wrote without wrestling lifetimes.
    fn rig(input: &[u8]) -> (StdioTransport, std::rc::Rc<std::cell::RefCell<Vec<u8>>>) {
        let cursor = Cursor::new(input.to_vec());
        let out = std::rc::Rc::new(std::cell::RefCell::new(Vec::<u8>::new()));
        let writer: Box<dyn Write> = Box::new(SharedWriter(out.clone()));
        let t = StdioTransport::from_streams(cursor, writer);
        (t, out)
    }

    /// Small `Write` adapter that pushes into a shared `Rc<RefCell<Vec<u8>>>`
    /// so a test can read the written bytes after the transport
    /// drops the writer.
    struct SharedWriter(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);
    impl Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn read_line_strips_trailing_newline() {
        let (mut t, _out) = rig(b"hello\nworld\n");
        assert_eq!(t.read_line().unwrap(), "hello");
        assert_eq!(t.read_line().unwrap(), "world");
    }

    #[test]
    fn read_line_strips_crlf() {
        let (mut t, _out) = rig(b"hello\r\nworld\r\n");
        assert_eq!(t.read_line().unwrap(), "hello");
        assert_eq!(t.read_line().unwrap(), "world");
    }

    #[test]
    fn read_line_empty_input_returns_eof() {
        let (mut t, _out) = rig(b"");
        match t.read_line() {
            Err(TransportError::Eof) => {}
            other => panic!("expected EOF, got {:?}", other),
        }
    }

    #[test]
    fn read_line_on_unterminated_final_line_returns_payload() {
        // Last line without `\n` is still a valid request — the
        // server treats it as the final message before peer hangup.
        let (mut t, _out) = rig(b"hello\nfinal");
        assert_eq!(t.read_line().unwrap(), "hello");
        assert_eq!(t.read_line().unwrap(), "final");
        // Next read hits EOF.
        match t.read_line() {
            Err(TransportError::Eof) => {}
            other => panic!("expected EOF after exhausted input, got {:?}", other),
        }
    }

    #[test]
    fn write_line_appends_newline_and_flushes() {
        let (mut t, out) = rig(b"");
        t.write_line(r#"{"id":1,"ok":true,"body":"{}"}"#).unwrap();
        let written = out.borrow().clone();
        assert_eq!(
            String::from_utf8(written).unwrap(),
            "{\"id\":1,\"ok\":true,\"body\":\"{}\"}\n"
        );
    }

    #[test]
    fn round_trip_request_response_pair() {
        // Input has one request line; we read it, then write a
        // response. Validates the contract callers will use.
        let req = r#"{"id":1,"verb":"exit"}"#;
        let mut input = Vec::new();
        input.extend_from_slice(req.as_bytes());
        input.push(b'\n');

        let (mut t, out) = rig(&input);
        let line = t.read_line().unwrap();
        assert_eq!(line, req);
        t.write_line(r#"{"id":1,"ok":true,"body":"bye"}"#).unwrap();
        let written = String::from_utf8(out.borrow().clone()).unwrap();
        assert_eq!(written, "{\"id\":1,\"ok\":true,\"body\":\"bye\"}\n");
    }
}
