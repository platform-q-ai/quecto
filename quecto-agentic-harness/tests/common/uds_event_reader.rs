//! Stateful test-client framing: polling must retain read-ahead and partial frames.
use std::io::{self, BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::time::Duration;

#[derive(Debug)]
pub struct EventReader {
    reader: BufReader<UnixStream>,
    pending: Vec<u8>,
}

impl EventReader {
    pub fn new(stream: UnixStream) -> io::Result<Self> {
        stream.set_read_timeout(Some(Duration::from_millis(50)))?;
        Ok(Self {
            reader: BufReader::new(stream),
            pending: Vec::new(),
        })
    }

    pub fn poll(&mut self) -> io::Result<Option<String>> {
        match self.reader.read_until(b'\n', &mut self.pending) {
            Ok(0) if self.pending.is_empty() => Ok(None),
            Ok(_) if self.pending.last() == Some(&b'\n') => {
                let bytes = std::mem::take(&mut self.pending);
                String::from_utf8(bytes)
                    .map(|line| Some(line.trim_end().to_owned()))
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
            }
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "incomplete UDS event",
            )),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
}
