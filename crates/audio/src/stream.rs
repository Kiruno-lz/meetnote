use crate::AudioError;
use std::collections::VecDeque;
use std::fs::File;
use std::path::{Path, PathBuf};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::{get_codecs, get_probe};

pub const TARGET_SAMPLE_RATE_HZ: u32 = 16_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSourceInfo {
    pub path: PathBuf,
    pub source_sample_rate_hz: u32,
    pub source_channels: usize,
    pub source_frames: Option<u64>,
    pub estimated_output_samples: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioChunk {
    pub sequence: u64,
    pub start_sample: u64,
    pub sample_rate: u32,
    pub samples: Vec<f32>,
    pub is_final: bool,
}

pub struct AudioChunkReader {
    path: PathBuf,
    path_display: String,
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    source_info: AudioSourceInfo,
    resampler: LinearResampler,
    output: VecDeque<f32>,
    chunk_samples: usize,
    sequence: u64,
    output_start_sample: u64,
    packet_sequence: u64,
    decoded_source_frames: u64,
    eof: bool,
}

impl AudioChunkReader {
    pub fn open(path: impl AsRef<Path>, chunk_samples: usize) -> Result<Self, AudioError> {
        if chunk_samples == 0 {
            return Err(AudioError::Decode {
                path: path.as_ref().display().to_string(),
                message: "audio chunk size must be greater than zero".into(),
            });
        }
        let path = path.as_ref().to_path_buf();
        let path_display = path.display().to_string();
        let file = File::open(&path).map_err(|error| AudioError::Decode {
            path: path_display.clone(),
            message: error.to_string(),
        })?;
        let media_source = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
            hint.with_extension(extension);
        }
        let probed = get_probe()
            .format(
                &hint,
                media_source,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(|error| AudioError::Decode {
                path: path_display.clone(),
                message: error.to_string(),
            })?;
        let format = probed.format;
        let track = format.default_track().ok_or_else(|| AudioError::Decode {
            path: path_display.clone(),
            message: "audio has no default track".into(),
        })?;
        let track_id = track.id;
        let source_sample_rate_hz =
            track
                .codec_params
                .sample_rate
                .ok_or_else(|| AudioError::Decode {
                    path: path_display.clone(),
                    message: "audio track has no sample rate".into(),
                })?;
        let source_channels = track
            .codec_params
            .channels
            .map(|channels| channels.count())
            .unwrap_or(0);
        let source_frames = if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"))
        {
            hound::WavReader::open(&path)
                .ok()
                .map(|reader| u64::from(reader.duration()))
                .or(track.codec_params.n_frames)
        } else {
            track.codec_params.n_frames
        };
        let estimated_output_samples = source_frames.map(|frames| {
            frames.saturating_mul(u64::from(TARGET_SAMPLE_RATE_HZ))
                / u64::from(source_sample_rate_hz)
        });
        let decoder = get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|error| AudioError::Decode {
                path: path_display.clone(),
                message: error.to_string(),
            })?;
        let source_info = AudioSourceInfo {
            path: path.clone(),
            source_sample_rate_hz,
            source_channels,
            source_frames,
            estimated_output_samples,
        };

        Ok(Self {
            path,
            path_display,
            format,
            decoder,
            track_id,
            source_info,
            resampler: LinearResampler::new(source_sample_rate_hz, TARGET_SAMPLE_RATE_HZ),
            output: VecDeque::new(),
            chunk_samples,
            sequence: 0,
            output_start_sample: 0,
            packet_sequence: 0,
            decoded_source_frames: 0,
            eof: false,
        })
    }

    pub fn source_info(&self) -> &AudioSourceInfo {
        &self.source_info
    }

    pub fn next_chunk(&mut self) -> Result<Option<AudioChunk>, AudioError> {
        self.next_chunk_cancellable(|| false)
    }

    pub fn next_chunk_cancellable(
        &mut self,
        mut is_cancelled: impl FnMut() -> bool,
    ) -> Result<Option<AudioChunk>, AudioError> {
        while self.output.len() <= self.chunk_samples && !self.eof {
            if is_cancelled() {
                return Err(AudioError::Cancelled {
                    path: self.path_display.clone(),
                    output_sample: self.output_start_sample,
                });
            }
            self.decode_next_packet()?;
        }

        if self.output.is_empty() {
            if self.output_start_sample == 0 {
                return Err(AudioError::Empty {
                    path: self.path_display.clone(),
                });
            }
            return Ok(None);
        }

        let take = self.output.len().min(self.chunk_samples);
        let samples = self.output.drain(..take).collect::<Vec<_>>();
        let start_sample = self.output_start_sample;
        self.output_start_sample = self.output_start_sample.saturating_add(take as u64);
        let is_final = self.eof && self.output.is_empty();
        let chunk = AudioChunk {
            sequence: self.sequence,
            start_sample,
            sample_rate: TARGET_SAMPLE_RATE_HZ,
            samples,
            is_final,
        };
        self.sequence = self.sequence.saturating_add(1);
        Ok(Some(chunk))
    }

    fn decode_next_packet(&mut self) -> Result<(), AudioError> {
        let packet = loop {
            match self.format.next_packet() {
                Ok(packet) if packet.track_id() == self.track_id => break packet,
                Ok(_) => continue,
                Err(SymphoniaError::IoError(error))
                    if error.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    self.eof = true;
                    self.resampler.finish(&mut self.output);
                    return Ok(());
                }
                Err(error) => return Err(self.decode_error(error.to_string())),
            }
        };
        self.packet_sequence = self.packet_sequence.saturating_add(1);
        let decoded = match self.decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(error) => {
                return Err(AudioError::Decode {
                    path: self.path.display().to_string(),
                    message: format!("packet {}: {error}", self.packet_sequence),
                })
            }
        };
        let spec = *decoded.spec();
        if spec.rate != self.source_info.source_sample_rate_hz {
            return Err(self.decode_error(format!(
                "sample rate changed from {} to {}",
                self.source_info.source_sample_rate_hz, spec.rate
            )));
        }
        let channels = spec.channels.count().max(1);
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        let decoded_frames = buffer.len() / channels;
        let frames_to_take = self
            .source_info
            .source_frames
            .map(|total| {
                total
                    .saturating_sub(self.decoded_source_frames)
                    .min(decoded_frames as u64) as usize
            })
            .unwrap_or(decoded_frames);
        let mono = buffer
            .samples()
            .chunks(channels)
            .take(frames_to_take)
            .map(|frame| frame.iter().copied().sum::<f32>() / frame.len() as f32);
        self.resampler.push(mono, &mut self.output);
        self.decoded_source_frames = self
            .decoded_source_frames
            .saturating_add(frames_to_take as u64);
        Ok(())
    }

    fn decode_error(&self, message: String) -> AudioError {
        AudioError::Decode {
            path: self.path.display().to_string(),
            message: format!("packet {}: {message}", self.packet_sequence),
        }
    }
}

