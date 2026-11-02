//! Carrying audio from where it is read to where it is written, through the
//! effect chain. Kept apart from any real device so it can be tested.

use lumencast_audio::chain::{AudioChain, ChainSettings};
use lumencast_audio::meter::{measure, Level};
use lumencast_audio::noise::FRAME;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};

/// Frames of audio handled at a time (10 ms at 48 kHz, what RNNoise takes).
pub const BLOCK: usize = FRAME;

/// Reads interleaved 32-bit float blocks of `channels` channels, runs each
/// channel through its own chain, and writes the result in the same layout.
/// Returns when the input ends. `settings` is read on every block, so changes
/// apply at once without restarting. `levels` receives the input and output
/// level of every block (of the loudest channel).
pub fn run(
    input: &mut dyn Read,
    output: &mut dyn Write,
    channels: usize,
    settings: &Arc<Mutex<ChainSettings>>,
    levels: &mut dyn FnMut(Level, Level),
) -> io::Result<()> {
    let initial = settings.lock().map(|s| *s).unwrap_or_default();
    let mut chains: Vec<AudioChain> = (0..channels).map(|_| AudioChain::new(initial)).collect();
    let mut applied = initial;
    let mut bytes = vec![0u8; BLOCK * channels * 4];
    loop {
        match input.read_exact(&mut bytes) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(error),
        }
        if let Ok(wanted) = settings.try_lock().map(|s| *s) {
            if wanted != applied {
                chains.iter_mut().for_each(|chain| chain.update(wanted));
                applied = wanted;
            }
        }
        // Split the interleaved samples into one signal per channel.
        let mut signals: Vec<Vec<f32>> = vec![Vec::with_capacity(BLOCK); channels];
        for (n, sample) in bytes.chunks_exact(4).enumerate() {
            signals[n % channels].push(f32::from_le_bytes([
                sample[0], sample[1], sample[2], sample[3],
            ]));
        }
        let heard = signals.iter().map(|s| measure(s)).fold(None, loudest);
        let cleaned: Vec<Vec<f32>> = signals
            .iter()
            .zip(chains.iter_mut())
            .map(|(signal, chain)| chain.process(signal))
            .collect();
        let sent = cleaned.iter().map(|s| measure(s)).fold(None, loudest);
        let mut out = Vec::with_capacity(bytes.len());
        for n in 0..BLOCK {
            for channel in &cleaned {
                out.extend_from_slice(&channel[n].to_le_bytes());
            }
        }
        output.write_all(&out)?;
        if let (Some(heard), Some(sent)) = (heard, sent) {
            levels(heard, sent);
        }
    }
}

fn loudest(best: Option<Level>, next: Level) -> Option<Level> {
    Some(match best {
        Some(best) if best.rms_db >= next.rms_db => best,
        _ => next,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn bytes(samples: &[f32]) -> Vec<u8> {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    fn floats(bytes: &[u8]) -> Vec<f32> {
        bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect()
    }

    fn settings(chain: ChainSettings) -> Arc<Mutex<ChainSettings>> {
        Arc::new(Mutex::new(chain))
    }

    #[test]
    fn with_no_effect_the_audio_comes_out_as_it_went_in() {
        let input: Vec<f32> = (0..BLOCK * 3)
            .map(|n| (n as f32 * 0.01).sin() * 0.3)
            .collect();
        let mut output = Vec::new();
        let mut blocks = 0;
        run(
            &mut Cursor::new(bytes(&input)),
            &mut output,
            1,
            &settings(ChainSettings::default()),
            &mut |_, _| blocks += 1,
        )
        .unwrap();
        assert_eq!(floats(&output), input);
        assert_eq!(blocks, 3);
    }

    #[test]
    fn stereo_is_processed_per_channel_and_stays_interleaved() {
        // Left loud, right silent: after noise removal off, both stay as they were.
        let mut input = Vec::new();
        for n in 0..BLOCK {
            input.push((n as f32 * 0.05).sin() * 0.4);
            input.push(0.0);
        }
        let mut output = Vec::new();
        run(
            &mut Cursor::new(bytes(&input)),
            &mut output,
            2,
            &settings(ChainSettings::default()),
            &mut |_, _| {},
        )
        .unwrap();
        assert_eq!(floats(&output), input);
    }

    #[test]
    fn a_half_block_at_the_end_is_dropped_and_the_input_ending_is_not_an_error() {
        let mut input = bytes(&vec![0.1; BLOCK]);
        input.extend(bytes(&vec![0.1; BLOCK / 2]));
        let mut output = Vec::new();
        run(
            &mut Cursor::new(input),
            &mut output,
            1,
            &settings(ChainSettings::default()),
            &mut |_, _| {},
        )
        .unwrap();
        assert_eq!(output.len(), BLOCK * 4);
    }

    #[test]
    fn the_levels_report_what_went_in_and_what_came_out() {
        let studio = ChainSettings {
            studio_voice: Some(lumencast_audio::voice::MicProfile::Headset),
            ..ChainSettings::default()
        };
        let input: Vec<f32> = (0..BLOCK * 20)
            .map(|n| (n as f32 * 0.13).sin() * 0.05)
            .collect();
        let mut last = None;
        run(
            &mut Cursor::new(bytes(&input)),
            &mut Vec::new(),
            1,
            &settings(studio),
            &mut |heard, sent| last = Some((heard, sent)),
        )
        .unwrap();
        let (heard, sent) = last.unwrap();
        assert!(heard.rms_db < -20.0 && heard.rms_db > -40.0, "{heard:?}");
        assert!(
            sent.rms_db > heard.rms_db,
            "the studio voice lifts a quiet voice: {sent:?}"
        );
    }

    #[test]
    fn changing_the_settings_while_it_runs_takes_effect_without_restarting() {
        struct Feed {
            blocks: usize,
            handle: Arc<Mutex<ChainSettings>>,
        }
        impl Read for Feed {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.blocks == 0 {
                    return Ok(0);
                }
                self.blocks -= 1;
                if self.blocks == 10 {
                    // Half way through, the studio voice is switched on.
                    self.handle.lock().unwrap().studio_voice =
                        Some(lumencast_audio::voice::MicProfile::Headset);
                }
                let block = bytes(&vec![0.03; buf.len() / 4]);
                buf.copy_from_slice(&block);
                Ok(buf.len())
            }
        }
        let shared = settings(ChainSettings::default());
        let mut seen = Vec::new();
        run(
            &mut Feed {
                blocks: 20,
                handle: Arc::clone(&shared),
            },
            &mut Vec::new(),
            1,
            &shared,
            &mut |_, sent| seen.push(sent.rms_db),
        )
        .unwrap();
        assert!(
            (seen[0] - seen[1]).abs() < 0.01,
            "the first blocks are untouched"
        );
        assert!(
            (seen[19] - seen[0]).abs() > 1.0,
            "the last ones went through the studio voice"
        );
    }
}
