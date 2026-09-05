use crate::{AudioChunk, TARGET_SAMPLE_RATE_HZ};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioWindowPlan {
    pub core_samples: u64,
    pub context_samples: u64,
}

impl AudioWindowPlan {
    pub fn new(core_samples: u64, context_samples: u64) -> Result<Self, WindowPlanError> {
        if core_samples == 0 {
            return Err(WindowPlanError::InvalidPlan(
                "window core must contain at least one sample".into(),
            ));
        }
        Ok(Self {
            core_samples,
            context_samples,
        })
    }

    pub fn bounds_for_total(self, total_samples: u64) -> impl Iterator<Item = AudioWindowBounds> {
        let window_count = total_samples.div_ceil(self.core_samples);
        (0..window_count).map(move |sequence| {
            let core_start = sequence.saturating_mul(self.core_samples);
            let core_end = core_start
                .saturating_add(self.core_samples)
                .min(total_samples);
            AudioWindowBounds {
                sequence,
                global_start_sample: core_start.saturating_sub(self.context_samples),
                global_end_sample: core_end
                    .saturating_add(self.context_samples)
                    .min(total_samples),
                valid_core_start_sample: core_start,
                valid_core_end_sample: core_end,
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioWindowBounds {
    pub sequence: u64,
    pub global_start_sample: u64,
    pub global_end_sample: u64,
    pub valid_core_start_sample: u64,
    pub valid_core_end_sample: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioWindow {
    pub sequence: u64,
    pub sample_rate: u32,
    pub samples: Arc<[f32]>,
    pub global_start_sample: u64,
    pub global_end_sample: u64,
    pub valid_core_start_sample: u64,
    pub valid_core_end_sample: u64,
    pub source_total_samples: Option<u64>,
    pub is_final: bool,
}

impl AudioWindow {
    pub fn local_core_range(&self) -> std::ops::Range<u64> {
        (self.valid_core_start_sample - self.global_start_sample)
            ..(self.valid_core_end_sample - self.global_start_sample)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WindowPlanError {
    #[error("invalid processing window plan: {0}")]
    InvalidPlan(String),
    #[error("audio chunk sequence mismatch: expected {expected}, found {actual}")]
    Sequence { expected: u64, actual: u64 },
    #[error("audio chunk offset mismatch: expected {expected}, found {actual}")]
    Offset { expected: u64, actual: u64 },
    #[error("audio chunk sample rate must be 16000 Hz, found {0}")]
    SampleRate(u32),
    #[error("received audio after the final chunk")]
    AfterFinal,
    #[error("audio stream ended without a final chunk")]
    MissingFinal,
}

pub struct ProcessingWindowPlanner {
    plan: AudioWindowPlan,
    buffer: Vec<f32>,
    buffer_start: u64,
    available_end: u64,
    next_chunk_sequence: u64,
    next_window_sequence: u64,
    next_core_start: u64,
    source_total_samples: Option<u64>,
    finished: bool,
}

impl ProcessingWindowPlanner {
    pub fn new(plan: AudioWindowPlan) -> Self {
        Self {
            plan,
            buffer: Vec::new(),
            buffer_start: 0,
            available_end: 0,
            next_chunk_sequence: 0,
            next_window_sequence: 0,
            next_core_start: 0,
            source_total_samples: None,
            finished: false,
        }
    }

    pub fn push(&mut self, chunk: AudioChunk) -> Result<Vec<AudioWindow>, WindowPlanError> {
        if self.finished {
            return Err(WindowPlanError::AfterFinal);
        }
        if chunk.sequence != self.next_chunk_sequence {
            return Err(WindowPlanError::Sequence {
                expected: self.next_chunk_sequence,
                actual: chunk.sequence,
            });
        }
        if chunk.start_sample != self.available_end {
            return Err(WindowPlanError::Offset {
                expected: self.available_end,
                actual: chunk.start_sample,
            });
        }
        if chunk.sample_rate != TARGET_SAMPLE_RATE_HZ {
            return Err(WindowPlanError::SampleRate(chunk.sample_rate));
        }
        self.next_chunk_sequence = self.next_chunk_sequence.saturating_add(1);
        self.available_end = self
            .available_end
            .saturating_add(chunk.samples.len() as u64);
        self.buffer.extend(chunk.samples);
        if chunk.is_final {
            self.finished = true;
            self.source_total_samples = Some(self.available_end);
        }
        self.drain_ready()
    }

    pub fn finish(self) -> Result<(), WindowPlanError> {
        if self.finished {
            Ok(())
        } else {
            Err(WindowPlanError::MissingFinal)
        }
    }

    fn drain_ready(&mut self) -> Result<Vec<AudioWindow>, WindowPlanError> {
        let mut windows = Vec::new();
        while self.next_core_start < self.available_end {
            let nominal_core_end = self.next_core_start.saturating_add(self.plan.core_samples);
            let required_end = nominal_core_end.saturating_add(self.plan.context_samples);
            if !self.finished && self.available_end < required_end {
                break;
            }
            let core_end = nominal_core_end.min(self.available_end);
            let audio_start = self
                .next_core_start
                .saturating_sub(self.plan.context_samples);
            let audio_end = core_end
                .saturating_add(self.plan.context_samples)
                .min(self.available_end);
            let start = usize::try_from(audio_start - self.buffer_start)
                .map_err(|_| WindowPlanError::InvalidPlan("window start overflowed".into()))?;
            let end = usize::try_from(audio_end - self.buffer_start)
                .map_err(|_| WindowPlanError::InvalidPlan("window end overflowed".into()))?;
            let samples = Arc::<[f32]>::from(self.buffer[start..end].to_vec());
            let is_final = self.finished && core_end == self.available_end;
            windows.push(AudioWindow {
                sequence: self.next_window_sequence,
                sample_rate: TARGET_SAMPLE_RATE_HZ,
                samples,
                global_start_sample: audio_start,
                global_end_sample: audio_end,
                valid_core_start_sample: self.next_core_start,
                valid_core_end_sample: core_end,
                source_total_samples: self.source_total_samples,
                is_final,
            });
            self.next_window_sequence = self.next_window_sequence.saturating_add(1);
            self.next_core_start = core_end;

            let retain_from = self
                .next_core_start
                .saturating_sub(self.plan.context_samples);
            let drain = usize::try_from(retain_from - self.buffer_start)
                .map_err(|_| WindowPlanError::InvalidPlan("buffer offset overflowed".into()))?;
            if drain > 0 {
                self.buffer.drain(..drain);
                self.buffer_start = retain_from;
            }
        }
        Ok(windows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(sequence: u64, start: u64, samples: usize, final_chunk: bool) -> AudioChunk {
        AudioChunk {
            sequence,
            start_sample: start,
            sample_rate: TARGET_SAMPLE_RATE_HZ,
            samples: (start..start + samples as u64)
                .map(|sample| sample as f32)
                .collect(),
            is_final: final_chunk,
        }
    }

    #[test]
    fn valid_cores_cover_every_sample_once_with_bounded_overlap() {
        let mut planner = ProcessingWindowPlanner::new(AudioWindowPlan::new(10, 2).unwrap());
        let mut windows = Vec::new();
        windows.extend(planner.push(chunk(0, 0, 7, false)).unwrap());
        windows.extend(planner.push(chunk(1, 7, 7, false)).unwrap());
        windows.extend(planner.push(chunk(2, 14, 9, true)).unwrap());
        planner.finish().unwrap();

        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].global_start_sample, 0);
        assert_eq!(windows[0].global_end_sample, 12);
        assert_eq!(windows[1].global_start_sample, 8);
        assert_eq!(windows[1].global_end_sample, 22);
        assert_eq!(windows[2].global_start_sample, 18);
        assert_eq!(windows[2].global_end_sample, 23);
        let cores = windows
            .iter()
            .map(|window| window.valid_core_start_sample..window.valid_core_end_sample)
            .collect::<Vec<_>>();
        assert_eq!(cores, vec![0..10, 10..20, 20..23]);
        assert!(windows[2].is_final);
    }

    #[test]
    fn twenty_hours_are_covered_without_materializing_the_duration() {
        let hour = u64::from(TARGET_SAMPLE_RATE_HZ) * 60 * 60;
        let total = hour * 20;
        let core = u64::from(TARGET_SAMPLE_RATE_HZ) * 60;
        let context = u64::from(TARGET_SAMPLE_RATE_HZ) * 15;
        let plan = AudioWindowPlan::new(core, context).unwrap();
        let windows = plan.bounds_for_total(total).collect::<Vec<_>>();

        assert_eq!(windows.len(), 20 * 60);
        assert_eq!(windows.first().unwrap().valid_core_start_sample, 0);
        assert_eq!(windows.last().unwrap().valid_core_end_sample, total);
        for pair in windows.windows(2) {
            assert_eq!(
                pair[0].valid_core_end_sample,
                pair[1].valid_core_start_sample
            );
            assert!(pair[0].global_end_sample >= pair[1].global_start_sample);
        }
        assert!(windows.iter().all(|window| {
            window.global_end_sample - window.global_start_sample <= core + context * 2
        }));
    }
}
