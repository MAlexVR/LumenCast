//! Where audio comes from and goes to. On Linux that is PipeWire, driven with
//! its `pw-cat` tool through pipes, which needs no library to be linked and
//! lets the program publish a virtual microphone and a virtual output.
//!
//! A virtual device is a `pw-loopback` child: a sink the program plays into and
//! a source (or monitor) others read from. Nodes made by `pw-cat` itself get no
//! ports for a mono layout, so they cannot serve as devices.

use crate::audio::settings::{MICROPHONE_NAME, MICROPHONE_NODE, SPEAKERS_NAME, SPEAKERS_NODE};
use std::io::{self, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Makes a blocked `read` end, from another thread.
pub type Closer = Box<dyn Fn() + Send + Sync>;

/// A stream of audio coming in: 48 kHz, 32-bit float, little endian, interleaved.
pub trait Capture: Read + Send {
    fn closer(&self) -> Closer;
}

/// A stream of audio going out, in the same format.
pub trait Playback: Write + Send {}

pub trait AudioIo: Send + Sync + 'static {
    /// A real microphone, mono; `None` is the system default.
    fn open_source(&self, target: Option<&str>) -> Result<Box<dyn Capture>, String>;
    /// Publishes the virtual microphone; the caller writes mono.
    fn open_virtual_microphone(&self) -> Result<Box<dyn Playback>, String>;
    /// Publishes the virtual output and receives what programs play into it, stereo.
    fn open_virtual_speakers(&self) -> Result<Box<dyn Capture>, String>;
    /// A real output, stereo; `None` is the system default.
    fn open_sink(&self, target: Option<&str>) -> Result<Box<dyn Playback>, String>;
}

/// PipeWire through `pw-cat`.
pub struct PipeWireIo;

fn spawn(arguments: &[String], input: bool, output: bool) -> Result<Child, String> {
    Command::new("pw-cat")
        .args(arguments)
        .stdin(if input { Stdio::piped() } else { Stdio::null() })
        .stdout(if output {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                "pw-cat was not found; install PipeWire's tools (pipewire-utils)".to_owned()
            } else {
                format!("pw-cat could not be started: {error}")
            }
        })
}

fn format(channels: u8) -> Vec<String> {
    [
        "--rate",
        "48000",
        "--channels",
        &channels.to_string(),
        "--format",
        "f32",
        "--raw",
        "--latency",
        "20ms",
    ]
    .iter()
    .map(ToString::to_string)
    .collect()
}

fn with_target(mut base: Vec<String>, target: Option<&str>) -> Vec<String> {
    if let Some(target) = target {
        base.push("--target".into());
        base.push(target.to_owned());
    }
    base
}

/// The stream's file argument: raw samples on the standard input or output.
fn standard_stream(mut arguments: Vec<String>) -> Vec<String> {
    arguments.push("-".into());
    arguments
}

fn properties(class: &str, node: &str, description: &str) -> String {
    format!("{{ media.class={class} node.name={node} node.description=\"{description}\" }}")
}

/// The arguments of a `pw-loopback` that publishes `sink` to programs and `output`
/// to whoever reads it. `output` is described by the caller.
fn loopback_arguments(name: &str, sink: &str, output: &str) -> Vec<String> {
    ["-n", name, "-i", sink, "-o", output]
        .iter()
        .map(ToString::to_string)
        .collect()
}

/// Starts a loopback and waits until the node `ready` has its ports.
fn start_loopback(arguments: &[String], ready: &str) -> Result<Child, String> {
    let mut child = Command::new("pw-loopback")
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                "pw-loopback was not found; install PipeWire's tools (pipewire-utils)".to_owned()
            } else {
                format!("pw-loopback could not be started: {error}")
            }
        })?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            return Err("pw-loopback ended before the virtual device appeared".to_owned());
        }
        if has_ports(ready) {
            return Ok(child);
        }
        thread::sleep(Duration::from_millis(40));
    }
    let _ = child.kill();
    let _ = child.wait();
    Err("the virtual device did not appear in PipeWire".to_owned())
}

/// Whether PipeWire lists ports for the node, in either direction.
fn has_ports(node: &str) -> bool {
    let prefix = format!("{node}:");
    ["-i", "-o"].iter().any(|direction| {
        Command::new("pw-link")
            .arg(direction)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .any(|line| line.trim_start().starts_with(&prefix))
            })
            .unwrap_or(false)
    })
}

fn end(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

struct PipeCapture {
    stdout: ChildStdout,
    child: Arc<Mutex<Child>>,
    /// The loopback that keeps a virtual device published while this reads.
    device: Option<Child>,
}

impl Read for PipeCapture {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stdout.read(buf)
    }
}

impl Capture for PipeCapture {
    fn closer(&self) -> Closer {
        let child = Arc::clone(&self.child);
        Box::new(move || {
            if let Ok(mut child) = child.lock() {
                let _ = child.kill();
            }
        })
    }
}

impl Drop for PipeCapture {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            end(&mut child);
        }
        if let Some(device) = self.device.as_mut() {
            end(device);
        }
    }
}

struct PipePlayback {
    stdin: Option<ChildStdin>,
    child: Child,
    /// The loopback that keeps a virtual device published while this writes.
    device: Option<Child>,
    /// Each incoming sample goes out on both channels.
    duplicate: bool,
}

impl Write for PipePlayback {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Some(stdin) = self.stdin.as_mut() else {
            return Err(io::ErrorKind::BrokenPipe.into());
        };
        if self.duplicate {
            stdin.write_all(&duplicated(buf))?;
            Ok(buf.len())
        } else {
            stdin.write(buf)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.stdin.as_mut() {
            Some(stdin) => stdin.flush(),
            None => Ok(()),
        }
    }
}