struct LinearResampler {
    source_rate: u32,
    target_rate: u32,
    input: VecDeque<f32>,
    input_base: u64,
    input_seen: u64,
    next_output: u64,
    finished: bool,
}

impl LinearResampler {
    fn new(source_rate: u32, target_rate: u32) -> Self {
        Self {
            source_rate,
            target_rate,
            input: VecDeque::new(),
            input_base: 0,
            input_seen: 0,
            next_output: 0,
            finished: false,
        }
    }

    fn push(&mut self, samples: impl IntoIterator<Item = f32>, output: &mut VecDeque<f32>) {
        let before = self.input.len();
        self.input.extend(samples);
        self.input_seen = self
            .input_seen
            .saturating_add((self.input.len() - before) as u64);
        self.drain_available(output);
    }

    fn finish(&mut self, output: &mut VecDeque<f32>) {
        self.finished = true;
        self.drain_available(output);
    }

    fn drain_available(&mut self, output: &mut VecDeque<f32>) {
        if self.input_seen == 0 {
            return;
        }
        let output_len = if self.input_seen < 2 {
            if self.finished {
                self.input_seen
            } else {
                0
            }
        } else {
            self.input_seen.saturating_mul(u64::from(self.target_rate))
                / u64::from(self.source_rate)
        };

        while self.next_output < output_len {
            let position = u128::from(self.next_output) * u128::from(self.source_rate);
            let left = (position / u128::from(self.target_rate)) as u64;
            let remainder = (position % u128::from(self.target_rate)) as u32;
            if left >= self.input_seen {
                break;
            }
            let needs_right = remainder != 0;
            if needs_right && left.saturating_add(1) >= self.input_seen && !self.finished {
                break;
            }
            let left_value = self.sample(left);
            let right_index = left.saturating_add(1).min(self.input_seen - 1);
            let right_value = self.sample(right_index);
            let fraction = remainder as f32 / self.target_rate as f32;
            output.push_back(left_value * (1.0 - fraction) + right_value * fraction);
            self.next_output = self.next_output.saturating_add(1);
        }

        let next_position = u128::from(self.next_output) * u128::from(self.source_rate);
        let keep_from = (next_position / u128::from(self.target_rate)) as u64;
        while self.input_base < keep_from && self.input.len() > 1 {
            self.input.pop_front();
            self.input_base = self.input_base.saturating_add(1);
        }
    }

