use std::path::Path;
use thiserror::Error;

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
