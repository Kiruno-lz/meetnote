//! The selected document's source audio, as the last check on a worker
//! found it (`WorkspaceService::audio_status`), and the relink in flight.

use application::{AudioSourceError, AudioStatus};
use domain::ProjectId;

/// What the tree row and the Meetings header flag about a document's audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AudioProblem {
    /// No file at the recorded path; processing is disabled.
    Missing,
    /// The file is not the audio the results were computed from.
    Mismatch,
}

impl AudioProblem {
    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::Missing => "audio_missing",
            Self::Mismatch => "audio_mismatch",
        }
    }
}

/// The latest audio check, of one document.
#[derive(Debug, Default)]
pub(crate) struct AudioCheck {
    /// The document the latest check is about.
    document_id: Option<ProjectId>,
    /// What the latest landed check of that document found.
    result: Option<Result<AudioStatus, AudioSourceError>>,
    /// The latest check; an older one that lands later is dropped.
    generation: u64,
    /// The latest check that landed.
    landed: u64,
    /// The document whose relink is in flight.
    pub(crate) relinking: Option<ProjectId>,
}

impl AudioCheck {
    /// Starts a check of `document_id` and returns its generation. The
    /// document's last result stays shown until the new one lands; another
    /// document's is dropped.
    pub(crate) fn begin(&mut self, document_id: ProjectId) -> u64 {
        if self.document_id.as_ref() != Some(&document_id) {
            self.result = None;
            self.document_id = Some(document_id);
        }
        self.generation += 1;
        self.generation
    }

    /// Records what check `generation` found, unless a newer check started;
    /// returns whether it was recorded.
    pub(crate) fn apply(
        &mut self,
        generation: u64,
        result: Result<AudioStatus, AudioSourceError>,
    ) -> bool {
        if generation != self.generation {
            return false;
        }
        self.result = Some(result);
        self.landed = generation;
        true
    }

    /// Whether the latest check has not landed yet.
    pub(crate) fn pending(&self) -> bool {
        self.document_id.is_some() && self.landed != self.generation
    }

    /// Stops accepting an in-flight check while keeping the displayed
    /// result. A documents-root switch restarts the check once it settles.
    pub(crate) fn invalidate_pending(&mut self) {
        self.generation += 1;
        self.landed = self.generation;
    }

    /// Forgets every check, including one still running.
    pub(crate) fn clear(&mut self) {
        self.document_id = None;
        self.result = None;
        self.relinking = None;
        self.generation += 1;
    }

    /// What the latest landed check of `document_id` found.
    pub(crate) fn result_for(
        &self,
        document_id: &ProjectId,
    ) -> Option<&Result<AudioStatus, AudioSourceError>> {
        self.result
            .as_ref()
            .filter(|_| self.document_id.as_ref() == Some(document_id))
    }

    pub(crate) fn problem(&self, document_id: &ProjectId) -> Option<AudioProblem> {
        match self.result_for(document_id)? {
            Ok(AudioStatus::Missing { .. }) => Some(AudioProblem::Missing),
            Ok(AudioStatus::ChecksumMismatch { .. }) => Some(AudioProblem::Mismatch),
            Ok(AudioStatus::Present { .. } | AudioStatus::Unverifiable { .. }) | Err(_) => None,
        }
    }

    /// Whether `document_id`'s audio file is known to be missing.
    pub(crate) fn missing(&self, document_id: &ProjectId) -> bool {
        self.problem(document_id) == Some(AudioProblem::Missing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: &str) -> ProjectId {
        ProjectId::new(value)
    }

    fn missing() -> Result<AudioStatus, AudioSourceError> {
        Ok(AudioStatus::Missing {
            path: "weekly.wav".into(),
        })
    }

    #[test]
    fn only_the_latest_check_of_the_selected_document_lands() {
        let mut check = AudioCheck::default();
        assert!(!check.pending());
        let first = check.begin(id("weekly"));
        let second = check.begin(id("weekly"));
        assert!(
            !check.apply(first, missing()),
            "a superseded check is dropped"
        );
        assert_eq!(check.result_for(&id("weekly")), None);
        assert!(check.pending());
        assert!(check.apply(second, missing()));
        assert!(!check.pending());
        assert_eq!(check.result_for(&id("weekly")), Some(&missing()));
        assert_eq!(check.result_for(&id("other")), None);

        // Checking the same document again keeps its result until it lands.
        let again = check.begin(id("weekly"));
        assert_eq!(check.result_for(&id("weekly")), Some(&missing()));
        let present = Ok(AudioStatus::Present {
            path: "weekly.wav".into(),
            checksum: "ab".repeat(32),
        });
        assert!(check.apply(again, present.clone()));
        assert_eq!(check.result_for(&id("weekly")), Some(&present));

        // Another document starts without a result; clearing drops all.
        let other = check.begin(id("other"));
        assert_eq!(check.result_for(&id("weekly")), None);
        assert_eq!(check.result_for(&id("other")), None);
        check.clear();
        assert!(!check.pending());
        assert!(!check.apply(other, missing()));
        assert_eq!(check.result_for(&id("other")), None);
    }

    #[test]
    fn missing_and_mismatched_audio_are_problems_and_only_missing_blocks() {
        let cases = [
            (missing(), Some(AudioProblem::Missing)),
            (
                Ok(AudioStatus::ChecksumMismatch {
                    path: "weekly.wav".into(),
                    expected: "ab".repeat(32),
                    actual: "cd".repeat(32),
                }),
                Some(AudioProblem::Mismatch),
            ),
            (
                Ok(AudioStatus::Present {
                    path: "weekly.wav".into(),
                    checksum: "ab".repeat(32),
                }),
                None,
            ),
            (
                Ok(AudioStatus::Unverifiable {
                    path: "weekly.wav".into(),
                }),
                None,
            ),
            (Err(AudioSourceError::Service("offline".into())), None),
        ];
        for (result, problem) in cases {
            let mut check = AudioCheck::default();
            let generation = check.begin(id("weekly"));
            check.apply(generation, result.clone());
            assert_eq!(check.problem(&id("weekly")), problem, "{result:?}");
            assert_eq!(
                check.missing(&id("weekly")),
                problem == Some(AudioProblem::Missing)
            );
            assert_eq!(check.problem(&id("other")), None);
        }
        for problem in [AudioProblem::Missing, AudioProblem::Mismatch] {
            for locale in [crate::i18n::Locale::ZhCn, crate::i18n::Locale::EnUs] {
                assert!(!crate::i18n::text(locale, problem.label_key()).is_empty());
            }
        }
    }
}