    fn sample(&self, index: u64) -> f32 {
        self.input[(index - self.input_base) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_boundaries_do_not_change_linear_resampling() {
        let input = (0..441)
            .map(|value| value as f32 / 441.0)
            .collect::<Vec<_>>();
        let mut resampler = LinearResampler::new(44_100, TARGET_SAMPLE_RATE_HZ);
        let mut output = VecDeque::new();
        for packet in input.chunks(17) {
            resampler.push(packet.iter().copied(), &mut output);
        }
        resampler.finish(&mut output);

        let expected = crate::resample_linear(&input, 44_100, TARGET_SAMPLE_RATE_HZ);
        assert_eq!(output.len(), expected.len());
        for (actual, expected) in output.iter().zip(expected) {
            assert!((actual - expected).abs() < 1e-6);
        }
    }

    #[test]
    fn chunk_reader_matches_whole_file_wav_samples_and_offsets() {
        let path = std::env::temp_dir().join(format!(
            "meetnote-audio-stream-{}-{}.wav",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut writer = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 2,
                sample_rate: 44_100,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for frame in 0..12_345i32 {
            writer.write_sample(((frame * 17) % 20_000) as i16).unwrap();
            writer
                .write_sample(-(((frame * 11) % 12_000) as i16))
                .unwrap();
        }
        writer.finalize().unwrap();

        let whole = crate::read_wav_mono_16k(&path).unwrap();
        let mut reader = AudioChunkReader::open(&path, 137).unwrap();
        let mut streamed = Vec::new();
        let mut sequence = 0;
        let mut final_chunks = 0;
        while let Some(chunk) = reader.next_chunk().unwrap() {
            assert_eq!(chunk.sequence, sequence);
            assert_eq!(chunk.start_sample, streamed.len() as u64);
            assert!(chunk.samples.len() <= 137);
            if chunk.is_final {
                final_chunks += 1;
            }
            streamed.extend(chunk.samples);
            sequence += 1;
        }
        assert_eq!(final_chunks, 1);
        assert_eq!(streamed.len(), whole.samples.len());
        for (streamed, whole) in streamed.iter().zip(whole.samples.iter()) {
            assert!((streamed - whole).abs() < 1e-6);
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn cancellation_is_checked_before_decoding_the_next_chunk() {
        let path = std::env::temp_dir().join(format!(
            "meetnote-audio-cancel-{}-{}.wav",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut writer = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate: TARGET_SAMPLE_RATE_HZ,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        writer.write_sample(0i16).unwrap();
        writer.finalize().unwrap();
        let mut reader = AudioChunkReader::open(&path, 1).unwrap();

        let error = reader.next_chunk_cancellable(|| true).unwrap_err();

        assert!(matches!(
            error,
            AudioError::Cancelled {
                output_sample: 0,
                ..
            }
        ));
        std::fs::remove_file(path).unwrap();
    }
}
