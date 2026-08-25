#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmProviderDescriptor {
    pub id: String,
    pub local: bool,
}

pub trait LlmProvider: Send + Sync {
    fn descriptor(&self) -> LlmProviderDescriptor;
}
