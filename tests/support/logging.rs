use std::{
    io,
    sync::{Arc, Mutex},
};

use serde_json::Value;
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone, Default)]
pub struct SharedWriter(Arc<Mutex<Vec<u8>>>);

impl SharedWriter {
    pub fn snapshot(&self) -> io::Result<String> {
        let bytes = self
            .0
            .lock()
            .map_err(|_| io::Error::other("log writer lock poisoned"))?
            .clone();
        String::from_utf8(bytes).map_err(io::Error::other)
    }

    pub fn parsed_lines(&self) -> io::Result<Vec<Value>> {
        self.snapshot()?
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).map_err(io::Error::other))
            .collect()
    }
}

impl<'a> MakeWriter<'a> for SharedWriter {
    type Writer = SharedWriterGuard;

    fn make_writer(&'a self) -> Self::Writer {
        SharedWriterGuard(self.0.clone())
    }
}

pub struct SharedWriterGuard(Arc<Mutex<Vec<u8>>>);

impl io::Write for SharedWriterGuard {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("log writer lock poisoned"))?
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
