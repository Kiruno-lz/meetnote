use std::path::Path;

use contracts::{
    AlgorithmRef, DiarizationStageConfiguration, DiarizationStageKind, DiarizationStageOption,
    ModelRef,
};
use settings::AppSettings;

use super::processing::ProcessingConfig;

pub(crate) struct StageDraft {
    pub parent: Option<ModelRef>,
    pub selection: DiarizationStageConfiguration,
    options: Vec<DiarizationStageOption>,
}

impl StageDraft {
    pub(super) fn load(config: &ProcessingConfig, settings: &AppSettings) -> Self {
        let parent = settings.diarization_model.clone();
        let options = parent
            .as_ref()
            .and_then(|model| config.configurable_stages(settings, model))
            .unwrap_or_default();
        Self {
            parent,
            selection: settings.diarization_stages.clone(),
            options,
        }
    }

    pub fn options(
        &self,
        stage: DiarizationStageKind,
    ) -> impl Iterator<Item = &DiarizationStageOption> {
        self.options
            .iter()
            .filter(move |option| option.stage == stage)
    }

    pub fn selected_id(&self, stage: DiarizationStageKind) -> Option<&str> {
        let (model, algorithm) = match stage {
            DiarizationStageKind::Segmentation => {
                (self.selection.segmentation_model.as_ref(), None)
            }
            DiarizationStageKind::Embedding => (self.selection.embedding_model.as_ref(), None),
            DiarizationStageKind::Clustering => {
                (None, self.selection.clustering_algorithm.as_ref())
            }
        };
        if let Some(model) = model {
            return self
                .options(stage)
                .find(|option| option.model_id.as_deref().unwrap_or(&option.id) == model.model_id)
                .map(|option| option.id.as_str())
                .or(Some(model.model_id.as_str()));
        }
        algorithm.map(|algorithm| algorithm.id.as_str())
    }

    pub fn select(&mut self, stage: DiarizationStageKind, choice: Option<&str>) {
        let option = choice.and_then(|id| self.options(stage).find(|option| option.id == id));
        let model = || -> Option<ModelRef> {
            let option = option?;
            let parent = self.parent.as_ref()?;
            let filename = option.artifact_files.first()?;
            Some(ModelRef {
                backend_id: parent.backend_id.clone(),
                model_id: option.model_id.clone().unwrap_or_else(|| option.id.clone()),
                revision: option.revision.clone().or_else(|| parent.revision.clone()),
                artifact_path: Path::new(&parent.artifact_path)
                    .join(filename)
                    .display()
                    .to_string(),
                artifacts: parent
                    .artifacts
                    .iter()
                    .filter(|artifact| option.artifact_files.contains(&artifact.filename))
                    .cloned()
                    .collect(),
            })
        };
        match stage {
            DiarizationStageKind::Segmentation => self.selection.segmentation_model = model(),
            DiarizationStageKind::Embedding => self.selection.embedding_model = model(),
            DiarizationStageKind::Clustering => {
                self.selection.clustering_algorithm = option.map(|option| AlgorithmRef {
                    id: option.id.clone(),
                    revision: option.revision.clone(),
                    parameters: None,
                });
            }
        }
    }
}
