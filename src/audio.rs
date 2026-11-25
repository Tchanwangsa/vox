//! The microphone: a cpal input stream that keeps mono samples and a short
//! run of loudness readings for the waveform, and hands back a 16 kHz WAV.

use std::collections::VecDeque;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// macOS microphone permission state from `AVCaptureDevice`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicPermission {
    /// macOS has not asked yet; opening the mic will trigger the system prompt.
    NotDetermined,
    /// Access was denied or restricted in System Settings.
    Denied,
    /// Microphone access is allowed.
    Allowed,
}

#[allow(unexpected_cfgs)] // objc's `sel_impl!` checks a cfg this crate never sets
pub fn microphone_permission() -> MicPermission {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};

    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {
        static AVMediaTypeAudio: *const Object;
    }

    let status: isize = unsafe {
        msg_send![
            class!(AVCaptureDevice),
            authorizationStatusForMediaType: AVMediaTypeAudio
        ]
    };
    match status {
        1 | 2 => MicPermission::Denied,
        3 => MicPermission::Allowed,
        _ => MicPermission::NotDetermined,
    }
}

/// What both engines want: Whisper resamples to this anyway.
pub const RATE: u32 = 16_000;

/// How many loudness readings the waveform shows.
pub const BARS: usize = 30;

/// One reading per this many seconds of audio.
const READING_SECS: f32 = 0.035;

#[derive(Default)]
struct Shared {
    samples: Vec<f32>,
    /// Newest last, each 0..1.
    levels: VecDeque<f32>,
    /// Samples and their squared sum since the last reading.
    pending: (usize, f32),
    /// The loudest sample heard at all; zero means the mic gave silence.
    peak: f32,
}

impl Shared {
    /// One mono sample in; a loudness reading out every `per_reading` samples.
    fn take(&mut self, sample: f32, per_reading: usize) {
        self.samples.push(sample);
        self.peak = self.peak.max(sample.abs());
        self.pending.0 += 1;
        self.pending.1 += sample * sample;
        if self.pending.0 >= per_reading {
            let rms = (self.pending.1 / self.pending.0 as f32).sqrt();
            // Speech sits around 0.02–0.2 RMS; the curve lifts quiet talk.
            self.levels.push_back((rms * 9.0).min(1.0).powf(0.6));
            if self.levels.len() > BARS {
                self.levels.pop_front();
            }
            self.pending = (0, 0.0);
        }
    }
}

/// A handle the waveform reads from while the stream writes.
#[derive(Clone, Default)]
pub struct Levels(Arc<Mutex<Shared>>);

impl Levels {
    /// A made-up stretch of speech, for looking at the waveform without a mic.
    pub fn demo() -> Self {
        let levels = Self::default();
        levels.0.lock().unwrap().levels = (0..BARS)
            .map(|i| {
                let x = i as f32;
                (0.5 + 0.5 * (x * 0.9).sin()) * (0.35 + 0.65 * (0.5 + 0.5 * (x * 0.23 + 1.).sin()))
            })
            .collect();
        levels
    }

    /// The last [`BARS`] readings, oldest first, padded with silence.
    pub fn recent(&self) -> Vec<f32> {
        let shared = self.0.lock().unwrap();
        let mut out = vec![0.0; BARS.saturating_sub(shared.levels.len())];
        out.extend(shared.levels.iter().copied());
        out
    }
}

pub struct Recorder {
    stream: cpal::Stream,
    shared: Levels,
    rate: u32,
}

/// A finished recording.
pub struct Clip {
    /// 16 kHz mono 16-bit PCM WAV.
    pub wav: Vec<u8>,
    pub seconds: f32,
    /// False when every sample was zero: macOS hands a silent stream to an
    /// app without microphone access.
    pub heard_anything: bool,
}

