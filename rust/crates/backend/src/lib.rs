mod discovery;
pub mod embeddings;
pub mod frontier;
mod ollama;
mod openai_compat;
mod registry;

pub use discovery::{
    check_ollama, discover_local_models, pull_model, run_health_check,
    detect_system_ram_gb_public, HealthReport, LocalModel,
};
pub use embeddings::{cosine_similarity, EmbeddingClient, EmbeddingError};
pub use frontier::{CoordinatorConfig, CoordinatorProvider, FrontierPlanner};
pub use ollama::{OllamaBackend, OllamaChatRequest, OllamaGenerateRequest, OllamaMessage, BackendEvent};
mod remote_tachy;
pub use remote_tachy::RemoteTachyBackend;
pub use registry::{BackendConfig, BackendKind, BackendRegistry, DynBackend, FallbackApiClient, ModelEntry, ModelTier};
