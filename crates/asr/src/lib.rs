use application::{AsrBackend, AsrBackendProvider};
use contracts::{AsrRequest, AsrResult, BackendDescriptor, BackendError, ModelRef};
use runtime::JobContext;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Default)]
pub struct AsrBackendRegistry {
    backends: BTreeMap<String, Arc<dyn AsrBackend>>,
}

impl AsrBackendRegistry {
    pub fn register(&mut self, backend: Arc<dyn AsrBackend>) {
        let id = backend.descriptor().id;
        self.backends.insert(id, backend);
    }

    pub fn descriptor(&self, id: &str) -> Option<BackendDescriptor> {
        self.backends.get(id).map(|backend| backend.descriptor())
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn AsrBackend>> {
        self.backends.get(id).cloned()
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.backends.keys().map(String::as_str)
    }
}

pub struct AsrBackendManager {
    registry: AsrBackendRegistry,
}

impl AsrBackendManager {
    pub fn new(registry: AsrBackendRegistry) -> Self {
        Self { registry }
    }

    pub fn available_backends(&self) -> Vec<BackendDescriptor> {
        self.registry
            .ids()
            .filter_map(|id| self.registry.descriptor(id))
            .collect()
    }

    pub fn probe(&self, backend_id: &str, model: &ModelRef) -> Result<(), BackendError> {
        let backend = self.registry.get(backend_id).ok_or_else(|| BackendError {
            code: contracts::BackendErrorCode::BackendUnavailable,
            message: format!("ASR backend not registered: {backend_id}"),
        })?;
        backend.probe(model)
    }

    pub async fn transcribe(
        &self,
        backend_id: &str,
        request: AsrRequest,
        context: JobContext,
    ) -> Result<AsrResult, BackendError> {
        let backend = self.registry.get(backend_id).ok_or_else(|| BackendError {
            code: contracts::BackendErrorCode::BackendUnavailable,
            message: format!("ASR backend not registered: {backend_id}"),
        })?;
        backend.transcribe(request, context).await
    }
}

impl AsrBackendProvider for AsrBackendManager {
    fn available_backends(&self) -> Vec<BackendDescriptor> {
        self.available_backends()
    }

    fn backend(&self, backend_id: &str) -> Result<Arc<dyn AsrBackend>, BackendError> {
        self.registry.get(backend_id).ok_or_else(|| BackendError {
            code: contracts::BackendErrorCode::BackendUnavailable,
            message: format!("ASR backend not registered: {backend_id}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use application::{asr_descriptor, BackendFuture};

    struct FakeBackend;

    impl AsrBackend for FakeBackend {
        fn descriptor(&self) -> BackendDescriptor {
            asr_descriptor("fake", &["test"])
        }

        fn probe(&self, _model: &ModelRef) -> Result<(), BackendError> {
            Ok(())
        }

        fn transcribe(
            &self,
            _request: AsrRequest,
            _context: JobContext,
        ) -> BackendFuture<AsrResult> {
            Box::pin(async { Err(BackendError::not_implemented("test backend")) })
        }
    }

    #[test]
    fn registry_exposes_registered_backend() {
        let mut registry = AsrBackendRegistry::default();
        registry.register(Arc::new(FakeBackend));
        let manager = AsrBackendManager::new(registry);

        assert_eq!(manager.available_backends()[0].id, "fake");
    }
}
