//! The bytes of an H264 camera frame, without any decoder: where the stream
//! is inside a frame, and whether it holds a keyframe.

/// Length of the header some cameras put in front of every frame.
const VENDOR_HEADER: usize = 64;

/// The Annex B stream inside a camera frame.
///
/// Standard cameras send the stream as it is. The Samsung VD USB HDCAM (and,
/// by its descriptors, cameras of the same chip) sends each frame as a 64-byte
/// header, a `width` x `height` plane of 8-bit samples (a monochrome copy of
/// the picture, not part of the stream) and then the stream itself, which only
/// the first frame opens with its parameter sets and keyframe. A decoder fed
/// the whole frame never finds the stream. The size of the plane is written in
/// the header (two little-endian 16-bit numbers at bytes 6 and 8), and it is
/// the size of the stream, not necessarily the one that was asked for: the
/// camera answers 640x480 to any larger request. The prefix is recognised by
/// the stream starting exactly where that plane ends; anything else is left
/// alone. A frame with that header but nothing after the plane, or cut short
/// inside it (the camera sends such frames now and then), gives an empty slice.
pub fn elementary_stream(frame: &[u8]) -> &[u8] {
    const START_CODE: [u8; 4] = [0, 0, 0, 1];
    // A stream that begins with a start code is not a framed one.
    if frame.starts_with(&[0, 0, 1]) || frame.starts_with(&[0, 0, 0]) {
        return frame;
    }
    let number = |at: usize| {
        frame
            .get(at..at + 2)
            .map_or(0, |b| usize::from(u16::from_le_bytes([b[0], b[1]])))
    };
    let (width, height) = (number(6), number(8));
    if !(16..=8192).contains(&width) || !(16..=8192).contains(&height) {
        return frame;
    }
    match frame.get(VENDOR_HEADER + width * height..) {
        Some(rest) if rest.starts_with(&START_CODE) => rest,
        Some(rest) if rest.is_empty() => rest,
        // The plane itself is cut short: the frame was lost on the way.
        None => &[],
        _ => frame,
    }
}

/// Whether a camera frame holds what a decoder needs to start: the parameter
/// sets or a keyframe (NAL unit types 7 and 5). A frame that is not a stream at
/// all (cut short, or an empty one) has none.
pub fn has_keyframe(frame: &[u8]) -> bool {
    let stream = elementary_stream(frame);
    stream
        .windows(4)
        .any(|w| w[..3] == [0, 0, 1] && matches!(w[3] & 0x1f, 5 | 7))
}

/// What to do with a frame of a stream that cannot be decoded before its first
/// keyframe, and that the camera only sends at the start (and then after a
/// long time, or never): hold back what comes before it, and when it does not
/// come, start the stream again, which makes the camera send it. After a few
/// tries the frames are let through as they are.
#[derive(Debug)]
pub struct KeyframeGate {
    seen: bool,
    waiting: u32,
    /// Restarts still allowed.
    left: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Hand the frame on.
    Deliver,
    /// Keep it back: no keyframe has come yet.
    Hold,
    /// Start the camera's stream again and keep this frame back.
    Restart,
}

/// Frames to wait for the keyframe before starting the stream again: about
/// 0.8 s at 30 fps, longer than the camera takes to send it when it will.
pub const WAIT_FRAMES: u32 = 24;
pub const MAX_RESTARTS: u32 = 3;

impl Default for KeyframeGate {
    fn default() -> Self {
        Self::new(MAX_RESTARTS)
    }
}

impl KeyframeGate {
    /// A gate that may ask for `restarts` restarts before it lets frames through.
    pub fn new(restarts: u32) -> Self {
        Self {
            seen: false,
            waiting: 0,
            left: restarts,
        }
    }

