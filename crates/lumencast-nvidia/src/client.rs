//! The connection to the matte server: sends a shrunk copy of each frame and
//! returns the person matte it answers with.

use crate::protocol::{read_response, write_request};
use lumencast_fx::frame::Frame;
use lumencast_fx::matte::Matte;
use lumencast_fx::stages::resize;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

/// Width of the copy that is sent; the model itself sees 512x288.
pub const SEND_WIDTH: u32 = 512;
/// How long to wait between attempts to reach a server that is not there yet.
const RETRY_AFTER: Duration = Duration::from_millis(500);
const CONNECT_TIMEOUT: Duration = Duration::from_millis(150);
const IO_TIMEOUT: Duration = Duration::from_millis(800);

pub struct MatteClient {
    address: SocketAddr,
    stream: Option<TcpStream>,
    last_attempt: Option<Instant>,
}

/// The size of the copy sent for a frame, keeping its shape.
pub fn send_size(width: u32, height: u32) -> (u32, u32) {
    let scaled = (u64::from(SEND_WIDTH) * u64::from(height)) / u64::from(width.max(1));
    (SEND_WIDTH, (scaled as u32).clamp(16, 2048))
}

impl MatteClient {
    pub fn new(port: u16) -> Self {
        Self {
            address: SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
            stream: None,
            last_attempt: None,
        }
    }

    /// The matte of `frame`, or `None` while the server cannot be reached (it
    /// may still be loading its model); the caller carries on without it.
    pub fn matte(&mut self, frame: &Frame) -> Option<Matte> {
        let (width, height) = send_size(frame.width(), frame.height());
        let small = resize(frame, width, height);
        let stream = self.connected()?;
        let answer = write_request(stream, width, height, small.data())
            .and_then(|()| read_response(stream, width, height));
        match answer {
            Ok(matte) => Some(matte),
            Err(_) => {
                // A broken connection is dropped and tried again later.
                self.stream = None;
                None
            }
        }
    }

    fn connected(&mut self) -> Option<&mut TcpStream> {
        if self.stream.is_none() {
            if self
                .last_attempt
                .is_some_and(|at| at.elapsed() < RETRY_AFTER)
            {
                return None;
            }
            self.last_attempt = Some(Instant::now());
            let stream = TcpStream::connect_timeout(&self.address, CONNECT_TIMEOUT).ok()?;
            stream.set_nodelay(true).ok()?;
            stream.set_read_timeout(Some(IO_TIMEOUT)).ok()?;
            stream.set_write_timeout(Some(IO_TIMEOUT)).ok()?;
            self.stream = Some(stream);
        }
        self.stream.as_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::MAGIC;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    /// A server that answers `rounds` frames with a matte of 200s.
    fn fake_server(rounds: usize) -> (u16, thread::JoinHandle<Vec<(u32, u32)>>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut sizes = Vec::new();
            for _ in 0..rounds {
                let mut header = [0u8; 12];
                socket.read_exact(&mut header).unwrap();
                let width = u32::from_le_bytes(header[4..8].try_into().unwrap());
                let height = u32::from_le_bytes(header[8..12].try_into().unwrap());
                let mut pixels = vec![0u8; width as usize * height as usize * 3];
                socket.read_exact(&mut pixels).unwrap();
                sizes.push((width, height));
                for value in [MAGIC, 0, width, height] {
                    socket.write_all(&value.to_le_bytes()).unwrap();
                }
                socket
                    .write_all(&vec![200u8; width as usize * height as usize])
                    .unwrap();
            }
            sizes
        });
        (port, handle)
    }

    #[test]
    fn the_copy_sent_keeps_the_shape_of_the_frame() {
        assert_eq!(send_size(1280, 720), (512, 288));
        assert_eq!(send_size(640, 480), (512, 384));
        assert_eq!(send_size(1920, 1080), (512, 288));
        assert_eq!(send_size(1, 1_000_000).1, 2048);
    }

    #[test]
    fn frames_go_out_shrunk_and_the_matte_comes_back() {
        let (port, server) = fake_server(2);
        let mut client = MatteClient::new(port);
        let frame = Frame::from_fn(1280, 720, |_, _| [9, 9, 9]);
        for _ in 0..2 {
            let matte = client.matte(&frame).expect("a matte");
            assert_eq!((matte.width(), matte.height()), (512, 288));
            assert_eq!(matte.alpha()[0], 200);
        }
        assert_eq!(server.join().unwrap(), vec![(512, 288), (512, 288)]);
    }

    #[test]
    fn a_server_that_is_not_there_gives_none_and_is_not_hammered() {
        // Port 1 on loopback: nothing listens.
        let mut client = MatteClient::new(1);
        let frame = Frame::from_fn(64, 36, |_, _| [0; 3]);
        assert!(client.matte(&frame).is_none());
        let first = client.last_attempt;
        assert!(client.matte(&frame).is_none());
        assert_eq!(
            client.last_attempt, first,
            "no new attempt inside the pause"
        );
    }

    #[test]
    fn a_connection_that_breaks_is_dropped_for_a_later_retry() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let closer = thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            drop(socket);
        });
        let mut client = MatteClient::new(port);
        let frame = Frame::from_fn(64, 36, |_, _| [0; 3]);
        assert!(client.matte(&frame).is_none());
        assert!(client.stream.is_none());
        closer.join().unwrap();
    }
}
