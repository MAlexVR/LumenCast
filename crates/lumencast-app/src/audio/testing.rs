//! Fake audio devices for tests: pipes a test feeds and records what is written.

use crate::audio::io::{AudioIo, Capture, Closer, Playback};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// A pipe that a test feeds and a reader blocks on, until it is closed.
#[derive(Default)]
pub(crate) struct Pipe {
    pub(crate) data: Mutex<VecDeque<u8>>,
    pub(crate) wake: Condvar,
    pub(crate) closed: AtomicBool,
}

impl Pipe {
    pub(crate) fn feed(&self, samples: &[f32]) {
        let mut data = self.data.lock().unwrap();
        data.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
        self.wake.notify_all();
    }

    pub(crate) fn end(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.wake.notify_all();
    }
}

pub(crate) struct FakeCapture(Arc<Pipe>);

impl Read for FakeCapture {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut data = self.0.data.lock().unwrap();
        loop {
            if !data.is_empty() {
                let n = buf.len().min(data.len());
                for slot in buf.iter_mut().take(n) {
                    *slot = data.pop_front().unwrap_or(0);
                }
                return Ok(n);
            }
            if self.0.closed.load(Ordering::SeqCst) {
                return Ok(0);
            }
            data = self.0.wake.wait(data).unwrap();
        }
    }
}

impl Capture for FakeCapture {
    fn closer(&self) -> Closer {
        let pipe = Arc::clone(&self.0);
        Box::new(move || pipe.end())
    }
}

pub(crate) struct FakePlayback(Arc<Mutex<Vec<u8>>>);

impl Write for FakePlayback {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Playback for FakePlayback {}

#[derive(Default)]
pub(crate) struct FakeIo {
    pub(crate) opened: Mutex<Vec<String>>,
    pub(crate) inputs: Mutex<Vec<Arc<Pipe>>>,
    pub(crate) outputs: Mutex<Vec<Arc<Mutex<Vec<u8>>>>>,
    pub(crate) broken: Mutex<Option<&'static str>>,
}

impl FakeIo {
    pub(crate) fn open_input(&self, name: String) -> Result<Box<dyn Capture>, String> {
        self.fail_if(&name)?;
        self.opened.lock().unwrap().push(name);
        let pipe = Arc::new(Pipe::default());
        self.inputs.lock().unwrap().push(Arc::clone(&pipe));
        Ok(Box::new(FakeCapture(pipe)))
    }

    pub(crate) fn open_output(&self, name: String) -> Result<Box<dyn Playback>, String> {
        self.fail_if(&name)?;
        self.opened.lock().unwrap().push(name);
        let buffer = Arc::new(Mutex::new(Vec::new()));
        self.outputs.lock().unwrap().push(Arc::clone(&buffer));
        Ok(Box::new(FakePlayback(buffer)))
    }

    fn fail_if(&self, name: &str) -> Result<(), String> {
        match *self.broken.lock().unwrap() {
            Some(prefix) if name.starts_with(prefix) => Err(format!("{name} is not there")),
            _ => Ok(()),
        }
    }

    pub(crate) fn input(&self, n: usize) -> Arc<Pipe> {
        Arc::clone(&self.inputs.lock().unwrap()[n])
    }

    pub(crate) fn output_len(&self, n: usize) -> usize {
        self.outputs
            .lock()
            .unwrap()
            .get(n)
            .map_or(0, |o| o.lock().unwrap().len())
    }
}

impl AudioIo for FakeIo {
    fn open_source(&self, target: Option<&str>) -> Result<Box<dyn Capture>, String> {
        self.open_input(format!("source:{}", target.unwrap_or("default")))
    }

    fn open_virtual_microphone(&self) -> Result<Box<dyn Playback>, String> {
        self.open_output("virtual microphone".into())
    }

    fn open_virtual_speakers(&self) -> Result<Box<dyn Capture>, String> {
        self.open_input("virtual speakers".into())
    }

    fn open_sink(&self, target: Option<&str>) -> Result<Box<dyn Playback>, String> {
        self.open_output(format!("sink:{}", target.unwrap_or("default")))
    }
}