    pub fn judge(&mut self, keyframe: bool) -> Verdict {
        if self.seen {
            return Verdict::Deliver;
        }
        if keyframe {
            self.seen = true;
            return Verdict::Deliver;
        }
        if self.left == 0 {
            return Verdict::Deliver;
        }
        self.waiting += 1;
        if self.waiting >= WAIT_FRAMES {
            self.waiting = 0;
            self.left -= 1;
            Verdict::Restart
        } else {
            Verdict::Hold
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame as the Samsung sends it: header carrying the plane size, plane, stream.
    fn camera_frame(width: u16, height: u16, stream: &[u8]) -> Vec<u8> {
        let mut frame = vec![0xAA; VENDOR_HEADER + usize::from(width) * usize::from(height)];
        frame[6..8].copy_from_slice(&width.to_le_bytes());
        frame[8..10].copy_from_slice(&height.to_le_bytes());
        frame.extend_from_slice(stream);
        frame
    }

    #[test]
    fn the_stream_is_found_after_the_vendor_header_and_plane() {
        let stream = [0, 0, 0, 1, 0x67, 1, 2, 3];
        assert_eq!(elementary_stream(&camera_frame(16, 16, &stream)), stream);
        // The size comes from the header, whatever size was asked for.
        assert_eq!(elementary_stream(&camera_frame(640, 480, &stream)), stream);
        // An empty frame (header and plane only) has no stream to give.
        assert!(elementary_stream(&camera_frame(64, 48, &[])).is_empty());
        // So is one cut short inside its plane.
        let mut cut = camera_frame(64, 48, &[]);
        cut.truncate(VENDOR_HEADER + 64 * 48 - 32);
        assert!(elementary_stream(&cut).is_empty());
    }

    #[test]
    fn a_frame_that_is_already_a_stream_is_left_alone() {
        let frame = [0, 0, 0, 1, 0x65, 9, 9, 9];
        assert_eq!(elementary_stream(&frame), frame);
        let short = [1u8, 2, 3];
        assert_eq!(elementary_stream(&short), short);
        // A header with no start code where its plane ends is not the prefix.
        let mut other = camera_frame(16, 16, &[7, 7, 7, 7, 7, 7]);
        other[6] = 15;
        assert_eq!(elementary_stream(&other), other.as_slice());
    }

    #[test]
    fn a_keyframe_is_found_in_a_framed_or_a_plain_stream_and_nowhere_else() {
        let framed = camera_frame(16, 16, &[0, 0, 0, 1, 0x67, 1, 2, 0, 0, 0, 1, 0x65, 9]);
        assert!(has_keyframe(&framed));
        assert!(has_keyframe(&[0, 0, 0, 1, 0x65, 1, 2, 3]));
        // A slice that is not a keyframe, an empty frame, a cut one and noise.
        assert!(!has_keyframe(&camera_frame(
            16,
            16,
            &[0, 0, 0, 1, 0x41, 1, 2, 3]
        )));
        assert!(!has_keyframe(&camera_frame(16, 16, &[])));
        let mut cut = camera_frame(16, 16, &[]);
        cut.truncate(VENDOR_HEADER + 100);
        assert!(!has_keyframe(&cut));
        assert!(!has_keyframe(&[1, 2, 3, 4, 5]));
    }

    #[test]
    fn the_gate_holds_until_the_keyframe_and_then_never_holds_again() {
        let mut gate = KeyframeGate::default();
        assert_eq!(gate.judge(false), Verdict::Hold);
        assert_eq!(gate.judge(true), Verdict::Deliver);
        for _ in 0..100 {
            assert_eq!(gate.judge(false), Verdict::Deliver);
        }
    }

    #[test]
    fn a_keyframe_that_never_comes_restarts_the_stream_a_few_times_and_then_gives_up() {
        let mut gate = KeyframeGate::default();
        for restart in 1..=MAX_RESTARTS {
            for _ in 1..WAIT_FRAMES {
                assert_eq!(gate.judge(false), Verdict::Hold);
            }
            assert_eq!(gate.judge(false), Verdict::Restart, "restart {restart}");
        }
        // Out of tries: the frames go through as they are.
        assert_eq!(gate.judge(false), Verdict::Deliver);
    }

    #[test]
    fn a_gate_made_with_no_restarts_left_lets_every_frame_through() {
        let mut gate = KeyframeGate::new(0);
        assert_eq!(gate.judge(false), Verdict::Deliver);
        let mut one = KeyframeGate::new(1);
        for _ in 1..WAIT_FRAMES {
            assert_eq!(one.judge(false), Verdict::Hold);
        }
        assert_eq!(one.judge(false), Verdict::Restart);
        assert_eq!(one.judge(false), Verdict::Deliver);
    }

    #[test]
    fn a_keyframe_after_a_restart_opens_the_gate() {
        let mut gate = KeyframeGate::default();
        for _ in 0..WAIT_FRAMES {
            gate.judge(false);
        }
        assert_eq!(gate.judge(true), Verdict::Deliver);
        assert_eq!(gate.judge(false), Verdict::Deliver);
    }
}
