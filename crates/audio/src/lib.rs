use std::fs::File;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::{get_codecs, get_probe};
use thiserror::Error;

mod stream;
mod window;

pub use stream::{AudioChunk, AudioChunkReader, AudioSourceInfo, TARGET_SAMPLE_RATE_HZ};
pub use window::{
    AudioWindow, AudioWindowBounds, AudioWindowPlan, ProcessingWindowPlanner, WindowPlanError,
};

#[derive(Debug, Clone, PartialEq)]
pub struct MonoPcm16k {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("failed to open WAV file {path}: {source}")]
    Open { path: String, source: hound::Error },
    #[error("unsupported WAV format in {path}: expected integer PCM")]
    UnsupportedFormat { path: String },
    #[error("failed to read WAV samples from {path}: {source}")]
    Read { path: String, source: hound::Error },
    #[error("audio has no samples: {path}")]
    Empty { path: String },
    #[error("failed to decode audio file {path}: {message}")]
    Decode { path: String, message: String },
    #[error("audio decoding was cancelled at output sample {output_sample}: {path}")]
    Cancelled { path: String, output_sample: u64 },
}

pub fn read_audio_mono_16k(path: impl AsRef<Path>) -> Result<MonoPcm16k, AudioError> {
    let path = path.as_ref();
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"))
    {
        return read_wav_mono_16k(path);
    }
    read_with_symphonia(path)
}

pub fn read_wav_mono_16k(path: impl AsRef<Path>) -> Result<MonoPcm16k, AudioError> {
    let path = path.as_ref();
    let path_display = path.display().to_string();
    let mut reader = hound::WavReader::open(path).map_err(|source| AudioError::Open {
        path: path_display.clone(),
        source,
    })?;
    let spec = reader.spec();
    if spec.sample_format != hound::SampleFormat::Int {
        return Err(AudioError::UnsupportedFormat { path: path_display });
    }

    let channels = usize::from(spec.channels.max(1));
    let max_amplitude = 2_f32.powi(i32::from(spec.bits_per_sample.saturating_sub(1)));
    let samples: Vec<f32> = reader
        .samples::<i32>()
        .map(|sample| {
            sample
                .map(|value| value as f32 / max_amplitude)
                .map_err(|source| AudioError::Read {
                    path: path_display.clone(),
                    source,
                })
        })
        .collect::<Result<_, _>>()?;
    if samples.is_empty() {
        return Err(AudioError::Empty { path: path_display });
    }

    let mono = samples
        .chunks(channels)
        .map(|frame| frame.iter().copied().sum::<f32>() / frame.len() as f32)
        .collect::<Vec<_>>();
    let resampled = resample_linear(&mono, spec.sample_rate, 16_000);

    Ok(MonoPcm16k {
        samples: resampled,
        sample_rate: 16_000,
    })
}

fn resample_linear(samples: &[f32], source_rate: u32, target_rate: u32) -> Vec<f32> {
    if source_rate == target_rate || samples.len() < 2 {
        return samples.to_vec();
    }

    let output_len =
        ((samples.len() as u64 * u64::from(target_rate)) / u64::from(source_rate)) as usize;
    (0..output_len)
        .map(|index| {
            let source_position = index as f64 * f64::from(source_rate) / f64::from(target_rate);
            let left = source_position.floor() as usize;
            let right = (left + 1).min(samples.len() - 1);
            let fraction = source_position - left as f64;
            samples[left] * (1.0 - fraction as f32) + samples[right] * fraction as f32
        })
        .collect()
}

fn read_with_symphonia(path: &Path) -> Result<MonoPcm16k, AudioError> {
    let path_display = path.display().to_string();
    let file = File::open(path).map_err(|error| AudioError::Decode {
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
    let mut format = probed.format;
    let track = format.default_track().ok_or_else(|| AudioError::Decode {
        path: path_display.clone(),
        message: "audio has no default track".into(),
    })?;
    let track_id = track.id;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| AudioError::Decode {
            path: path_display.clone(),
            message: "audio track has no sample rate".into(),
        })?;
    let mut decoder = get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|error| AudioError::Decode {
            path: path_display.clone(),
            message: error.to_string(),
        })?;
    let mut mono = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(error) => {
                return Err(AudioError::Decode {
                    path: path_display.clone(),
                    message: error.to_string(),
                })
            }
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder
            .decode(&packet)
            .map_err(|error| AudioError::Decode {
                path: path_display.clone(),
                message: error.to_string(),
            })?;
        let spec = *decoded.spec();
        let channels = spec.channels.count().max(1);
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        mono.extend(
            buffer
                .samples()
                .chunks(channels)
                .map(|frame| frame.iter().copied().sum::<f32>() / frame.len() as f32),
        );
    }

    if mono.is_empty() {
        return Err(AudioError::Empty { path: path_display });
    }
    Ok(MonoPcm16k {
        samples: resample_linear(&mono, sample_rate, 16_000),
        sample_rate: 16_000,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resamples_a_short_signal_linearly() {
        let result = resample_linear(&[0.0, 1.0], 8_000, 16_000);

        assert_eq!(result.len(), 4);
        assert!((result[1] - 0.5).abs() < 0.001);
    }
}