impl Playback for PipePlayback {}

impl Drop for PipePlayback {
    fn drop(&mut self) {
        // Closing the input and ending the process removes the virtual node.
        self.stdin.take();
        end(&mut self.child);
        if let Some(device) = self.device.as_mut() {
            end(device);
        }
    }
}

/// Mono f32 samples as interleaved stereo.
fn duplicated(mono: &[u8]) -> Vec<u8> {
    let mut stereo = Vec::with_capacity(mono.len() * 2);
    for sample in mono.chunks_exact(4) {
        stereo.extend_from_slice(sample);
        stereo.extend_from_slice(sample);
    }
    stereo
}

fn capture(arguments: Vec<String>, device: Option<Child>) -> Result<Box<dyn Capture>, String> {
    let mut child = match spawn(&arguments, false, true) {
        Ok(child) => child,
        Err(error) => {
            if let Some(mut device) = device {
                end(&mut device);
            }
            return Err(error);
        }
    };
    let stdout = child.stdout.take().ok_or("pw-cat gave no output")?;
    Ok(Box::new(PipeCapture {
        stdout,
        child: Arc::new(Mutex::new(child)),
        device,
    }))
}

fn playback(
    arguments: Vec<String>,
    device: Option<Child>,
    duplicate: bool,
) -> Result<Box<dyn Playback>, String> {
    let mut child = match spawn(&arguments, true, false) {
        Ok(child) => child,
        Err(error) => {
            if let Some(mut device) = device {
                end(&mut device);
            }
            return Err(error);
        }
    };
    let stdin = child.stdin.take().ok_or("pw-cat took no input")?;
    Ok(Box::new(PipePlayback {
        stdin: Some(stdin),
        child,
        device,
        duplicate,
    }))
}

impl AudioIo for PipeWireIo {
    fn open_source(&self, target: Option<&str>) -> Result<Box<dyn Capture>, String> {
        let mut arguments = vec!["--record".to_owned()];
        arguments.extend(with_target(format(1), target));
        capture(standard_stream(arguments), None)
    }

    fn open_virtual_microphone(&self) -> Result<Box<dyn Playback>, String> {
        let input = format!("{MICROPHONE_NODE}_input");
        let loopback = start_loopback(
            &loopback_arguments(
                MICROPHONE_NODE,
                &properties("Audio/Sink", &input, "LumenCast Microphone input"),
                &properties("Audio/Source", MICROPHONE_NODE, MICROPHONE_NAME),
            ),
            &input,
        )?;
        let mut arguments = vec!["--playback".to_owned()];
        arguments.extend(with_target(format(2), Some(&input)));
        playback(standard_stream(arguments), Some(loopback), true)
    }

    fn open_virtual_speakers(&self) -> Result<Box<dyn Capture>, String> {
        let loopback = start_loopback(
            &loopback_arguments(
                SPEAKERS_NODE,
                &properties("Audio/Sink", SPEAKERS_NODE, SPEAKERS_NAME),
                // Nobody reads this side: the program records the sink's monitor.
                "{ node.autoconnect=false node.passive=true }",
            ),
            SPEAKERS_NODE,
        )?;
        let mut arguments = vec![
            "--record".to_owned(),
            "-P".to_owned(),
            "stream.capture.sink=true".to_owned(),
        ];
        arguments.extend(with_target(format(2), Some(SPEAKERS_NODE)));
        capture(standard_stream(arguments), Some(loopback))
    }

    fn open_sink(&self, target: Option<&str>) -> Result<Box<dyn Playback>, String> {
        let mut arguments = vec!["--playback".to_owned()];
        arguments.extend(with_target(format(2), target));
        playback(standard_stream(arguments), None, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_node_properties_quote_the_description() {
        assert_eq!(
            properties("Audio/Source", "lumencast_microphone", "LumenCast Microphone"),
            "{ media.class=Audio/Source node.name=lumencast_microphone node.description=\"LumenCast Microphone\" }"
        );
    }

    #[test]
    fn a_loopback_names_both_of_its_sides() {
        assert_eq!(
            loopback_arguments("dev", "{ a }", "{ b }"),
            ["-n", "dev", "-i", "{ a }", "-o", "{ b }"]
        );
    }

    #[test]
    fn mono_samples_are_written_on_both_channels() {
        let mono: Vec<u8> = [1.0_f32, -0.5]
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        let stereo: Vec<f32> = duplicated(&mono)
            .chunks_exact(4)
            .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
            .collect();
        assert_eq!(stereo, [1.0, 1.0, -0.5, -0.5]);
    }

    #[test]
    fn the_stream_format_is_48k_float_raw_with_a_short_latency() {
        let arguments = standard_stream(format(2)).join(" ");
        assert_eq!(
            arguments,
            "--rate 48000 --channels 2 --format f32 --raw --latency 20ms -"
        );
        // The target goes before the file argument, where every option parser accepts it.
        let targeted = standard_stream(with_target(format(1), Some("mic"))).join(" ");
        assert!(targeted.ends_with("--target mic -"));
    }

    #[test]
    fn a_target_is_only_added_when_one_is_given() {
        assert_eq!(with_target(vec!["a".into()], None), ["a"]);
        assert_eq!(
            with_target(vec!["a".into()], Some("alsa_input.x")),
            ["a", "--target", "alsa_input.x"]
        );
    }
}
