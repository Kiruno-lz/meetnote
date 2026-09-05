use crate::{
    align_transcript_and_speakers, apply_identity_matches, AudioWindowing, ClusterIdentityResolver,
    IdentityResolverConfig, JsonSpeakerProfileStore, PipelineResult, SpeakerLibraryService,
};
use audio::{
    AudioChunkReader, AudioWindow, AudioWindowPlan, ProcessingWindowPlanner, TARGET_SAMPLE_RATE_HZ,
};
use contracts::{
    AsrRequest, AsrResult, BackendError, BackendErrorCode, DiarizationRequest, DiarizationResult,
};
use runtime::{JobContext, Progress};
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::Arc;

pub trait AsrWindowSession: Send {
    fn process_window(
        &mut self,
        window: Arc<AudioWindow>,
        context: &JobContext,
    ) -> Result<(), BackendError>;
    fn finish(self: Box<Self>) -> Result<AsrResult, BackendError>;
}

pub trait DiarizationWindowSession: Send {
    fn process_window(
        &mut self,
        window: Arc<AudioWindow>,
        context: &JobContext,
    ) -> Result<(), BackendError>;
    fn finish(self: Box<Self>) -> Result<DiarizationResult, BackendError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LongAudioConfig {
    pub activation_samples: u64,
    pub decode_chunk_samples: usize,
    pub window_plan: AudioWindowPlan,
}

impl LongAudioConfig {
    pub fn is_long(&self, estimated_samples: Option<u64>) -> bool {
        estimated_samples.is_none_or(|samples| samples >= self.activation_samples)
    }
}

impl Default for LongAudioConfig {
    fn default() -> Self {
        let second = u64::from(TARGET_SAMPLE_RATE_HZ);
        Self {
            activation_samples: second * 30 * 60,
            decode_chunk_samples: TARGET_SAMPLE_RATE_HZ as usize,
            window_plan: AudioWindowPlan::new(second * 60, second * 15)
                .expect("the default long-audio window plan is valid"),
        }
    }
}

pub fn run_windowed_pipeline(
    asr_backend: Arc<dyn crate::AsrBackend>,
    diarization_backend: Arc<dyn crate::DiarizationBackend>,
    asr_request: AsrRequest,
    diarization_request: DiarizationRequest,
    asr_context: JobContext,
    diarization_context: JobContext,
    config: LongAudioConfig,
) -> crate::BackendFuture<PipelineResult> {
    Box::pin(runtime::spawn_blocking(move || {
        run_windowed_pipeline_sync(
            asr_backend,
            diarization_backend,
            asr_request,
            diarization_request,
            asr_context,
            diarization_context,
            config,
        )
    }))
}

fn run_windowed_pipeline_sync(
    asr_backend: Arc<dyn crate::AsrBackend>,
    diarization_backend: Arc<dyn crate::DiarizationBackend>,
    asr_request: AsrRequest,
    diarization_request: DiarizationRequest,
    asr_context: JobContext,
    diarization_context: JobContext,
    config: LongAudioConfig,
) -> Result<PipelineResult, BackendError> {
    let speaker_library_path = diarization_request.speaker_library_path.clone();
    let stage_provenance = diarization_backend.stage_provenance_with_configuration(
        &diarization_request.model,
        &diarization_request.stage_configuration,
    );
    let mut asr_session = asr_backend.create_window_session(asr_request.clone())?;
    let mut diarization_session =
        diarization_backend.create_window_session(diarization_request.clone())?;
    let mut reader = AudioChunkReader::open(&asr_request.audio_path, config.decode_chunk_samples)
        .map_err(audio_error)?;
    let estimated_total = reader.source_info().estimated_output_samples;
    let mut planner = ProcessingWindowPlanner::new(config.window_plan);
    let (asr_sender, asr_receiver) = sync_channel::<Arc<AudioWindow>>(0);
    let (diarization_sender, diarization_receiver) = sync_channel::<Arc<AudioWindow>>(0);

    let (asr, diarization, decode_result) = std::thread::scope(|scope| {
        let asr_worker_context = asr_context.clone();
        let diarization_worker_context = diarization_context.clone();
        let asr_worker = scope.spawn(move || {
            consume_asr_windows(&mut asr_session, asr_receiver, &asr_worker_context)?;
            asr_session.finish()
        });
        let diarization_worker = scope.spawn(move || {
            consume_diarization_windows(
                &mut diarization_session,
                diarization_receiver,
                &diarization_worker_context,
            )?;
            diarization_session.finish()
        });

        let decode_result: Result<AudioWindowing, BackendError> = (|| {
            let mut window_count = 0u64;
            let mut source_samples = 0u64;
            while let Some(chunk) = reader
                .next_chunk_cancellable(|| {
                    asr_context.is_cancelled() || diarization_context.is_cancelled()
                })
                .map_err(audio_error)?
            {
                for mut window in planner.push(chunk).map_err(window_error)? {
                    if window.source_total_samples.is_none() {
                        window.source_total_samples = estimated_total;
                    }
                    let window = Arc::new(window);
                    window_count = window_count.saturating_add(1);
                    source_samples = window.valid_core_end_sample;
                    asr_sender
                        .send(window.clone())
                        .map_err(|_| worker_stopped("ASR"))?;
                    diarization_sender
                        .send(window)
                        .map_err(|_| worker_stopped("diarization"))?;
                }
            }
            planner.finish().map_err(window_error)?;
            Ok(AudioWindowing {
                sample_rate_hz: TARGET_SAMPLE_RATE_HZ,
                core_samples: config.window_plan.core_samples,
                context_samples: config.window_plan.context_samples,
                window_count,
                source_samples,
            })
        })();
        drop(asr_sender);
        drop(diarization_sender);
        let asr = join_worker(asr_worker, "ASR");
        let diarization = join_worker(diarization_worker, "diarization");
        (asr, diarization, decode_result)
    });

    if let Err(error) = &decode_result {
        if error.code != BackendErrorCode::InferenceFailed {
            return Err(error.clone());
        }
    }
    let asr = asr?;
    let mut diarization = diarization?;
    let windowing = decode_result?;
    apply_speaker_identity(&mut diarization, speaker_library_path.as_deref())?;
    let aligned = align_transcript_and_speakers(&asr.segments, &diarization.segments);
    Ok(PipelineResult {
        asr,
        diarization,
        aligned,
        diarization_stage_provenance: stage_provenance,
        windowing: Some(windowing),
    })
}

fn consume_asr_windows(
    session: &mut Box<dyn AsrWindowSession>,
    receiver: Receiver<Arc<AudioWindow>>,
    context: &JobContext,
) -> Result<(), BackendError> {
    for window in receiver {
        ensure_not_cancelled(context, "ASR")?;
        session.process_window(window.clone(), context)?;
        report_window_progress(context, &window);
    }
    ensure_not_cancelled(context, "ASR")
}

fn consume_diarization_windows(
    session: &mut Box<dyn DiarizationWindowSession>,
    receiver: Receiver<Arc<AudioWindow>>,
    context: &JobContext,
) -> Result<(), BackendError> {
    for window in receiver {
        ensure_not_cancelled(context, "diarization")?;
        session.process_window(window.clone(), context)?;
        report_window_progress(context, &window);
    }
    ensure_not_cancelled(context, "diarization")
}

fn report_window_progress(context: &JobContext, window: &AudioWindow) {
    let total = window
        .source_total_samples
        .or_else(|| window.is_final.then_some(window.valid_core_end_sample))
        .map(|total| total.max(window.valid_core_end_sample));
    context.report(Progress {
        completed: window.valid_core_end_sample,
        total,
    });
}

fn apply_speaker_identity(
    diarization: &mut DiarizationResult,
    speaker_library_path: Option<&str>,
) -> Result<(), BackendError> {
    let Some(path) = speaker_library_path else {
        return Ok(());
    };
    let library = SpeakerLibraryService::new(Arc::new(JsonSpeakerProfileStore::new(path)));
    let resolver = ClusterIdentityResolver::new(IdentityResolverConfig::default());
    apply_identity_matches(diarization, &library, &resolver)
        .map(|_| ())
        .map_err(|error| BackendError {
            code: BackendErrorCode::PersistenceFailed,
            message: error.to_string(),
        })
}

fn ensure_not_cancelled(context: &JobContext, stage: &str) -> Result<(), BackendError> {
    if context.is_cancelled() {
        Err(BackendError {
            code: BackendErrorCode::Cancelled,
            message: format!("windowed {stage} processing was cancelled"),
        })
    } else {
        Ok(())
    }
}

fn join_worker<T>(
    worker: std::thread::ScopedJoinHandle<'_, Result<T, BackendError>>,
    name: &str,
) -> Result<T, BackendError> {
    worker.join().map_err(|_| BackendError {
        code: BackendErrorCode::InferenceFailed,
        message: format!("windowed {name} worker panicked"),
    })?
}

fn audio_error(error: audio::AudioError) -> BackendError {
    let code = if matches!(error, audio::AudioError::Cancelled { .. }) {
        BackendErrorCode::Cancelled
    } else {
        BackendErrorCode::InvalidAudio
    };
    BackendError {
        code,
        message: error.to_string(),
    }
}

fn window_error(error: audio::WindowPlanError) -> BackendError {
    BackendError {
        code: BackendErrorCode::InvalidAudio,
        message: error.to_string(),
    }
}

fn worker_stopped(name: &str) -> BackendError {
    BackendError {
        code: BackendErrorCode::InferenceFailed,
        message: format!("windowed {name} worker stopped before the audio ended"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        asr_descriptor, diarization_descriptor, AsrBackend, BackendFuture, DiarizationBackend,
    };
    use contracts::{
        BackendDescriptor, ComputeDevice, EmbeddingFeatureExtractor, EmbeddingFrontend,
        EmbeddingNormalization, EmbeddingSpace, ModelRef, SpeakerEmbeddingObservation,
        SpeakerSampleSource,
    };
    use domain::{DiarizationSegment, TranscriptSegment};
    use runtime::JobRuntime;
    use std::sync::Mutex;

    type SeenWindows = Arc<Mutex<Vec<(u64, usize, u64, u64)>>>;

    struct FakeWindowAsr {
        seen: SeenWindows,
    }

    struct FakeAsrSession {
        seen: SeenWindows,
        request: AsrRequest,
    }

    impl AsrBackend for FakeWindowAsr {
        fn descriptor(&self) -> BackendDescriptor {
            asr_descriptor("window-asr", &["bounded-offline-windows"])
        }

        fn probe(&self, _: &ModelRef) -> Result<(), BackendError> {
            Ok(())
        }

        fn transcribe(&self, _: AsrRequest, _: JobContext) -> BackendFuture<AsrResult> {
            Box::pin(async { Err(BackendError::not_implemented("whole-file fake")) })
        }

        fn supports_windowed_audio(&self) -> bool {
            true
        }

        fn create_window_session(
            &self,
            request: AsrRequest,
        ) -> Result<Box<dyn AsrWindowSession>, BackendError> {
            Ok(Box::new(FakeAsrSession {
                seen: self.seen.clone(),
                request,
            }))
        }
    }

    impl AsrWindowSession for FakeAsrSession {
        fn process_window(
            &mut self,
            window: Arc<AudioWindow>,
            _: &JobContext,
        ) -> Result<(), BackendError> {
            self.seen.lock().unwrap().push((
                window.sequence,
                Arc::as_ptr(&window) as *const () as usize,
                window.valid_core_start_sample,
                window.valid_core_end_sample,
            ));
            Ok(())
        }

        fn finish(self: Box<Self>) -> Result<AsrResult, BackendError> {
            Ok(AsrResult {
                backend: asr_descriptor("window-asr", &["bounded-offline-windows"]),
                model: self.request.model,
                language: self.request.language,
                segments: vec![TranscriptSegment {
                    id: "final-asr".into(),
                    start_ms: 0,
                    end_ms: 250,
                    text: "complete".into(),
                }],
            })
        }
    }

    struct FakeWindowDiarization {
        seen: SeenWindows,
    }

    struct FakeDiarizationSession {
        seen: SeenWindows,
        request: DiarizationRequest,
    }

    impl DiarizationBackend for FakeWindowDiarization {
        fn descriptor(&self) -> BackendDescriptor {
            diarization_descriptor("window-diarization", &["bounded-offline-windows"])
        }

        fn probe(&self, _: &ModelRef) -> Result<(), BackendError> {
            Ok(())
        }

        fn diarize(
            &self,
            _: DiarizationRequest,
            _: JobContext,
        ) -> BackendFuture<DiarizationResult> {
            Box::pin(async { Err(BackendError::not_implemented("whole-file fake")) })
        }

        fn supports_windowed_audio(&self) -> bool {
            true
        }

        fn create_window_session(
            &self,
            request: DiarizationRequest,
        ) -> Result<Box<dyn DiarizationWindowSession>, BackendError> {
            Ok(Box::new(FakeDiarizationSession {
                seen: self.seen.clone(),
                request,
            }))
        }
    }

    impl DiarizationWindowSession for FakeDiarizationSession {
        fn process_window(
            &mut self,
            window: Arc<AudioWindow>,
            _: &JobContext,
        ) -> Result<(), BackendError> {
            self.seen.lock().unwrap().push((
                window.sequence,
                Arc::as_ptr(&window) as *const () as usize,
                window.valid_core_start_sample,
                window.valid_core_end_sample,
            ));
            Ok(())
        }

        fn finish(self: Box<Self>) -> Result<DiarizationResult, BackendError> {
            Ok(DiarizationResult {
                backend: diarization_descriptor("window-diarization", &["bounded-offline-windows"]),
                model: self.request.model,
                segments: vec![DiarizationSegment {
                    id: "final-diarization".into(),
                    speaker_id: "speaker-0".into(),
                    speaker_profile_id: None,
                    speaker_label: None,
                    start_ms: 0,
                    end_ms: 250,
                    confidence: Some(1.0),
                }],
                speaker_count: Some(1),
                embedding_observations: vec![speaker_observation()],
                speaker_profile_candidates: Vec::new(),
            })
        }
    }

    #[test]
    fn coordinator_decodes_once_and_fans_out_the_same_bounded_windows() {
        let path = write_wav(4_000);
        let library_path = path.with_extension("speakers.json");
        let library =
            SpeakerLibraryService::new(Arc::new(JsonSpeakerProfileStore::new(&library_path)));
        library
            .enroll(
                "Known speaker",
                &speaker_observation(),
                SpeakerSampleSource {
                    project_id: None,
                    source_segment_id: None,
                    start_ms: 0,
                    end_ms: 250,
                    audio_path: Some("fixture.wav".into()),
                    audio_sha256: None,
                    backup_path: None,
                },
                0.7,
            )
            .unwrap();
        let asr_seen = Arc::new(Mutex::new(Vec::new()));
        let diarization_seen = Arc::new(Mutex::new(Vec::new()));
        let model = ModelRef {
            backend_id: "fake".into(),
            model_id: "fake".into(),
            revision: None,
            artifact_path: "fake".into(),
            artifacts: Vec::new(),
        };
        let runtime = JobRuntime::default();
        let (asr_context, _) = runtime.create_job();
        let (diarization_context, _) = runtime.create_job();
        let result = futures::executor::block_on(run_windowed_pipeline(
            Arc::new(FakeWindowAsr {
                seen: asr_seen.clone(),
            }),
            Arc::new(FakeWindowDiarization {
                seen: diarization_seen.clone(),
            }),
            AsrRequest {
                audio_path: path.display().to_string(),
                model: model.clone(),
                language: Some("en".into()),
                device: ComputeDevice::Cpu,
            },
            DiarizationRequest {
                audio_path: path.display().to_string(),
                model,
                expected_speakers: None,
                device: ComputeDevice::Cpu,
                speaker_library_path: Some(library_path.display().to_string()),
                stage_configuration: Default::default(),
            },
            asr_context,
            diarization_context,
            LongAudioConfig {
                activation_samples: 0,
                decode_chunk_samples: 333,
                window_plan: AudioWindowPlan::new(1_600, 400).unwrap(),
            },
        ))
        .unwrap();

        let asr_seen = asr_seen.lock().unwrap();
        let diarization_seen = diarization_seen.lock().unwrap();
        assert_eq!(asr_seen.len(), 3);
        assert_eq!(asr_seen.len(), diarization_seen.len());
        for (asr, diarization) in asr_seen.iter().zip(diarization_seen.iter()) {
            assert_eq!(asr.0, diarization.0);
            assert_eq!(asr.1, diarization.1);
            assert_eq!((asr.2, asr.3), (diarization.2, diarization.3));
        }
        assert_eq!(
            asr_seen
                .iter()
                .map(|seen| seen.2..seen.3)
                .collect::<Vec<_>>(),
            vec![0..1_600, 1_600..3_200, 3_200..4_000]
        );
        assert_eq!(result.aligned[0].speaker_id, "speaker-0");
        assert_eq!(
            result.aligned[0].speaker_label.as_deref(),
            Some("Known speaker")
        );
        assert_eq!(result.windowing.unwrap().window_count, 3);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(library_path).unwrap();
    }

    fn speaker_observation() -> SpeakerEmbeddingObservation {
        SpeakerEmbeddingObservation {
            id: "observation".into(),
            cluster_id: 0,
            start_ms: 0,
            end_ms: 250,
            vector: vec![1.0, 0.0],
            quality: 1.0,
            embedding_space: EmbeddingSpace {
                backend_id: "window-diarization".into(),
                model_id: "fixture-embedding".into(),
                model_revision: Some("1".into()),
                dimension: 2,
                sample_rate_hz: TARGET_SAMPLE_RATE_HZ,
                frontend: EmbeddingFrontend {
                    extractor: EmbeddingFeatureExtractor::KaldiFbank,
                    mel_bins: 80,
                    frame_length_ms: 25,
                    frame_shift_ms: 10,
                    mean_normalization: false,
                    dither: false,
                    overlap_aware: true,
                },
                legacy_feature_config: None,
                normalization: EmbeddingNormalization::L2,
            },
        }
    }

    fn write_wav(samples: usize) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "meetnote-window-coordinator-{}-{}.wav",
            std::process::id(),
            uuid::Uuid::new_v4()
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
        for index in 0..samples {
            writer.write_sample((index % 128) as i16).unwrap();
        }
        writer.finalize().unwrap();
        path
    }
}