impl Recorder {
    pub fn start() -> Result<Self, String> {
        let device = cpal::default_host()
            .default_input_device()
            .ok_or("No microphone found. Plug one in or pick an input in System Settings → Sound.")?;
        let supported = device
            .default_input_config()
            .map_err(|e| format!("The microphone would not open: {e}"))?;
        let format = supported.sample_format();
        let config = supported.config();
        let channels = config.channels as usize;
        let rate = config.sample_rate;
        let shared = Levels::default();
        let per_reading = ((rate as f32 * READING_SECS) as usize).max(1);

        let failed = |e| eprintln!("[vox] microphone stream: {e}");
        let sink = shared.clone();
        let stream = match format {
            cpal::SampleFormat::F32 => device.build_input_stream(
                config,
                move |data: &[f32], _: &_| {
                    let mut shared = sink.0.lock().unwrap();
                    for frame in data.chunks(channels) {
                        shared.take(frame.iter().sum::<f32>() / channels as f32, per_reading);
                    }
                },
                failed,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                config,
                move |data: &[i16], _: &_| {
                    let mut shared = sink.0.lock().unwrap();
                    for frame in data.chunks(channels) {
                        let sum: f32 = frame.iter().map(|&s| s as f32 / 32_768.0).sum();
                        shared.take(sum / channels as f32, per_reading);
                    }
                },
                failed,
                None,
            ),
            other => return Err(format!("The microphone's format ({other:?}) is not supported.")),
        }
        .map_err(|e| format!("The microphone would not open: {e}"))?;
        stream
            .play()
            .map_err(|e| format!("The microphone would not start: {e}"))?;

        Ok(Self {
            stream,
            shared,
            rate,
        })
    }

    pub fn levels(&self) -> Levels {
        self.shared.clone()
    }

    /// Stop the stream and encode what was said.
    pub fn finish(self) -> Result<Clip, String> {
        drop(self.stream);
        let (samples, peak) = {
            let mut shared = self.shared.0.lock().unwrap();
            (std::mem::take(&mut shared.samples), shared.peak)
        };
        let seconds = samples.len() as f32 / self.rate as f32;
        let wav = encode(&resample(&samples, self.rate, RATE))?;
        Ok(Clip {
            wav,
            seconds,
            heard_anything: peak > 0.0,
        })
    }
}

/// `samples` at `to` Hz. Each output sample averages the input it spans, which
/// low-passes enough for speech going down from 44.1 or 48 kHz.
fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() {
        return samples.to_vec();
    }
    let step = from as f64 / to as f64;
    let len = (samples.len() as f64 / step) as usize;
    (0..len)
        .map(|i| {
            let start = (i as f64 * step) as usize;
            let end = (((i + 1) as f64 * step) as usize).clamp(start + 1, samples.len());
            samples[start..end].iter().sum::<f32>() / (end - start) as f32
        })
        .collect()
}

fn encode(samples: &[f32]) -> Result<Vec<u8>, String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut bytes = Cursor::new(Vec::with_capacity(samples.len() * 2 + 44));
    let mut writer = hound::WavWriter::new(&mut bytes, spec).map_err(|e| e.to_string())?;
    for &sample in samples {
        writer
            .write_sample((sample.clamp(-1.0, 1.0) * 32_767.0) as i16)
            .map_err(|e| e.to_string())?;
    }
    writer.finalize().map_err(|e| e.to_string())?;
    Ok(bytes.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_keeps_the_length_in_time() {
        let second = vec![0.5; 48_000];
        assert_eq!(resample(&second, 48_000, RATE).len(), 16_000);
        assert_eq!(resample(&vec![0.5; 44_100], 44_100, RATE).len(), 16_000);
        assert!(resample(&second, 48_000, RATE).iter().all(|&s| (s - 0.5).abs() < 1e-6));
    }

    #[test]
    fn a_clip_encodes_as_a_readable_wav() {
        let wav = encode(&vec![0.25; 1_600]).unwrap();
        let reader = hound::WavReader::new(Cursor::new(wav)).unwrap();
        assert_eq!(reader.spec().sample_rate, RATE);
        assert_eq!(reader.len(), 1_600);
    }
}
