mod discovery;
mod ollama;
mod openai_compat;
mod registry;

pub use discovery::{
    check_ollama, discover_local_models, pull_model, run_health_check,
    detect_system_ram_gb_public, HealthReport, LocalModel,
};
pub use ollama::OllamaBackend;
pub use openai_compat::OpenAiCompatBackend;
pub use registry::{BackendConfig, BackendKind, BackendRegistry, DynBackend, ModelEntry, ModelTier};
