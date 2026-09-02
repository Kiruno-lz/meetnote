use application::{DiarizationBackend, DiarizationBackendProvider};
use contracts::{BackendDescriptor, BackendError, DiarizationRequest, DiarizationResult, ModelRef};
use runtime::JobContext;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct DiarizationBackendRegistry {
    backends: BTreeMap<String, Arc<dyn DiarizationBackend>>,
}

impl DiarizationBackendRegistry {
    pub fn register(&mut self, backend: Arc<dyn DiarizationBackend>) {
        let id = backend.descriptor().id;
        self.backends.insert(id, backend);
    }

    pub fn descriptor(&self, id: &str) -> Option<BackendDescriptor> {
        self.backends.get(id).map(|backend| backend.descriptor())
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn DiarizationBackend>> {
        self.backends.get(id).cloned()
    }
}

#[derive(Clone)]
pub struct DiarizationBackendManager {
    registry: DiarizationBackendRegistry,
}

impl DiarizationBackendManager {
    pub fn new(registry: DiarizationBackendRegistry) -> Self {
        Self { registry }
    }

    pub fn available_backends(&self) -> Vec<BackendDescriptor> {
        self.registry
            .backends
            .values()
            .map(|backend| backend.descriptor())
            .collect()
    }

    pub fn probe(&self, backend_id: &str, model: &ModelRef) -> Result<(), BackendError> {
        let backend = self.registry.get(backend_id).ok_or_else(|| BackendError {
            code: contracts::BackendErrorCode::BackendUnavailable,
            message: format!("Diarization backend not registered: {backend_id}"),
        })?;
        backend.probe(model)
    }

    pub async fn diarize(
        &self,
        backend_id: &str,
        request: DiarizationRequest,
        context: JobContext,
    ) -> Result<DiarizationResult, BackendError> {
        let backend = self.registry.get(backend_id).ok_or_else(|| BackendError {
            code: contracts::BackendErrorCode::BackendUnavailable,
            message: format!("Diarization backend not registered: {backend_id}"),
        })?;
        backend.diarize(request, context).await
    }
}

impl DiarizationBackendProvider for DiarizationBackendManager {
    fn available_backends(&self) -> Vec<BackendDescriptor> {
        self.available_backends()
    }

    fn backend(&self, backend_id: &str) -> Result<Arc<dyn DiarizationBackend>, BackendError> {
        self.registry.get(backend_id).ok_or_else(|| BackendError {
            code: contracts::BackendErrorCode::BackendUnavailable,
            message: format!("Diarization backend not registered: {backend_id}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use application::{diarization_descriptor, BackendFuture};

    struct FakeBackend;

    impl DiarizationBackend for FakeBackend {
        fn descriptor(&self) -> BackendDescriptor {
            diarization_descriptor("fake-diarization", &["test"])
        }

        fn probe(&self, _model: &ModelRef) -> Result<(), BackendError> {
            Ok(())
        }

        fn diarize(
            &self,
            _request: DiarizationRequest,
            _context: JobContext,
        ) -> BackendFuture<DiarizationResult> {
            Box::pin(async { Err(BackendError::not_implemented("test backend")) })
        }
    }

    #[test]
    fn registry_exposes_registered_backend() {
        let mut registry = DiarizationBackendRegistry::default();
        registry.register(Arc::new(FakeBackend));
        let manager = DiarizationBackendManager::new(registry);

        assert_eq!(manager.available_backends()[0].id, "fake-diarization");
    }
}
