mod discovery;
pub mod embeddings;
pub mod frontier;
mod ollama;
mod openai_compat;
mod registry;

pub use discovery::{
    check_ollama, detect_system_ram_gb_public, discover_local_models, pull_model, run_health_check,
    HealthReport, LocalModel,
};
pub use embeddings::{cosine_similarity, EmbeddingClient, EmbeddingError};
pub use frontier::{CoordinatorConfig, CoordinatorProvider, FrontierPlanner};
pub use ollama::{
    BackendEvent, OllamaBackend, OllamaChatRequest, OllamaGenerateRequest, OllamaMessage,
};
mod remote_tachy;
pub use registry::{
    BackendConfig, BackendKind, BackendRegistry, DynBackend, FallbackApiClient, ModelEntry,
    ModelTier,
};
pub use remote_tachy::RemoteTachyBackend;
