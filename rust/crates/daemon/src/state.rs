use platform::PermissionMode;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use audit::{
    AuditEvent, AuditEventKind, AuditLogger, FileAuditSink, FilePatch, HttpAuditSink,
    MeteringService, OAuthManager, PolicyEngine, S3AuditSink, SsoManager, StripeBillingConnector,
};
use backend::BackendRegistry;
use platform::{
    AgentConfig, AgentInstance, PlatformConfig, PlatformWorkspace, ScheduleRule, ScheduledTask,
    TaskScheduler,
};
use runtime::FileLockManager;
use serde::{Deserialize, Serialize};

use crate::marketplace::Marketplace;
use crate::mcp_client::McpClientManager;
use crate::saas::SaaSPlatform;
use crate::teams::TeamManager;

/// Persisted state — saved to .tachy/state.json on every mutation.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedState {
    agents: BTreeMap<String, AgentInstance>,
    conversations: BTreeMap<String, Conversation>,
    agent_counter: u64,
    task_counter: u64,
    conv_counter: u64,
    patch_counter: u64,
    pending_patches: Vec<PendingPatch>,
    inference_stats: InferenceStats,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub plans: BTreeMap<String, intelligence::Plan>,
    #[serde(default)]
    pub harnesses: BTreeMap<String, intelligence::AgenticHarness>,
    #[serde(default)]
    pub proposals: BTreeMap<String, crate::engine::OptimizationProposal>,
    #[serde(default)]
    pub investment_policy: platform::finance::InvestmentPolicy,
    #[serde(default)]
    pub active_proposals: BTreeMap<String, platform::governance::GovernanceProposal>,
    #[serde(default)]
    pub hive_mind: intelligence::HiveMind,
    #[serde(default)]
    pub agent_reputations: BTreeMap<String, platform::reputation::ReputationScore>,
    #[serde(default)]
    pub active_nodes: Vec<platform::compute::ComputeNode>,
    #[serde(default)]
    pub swarm_nodes: Vec<platform::replication::SwarmNode>,
    #[serde(default)]
    pub active_crisis: Option<intelligence::Anomaly>,
    #[serde(default)]
    pub expert_adapters: Vec<intelligence::ExpertAdapter>,
    #[serde(default)]
    pub allied_swarms: Vec<platform::diplomacy::DiplomaticSwarm>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamingJob {
    pub id: String,
    pub conv_id: String,
    pub step_index: usize,
    pub status: String,
    pub attempts: usize,
    pub max_attempts: usize,
    pub best_reward: f32,
}

/// A server-side conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub messages: Vec<ChatMessage>,
    pub created_at: String,
    pub updated_at: String,
    pub workspace: String,
    /// The team workspace this conversation belongs to.
    #[serde(default)]
    pub team_id: Option<String>,
    /// Compressed state summary provided by the Summary Agent.
    #[serde(default)]
    pub summary: Option<String>,
}

/// A single chat message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    pub timestamp: String,
    pub model: Option<String>,
    pub iterations: Option<usize>,
    pub tool_invocations: Option<u32>,
}

/// Webhook configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub url: String,
    pub events: Vec<String>,
    pub enabled: bool,
    /// Optional HMAC-SHA256 secret for request signing.
    /// If set, outbound webhooks include `X-Tachy-Signature: sha256=<hmac>`.
    pub secret: Option<String>,
}

/// A patch awaiting human approval from the policy engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingPatch {
    pub id: String,
    pub patch: FilePatch,
    pub reason: String,
    pub created_at: String,
}

/// Statistics for inference performance.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InferenceStats {
    pub total_requests: u64,
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub last_ttft_ms: u32,
    pub last_tokens_per_sec: f32,
    pub avg_ttft_ms: f32,
    pub avg_tokens_per_sec: f32,
    pub p50_ttft_ms: f32,
    pub p95_ttft_ms: f32,
    pub active_training_jobs: Vec<intelligence::finetune::TrainingJob>,
    #[serde(skip)]
    pub ttft_history: VecDeque<u32>,
}

impl InferenceStats {
    /// Record an inference with a known total token count.
    /// Splits evenly between input/output as an approximation.
    pub fn record(&mut self, ttft_ms: u32, tps: f32, tokens: u64) {
        self.record_with_split(ttft_ms, tps, tokens / 2, tokens - tokens / 2);
    }

    /// Record an inference with exact input/output token counts.
    pub fn record_with_split(
        &mut self,
        ttft_ms: u32,
        tps: f32,
        input_tokens: u64,
        output_tokens: u64,
    ) {
        let tokens = input_tokens + output_tokens;
        self.input_tokens += input_tokens;
        self.output_tokens += output_tokens;
        let count = self.total_requests as f32;
        self.avg_ttft_ms = (self.avg_ttft_ms * count + ttft_ms as f32) / (count + 1.0);
        self.avg_tokens_per_sec = (self.avg_tokens_per_sec * count + tps) / (count + 1.0);
        self.total_requests += 1;
        self.total_tokens += tokens;
        self.last_ttft_ms = ttft_ms;
        self.last_tokens_per_sec = tps;

        // Maintain p50/p95 over a sliding window
        self.ttft_history.push_back(ttft_ms);
        if self.ttft_history.len() > 100 {
            self.ttft_history.pop_front();
        }

        let mut sorted = self.ttft_history.iter().copied().collect::<Vec<u32>>();
        sorted.sort_unstable();
        if !sorted.is_empty() {
            let p50_idx = (sorted.len() as f32 * 0.50) as usize;
            let p95_idx = (sorted.len() as f32 * 0.95) as usize;
            self.p50_ttft_ms = sorted[p50_idx.min(sorted.len() - 1)] as f32;
            self.p95_ttft_ms = sorted[p95_idx.min(sorted.len() - 1)] as f32;
        }
    }
}

/// Identity and Access Management service.
pub struct IdentityService {
    pub sso_manager: SsoManager,
    pub oauth_manager: OAuthManager,
    pub user_store: audit::UserStore,
    pub agent_identities: Arc<Mutex<BTreeMap<String, platform::crypto::AgentIdentity>>>,
    /// Per-user resource-quota enforcement (token/cost/concurrency limits by role).
    pub quota_store: audit::QuotaStore,
}

impl IdentityService {
    pub fn get_or_create_identity(
        &self,
        agent_id: &str,
    ) -> Result<platform::crypto::AgentIdentity, String> {
        let mut identities = self.agent_identities.lock().unwrap();
        if let Some(id) = identities.get(agent_id) {
            Ok(id.clone())
        } else {
            let id = platform::crypto::AgentIdentity::generate();
            identities.insert(agent_id.to_string(), id.clone());
            Ok(id)
        }
    }
}

/// Usage, billing, and marketplace service.
pub struct CommerceService {
    pub metering: MeteringService,
    pub billing: Option<StripeBillingConnector>,
    pub marketplace: Marketplace,
    pub saas: Option<SaaSPlatform>,
}

impl CommerceService {
    /// Check if a team has exceeded their resource limits.
    pub fn check_team_quota(&self, team_id: &str, action: &str) -> Result<(), String> {
        if let Some(ref saas) = self.saas {
            // If in SaaS mode, check the tenant limits
            if let Err(e) = saas.check_limits(team_id, action) {
                return Err(format!("Quota exceeded: {e}"));
            }
        }

        // Also check local metering aggregates if needed (e.g. for self-hosted quotas)
        Ok(())
    }
}

/// Swarm orchestration and distributed execution service.
pub struct SwarmService {
    pub orchestrator: Arc<Mutex<crate::parallel::Orchestrator>>,
    pub cloud_jobs: Vec<crate::batch_client::BatchJob>,
    pub worker_registry: crate::worker_registry::WorkerRegistry,
    pub mission_control: Arc<crate::internal_bus::MissionControl>,
    pub mission_feed: Arc<Mutex<VecDeque<crate::internal_bus::MissionEvent>>>,
}

/// External connectivity and webhook service.
pub struct ConnectivityService {
    pub mcp_client: McpClientManager,
    pub webhooks: Vec<WebhookConfig>,
    pub sync_manager: Option<platform::sync::SyncManager>,
}

/// Shared daemon state, wrapped in Arc<Mutex<>> for thread safety.
pub struct DaemonState {
    pub workspace_root: PathBuf,
    pub workspace: Option<platform::PlatformWorkspace>,
    pub config: PlatformConfig,
    pub permission_mode: PermissionMode,
    pub registry: BackendRegistry,
    pub scheduler: TaskScheduler,
    pub agents: BTreeMap<String, AgentInstance>,
    pub conversations: BTreeMap<String, Conversation>,
    pub audit_logger: Arc<AuditLogger>,
    pub swarm_manager: Arc<crate::swarm::SwarmManager>,
    pub agent_counter: u64,
    pub task_counter: u64,
    pub conv_counter: u64,
    pub api_key: Option<String>,

    // Refactored Services
    pub identity: IdentityService,
    pub commerce: CommerceService,
    pub swarm: SwarmService,
    pub connectivity: ConnectivityService,

    /// Shared file lock manager for parallel agent safety.
    pub file_locks: FileLockManager,
    /// Policy engine for patch-level governance.
    pub policy_engine: PolicyEngine,
    /// Sovereign semantic cache for zero-latency responses.
    pub semantic_cache: Arc<runtime::SemanticCache>,
    /// Client for generating semantic embeddings.
    pub embedding_client: Arc<backend::embeddings::EmbeddingClient>,
    /// Patches awaiting human approval.
    pub pending_patches: Vec<PendingPatch>,
    /// Counter for pending patch IDs.
    pub patch_counter: u64,
    pub evolution: crate::engine::EvolutionManager,
    pub proposals: BTreeMap<String, crate::engine::OptimizationProposal>,
    pub liquidity: platform::finance::LiquidityMonitor,
    pub investment_policy: platform::finance::InvestmentPolicy,
    pub active_proposals: BTreeMap<String, platform::governance::GovernanceProposal>,
    pub hive_mind: intelligence::HiveMind,
    pub agent_reputations: BTreeMap<String, platform::reputation::ReputationScore>,
    pub active_nodes: Vec<platform::compute::ComputeNode>,
    pub swarm_nodes: Vec<platform::replication::SwarmNode>,
    pub active_crisis: Option<intelligence::Anomaly>,
    pub expert_adapters: Vec<intelligence::ExpertAdapter>,
    pub allied_swarms: Vec<platform::diplomacy::DiplomaticSwarm>,
    /// Expert Adapter registry — persisted to `.tachy/adapters.json`.
    pub adapter_registry: intelligence::AdapterRegistry,
    /// Team workspace manager.
    pub team_manager: TeamManager,
    /// Real-time inference performance tracking.
    pub inference_stats: InferenceStats,
    /// OpenTelemetry-compatible tracer (no-op when `TACHY_OTLP_ENDPOINT` not set).
    pub tracer: crate::telemetry::Tracer,
    /// Live event bus — subscribers receive SSE messages in real time.
    /// Capacity 256: oldest events dropped if no consumer keeps up.
    pub event_bus: tokio::sync::broadcast::Sender<String>,
    /// Named DAG templates — reusable swarm configurations saved by operators.
    pub run_templates: HashMap<String, RunTemplate>,
    /// Autonomous engineering plans.
    pub plans: BTreeMap<String, intelligence::Plan>,
    /// Active agentic harness loops (Gather -> Act -> Verify).
    pub harnesses: BTreeMap<String, intelligence::AgenticHarness>,
    /// Live codebase index — updated incrementally after each file write so
    /// RAG context stays current without full rebuilds.
    pub codebase_index: Option<intelligence::CodebaseIndex>,
}

/// Status of a run template in the governance lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TemplateStatus {
    Draft,
    Approved,
    Deprecated,
}

/// A reusable swarm configuration that can be saved, loaded, and executed by name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTemplate {
    pub name: String,
    pub description: String,
    pub tasks: Vec<TemplateTask>,
    pub max_concurrency: usize,
    pub created_at: u64,
    /// Version of the template (e.g. "1.0.0").
    #[serde(default = "default_version")]
    pub version: String,
    /// Governance status of the template.
    #[serde(default = "default_status")]
    pub status: TemplateStatus,
    pub team_id: Option<String>,
    /// Cryptographic signature for governance verification.
    #[serde(default)]
    pub signature: Option<String>,
}

impl RunTemplate {
    #[must_use]
    pub fn calculate_hash(&self) -> String {
        let mut text = format!("template:{}:v{}", self.name, self.version);
        for task in &self.tasks {
            text.push_str(&format!("|task:{}:{}", task.template, task.prompt));
            for dep in &task.deps {
                text.push_str(&format!("|dep:{dep}"));
            }
        }
        audit::hash_text(&text)
    }

    #[must_use]
    pub fn verify_signature(&self) -> bool {
        let Some(sig) = &self.signature else {
            return false;
        };
        let hash = self.calculate_hash();
        // In production, this would use a real public key.
        // For the hardening POC, we implement identity-signing (hash == signature).
        sig == &hash
    }
}

fn default_version() -> String {
    "1.0.0".to_string()
}
fn default_status() -> TemplateStatus {
    TemplateStatus::Draft
}

/// A task definition inside a `RunTemplate`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateTask {
    pub template: String,
    pub prompt: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub deps: Vec<String>,
    #[serde(default = "default_priority")]
    pub priority: u8,
}

fn default_priority() -> u8 {
    5
}

/// Audit sink that fires SSE events on `Critical` severity or `GovernanceViolation` kind.
/// Registered as a sink in `DaemonState::init` so alerting flows through the existing
/// `AuditLogger` dispatch path without coupling the `audit` crate to Tokio.
struct AlertAuditSink {
    bus: tokio::sync::broadcast::Sender<String>,
}

impl audit::AuditSink for AlertAuditSink {
    fn write_event(&self, event: &audit::AuditEvent) -> Result<(), String> {
        let is_critical = matches!(event.severity, audit::AuditSeverity::Critical);
        let is_violation = matches!(event.kind, audit::AuditEventKind::GovernanceViolation);
        if is_critical || is_violation {
            let msg = format!(
                "event: audit_alert\ndata: {}\n\n",
                serde_json::json!({
                    "kind": "audit_alert",
                    "payload": {
                        "severity": format!("{:?}", event.severity),
                        "event_kind": format!("{:?}", event.kind),
                        "detail": event.detail,
                        "session_id": event.session_id,
                        "agent_id": event.agent_id,
                        "hash": event.hash,
                        "ts": event.timestamp,
                    }
                })
            );
            let _ = self.bus.send(msg);
        }
        Ok(())
    }

    fn flush(&self) -> Result<(), String> {
        Ok(())
    }
}

impl DaemonState {
    pub fn init(workspace_root: PathBuf) -> Result<Self, String> {
        let ws = PlatformWorkspace::init(&workspace_root)?;

        // Create the event bus early so AlertAuditSink can reference it.
        let (event_bus_tx, _) = tokio::sync::broadcast::channel::<String>(256);

        let mut audit_logger = AuditLogger::resume_from_file(&ws.audit_log_path());
        if let Ok(sink) = FileAuditSink::new(ws.audit_log_path()) {
            audit_logger.add_sink(sink);
        }
        // Wire critical-event alerting into the audit chain.
        audit_logger.add_sink(AlertAuditSink {
            bus: event_bus_tx.clone(),
        });

        audit_logger.log(&AuditEvent::new(
            "daemon",
            AuditEventKind::SessionStart,
            "daemon started",
        ));

        // Optional HTTP sink — set TACHY_AUDIT_HTTP_URL to enable
        if let Ok(url) = std::env::var("TACHY_AUDIT_HTTP_URL") {
            let token = std::env::var("TACHY_AUDIT_HTTP_TOKEN").ok();
            let sink = HttpAuditSink::new(url, token, None);
            audit_logger.add_sink(sink);
        }

        // Optional S3/MinIO sink — set TACHY_AUDIT_S3_BUCKET + credentials to enable
        if let (Ok(bucket), Ok(region), Ok(access_key), Ok(secret_key)) = (
            std::env::var("TACHY_AUDIT_S3_BUCKET"),
            std::env::var("TACHY_AUDIT_S3_REGION"),
            std::env::var("TACHY_AUDIT_S3_ACCESS_KEY"),
            std::env::var("TACHY_AUDIT_S3_SECRET_KEY"),
        ) {
            let prefix =
                std::env::var("TACHY_AUDIT_S3_PREFIX").unwrap_or_else(|_| "audit".to_string());
            let mut sink = S3AuditSink::new(bucket, prefix, region, access_key, secret_key, None);
            if let Ok(ep) = std::env::var("TACHY_AUDIT_S3_ENDPOINT") {
                sink = sink.with_endpoint(ep);
            }
            audit_logger.add_sink(sink);
        }

        let audit_logger = Arc::new(audit_logger);

        let registry = BackendRegistry::with_defaults();

        // Load API key from env or config
        let api_key = std::env::var("TACHY_API_KEY").ok();

        // Register all known env secrets for masking in the audit trail.
        if let Some(ref k) = api_key {
            audit_logger.mask_secret(k);
        }
        for var in &[
            "YAYA_API_KEY",
            "TACHY_AUDIT_S3_ACCESS_KEY",
            "TACHY_AUDIT_S3_SECRET_KEY",
            "TACHY_SYNC_KEY",
            "TACHY_AUDIT_HTTP_TOKEN",
        ] {
            if let Ok(s) = std::env::var(var) {
                audit_logger.mask_secret(&s);
            }
        }

        // Restore persisted state if it exists
        let state_path = workspace_root.join(".tachy").join("state.json");
        let persisted = load_persisted_state(&state_path);
        let restored = persisted.clone();

        let (
            agents,
            conversations,
            agent_counter,
            task_counter,
            conv_counter,
            patch_counter,
            pending_patches,
            inference_stats,
            device_id,
            plans,
            harnesses,
            proposals,
        ) = match persisted {
            Some(p) => {
                let count = p.agents.len();
                audit_logger.log(&AuditEvent::new(
                    "daemon",
                    AuditEventKind::SessionStart,
                    format!(
                        "restored {count} agents, {} conversations from disk",
                        p.conversations.len()
                    ),
                ));
                (
                    p.agents,
                    p.conversations,
                    p.agent_counter,
                    p.task_counter,
                    p.conv_counter,
                    p.patch_counter,
                    p.pending_patches,
                    p.inference_stats,
                    p.device_id,
                    p.plans,
                    p.harnesses,
                    p.proposals,
                )
            }
            None => (
                BTreeMap::new(),
                BTreeMap::new(),
                0,
                0,
                0,
                0,
                Vec::new(),
                InferenceStats::default(),
                format!("device-{}", uuid::Uuid::new_v4()),
                BTreeMap::new(),
                BTreeMap::new(),
                BTreeMap::new(),
            ),
        };

        // Initialize SyncManager if a key is provided
        let sync_manager = if let Ok(key_hex) = std::env::var("TACHY_SYNC_KEY") {
            if let Ok(key_bytes) = hex::decode(key_hex) {
                if let Ok(key) = key_bytes.try_into() {
                    Some(platform::sync::SyncManager::new(
                        workspace_root.clone(),
                        device_id.clone(),
                        key,
                    ))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Load webhooks from config
        let webhooks: Vec<WebhookConfig> = Vec::new(); // loaded from config if present

        // Restore run history from the durable JSONL log
        let orchestrator = {
            let mut orch = crate::parallel::Orchestrator::new(8);
            for run in crate::parallel::Orchestrator::load_run_history(&workspace_root) {
                orch.register_completed_run(run);
            }
            Arc::new(Mutex::new(orch))
        };

        let cache_path = workspace_root
            .join(".tachy")
            .join("cache")
            .join("semantic.json");
        let semantic_cache = runtime::SemanticCache::load(&cache_path)
            .unwrap_or_else(|_| runtime::SemanticCache::new());

        let mut state = Self {
            workspace_root: workspace_root.clone(),
            workspace: Some(platform::PlatformWorkspace {
                root: workspace_root.clone(),
                config: ws.config.clone(),
            }),
            config: ws.config,
            permission_mode: PermissionMode::default(),
            registry,
            scheduler: TaskScheduler::new(),
            agents,
            conversations,
            audit_logger: audit_logger.clone(),
            swarm_manager: Arc::new(crate::swarm::SwarmManager::new()),
            agent_counter,
            task_counter,
            conv_counter,
            api_key,

            identity: IdentityService {
                sso_manager: SsoManager::new(audit::SsoConfig::default()),
                oauth_manager: OAuthManager::new(),
                user_store: audit::UserStore::new(),
                agent_identities: Arc::new(Mutex::new(BTreeMap::new())),
                quota_store: audit::QuotaStore::new(),
            },
            commerce: CommerceService {
                metering: MeteringService::new(
                    audit_logger.clone(),
                    audit::cost_model::CostModelRegistry::load(&workspace_root),
                ),
                billing: None,
                marketplace: Marketplace::new(),
                saas: None,
            },
            swarm: SwarmService {
                orchestrator,
                cloud_jobs: Vec::new(),
                worker_registry: crate::worker_registry::WorkerRegistry::new(),
                mission_control: Arc::new(crate::internal_bus::MissionControl::new(1024)),
                mission_feed: Arc::new(Mutex::new(VecDeque::with_capacity(100))),
            },
            connectivity: ConnectivityService {
                mcp_client: McpClientManager::new(),
                webhooks,
                sync_manager,
            },

            file_locks: FileLockManager::new(),
            policy_engine: PolicyEngine::enterprise_default(),
            semantic_cache: Arc::new(semantic_cache),
            embedding_client: Arc::new(backend::embeddings::EmbeddingClient::new()),
            pending_patches,
            patch_counter,
            adapter_registry: {
                let mut reg = intelligence::AdapterRegistry::load(
                    workspace_root.join(".tachy").join("adapters.json"),
                );
                // Migrate any adapters previously stored in PersistedState
                if let Some(ref p) = restored {
                    reg.sync_from_vec(&p.expert_adapters);
                }
                reg
            },
            team_manager: TeamManager::new(),
            inference_stats,
            tracer: {
                let collector =
                    std::sync::Arc::new(Mutex::new(crate::telemetry::SpanCollector::new()));
                crate::telemetry::Tracer::new(collector)
            },
            event_bus: event_bus_tx,
            run_templates: HashMap::new(),
            plans,
            harnesses,
            evolution: crate::engine::EvolutionManager::new(&workspace_root),
            proposals,
            liquidity: platform::finance::LiquidityMonitor::new(),
            investment_policy: restored
                .as_ref()
                .map(|p| p.investment_policy.clone())
                .unwrap_or_default(),
            active_proposals: restored
                .as_ref()
                .map(|p| p.active_proposals.clone())
                .unwrap_or_default(),
            hive_mind: restored
                .as_ref()
                .map(|p| p.hive_mind.clone())
                .unwrap_or_default(),
            agent_reputations: restored
                .as_ref()
                .map(|p| p.agent_reputations.clone())
                .unwrap_or_default(),
            active_nodes: restored
                .as_ref()
                .map(|p| p.active_nodes.clone())
                .unwrap_or_default(),
            swarm_nodes: restored
                .as_ref()
                .map(|p| p.swarm_nodes.clone())
                .unwrap_or_default(),
            active_crisis: restored.as_ref().and_then(|p| p.active_crisis.clone()),
            expert_adapters: restored
                .as_ref()
                .map(|p| p.expert_adapters.clone())
                .unwrap_or_default(),
            allied_swarms: restored
                .as_ref()
                .map(|p| p.allied_swarms.clone())
                .unwrap_or_default(),
            // Attempt to load a pre-built index; agents will trigger incremental
            // updates after file writes via reindex_changed_files.
            codebase_index: intelligence::CodebaseIndexer::load_index(&workspace_root).ok(),
        };

        // Bridge platform logs to the dashboard
        platform::logger::set_logger(Box::new(crate::logger::DashboardLogger::new(
            state.event_bus.clone(),
        )));
        println!("[DEBUG] Logger initialized in DaemonState::init");
        platform::log_info("Testing global logger bridge");

        // Auto-select Gemma 4 if no model is configured
        if state
            .config
            .agent_templates
            .iter()
            .all(|t| t.model.is_empty() || t.model == "gemma4:26b")
        {
            let report = backend::run_health_check("http://localhost:11434");
            if let Some(model) = report.recommended_model {
                if model.contains("gemma4") {
                    for t in &mut state.config.agent_templates {
                        if t.model.is_empty() || t.model == "gemma4:26b" {
                            t.model = model.clone();
                        }
                    }
                }
            }
        }

        if let Some(api_key) = &state.api_key {
            state.identity.user_store =
                audit::UserStore::with_default_admin(&audit::hash_api_key(api_key));
        }

        let mut stale_agents = Vec::new();
        for agent in state.agents.values_mut() {
            if agent.status == platform::AgentStatus::Running {
                agent.mark_failed("Recovered after daemon restart; previous run was interrupted.");
                stale_agents.push(agent.id.clone());
            }
        }
        if !stale_agents.is_empty() {
            state.audit_logger.log(&AuditEvent::new(
                "daemon",
                AuditEventKind::SessionStart,
                format!("recovered {} stale running agents", stale_agents.len()),
            ));
            state.save();
        }

        Ok(state)
    }

    pub fn load(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let state_path = self.workspace_root.join(".tachy").join("state.json");
        if !state_path.exists() {
            return Ok(());
        }

        let json = std::fs::read_to_string(state_path)?;
        let p: PersistedState = serde_json::from_str(&json)?;

        self.agents = p.agents;
        self.conversations = p.conversations;
        self.agent_counter = p.agent_counter;
        self.task_counter = p.task_counter;
        self.conv_counter = p.conv_counter;
        self.patch_counter = p.patch_counter;
        self.pending_patches = p.pending_patches;
        self.inference_stats = p.inference_stats;
        self.plans = p.plans;
        self.harnesses = p.harnesses;

        Ok(())
    }

    pub fn next_agent_id(&mut self) -> String {
        self.agent_counter += 1;
        format!("agent-{}", self.agent_counter)
    }

    pub fn next_task_id(&mut self) -> String {
        self.task_counter += 1;
        format!("task-{}", self.task_counter)
    }

    /// Persist current state to disk.
    pub fn save(&self) {
        let state_dir = self.workspace_root.join(".tachy");
        let state_path = state_dir.join("state.json");
        let tmp_path = state_dir.join("state.json.tmp");

        let persisted = PersistedState {
            agents: self.agents.clone(),
            conversations: self.conversations.clone(),
            agent_counter: self.agent_counter,
            task_counter: self.task_counter,
            conv_counter: self.conv_counter,
            patch_counter: self.patch_counter,
            pending_patches: self.pending_patches.clone(),
            inference_stats: self.inference_stats.clone(),
            device_id: self
                .connectivity
                .sync_manager
                .as_ref()
                .map(|m| m.device_id().to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            plans: self.plans.clone(),
            harnesses: self.harnesses.clone(),
            proposals: self.proposals.clone(),
            investment_policy: self.investment_policy.clone(),
            active_proposals: self.active_proposals.clone(),
            hive_mind: self.hive_mind.clone(),
            agent_reputations: self.agent_reputations.clone(),
            active_nodes: self.active_nodes.clone(),
            swarm_nodes: self.swarm_nodes.clone(),
            active_crisis: self.active_crisis.clone(),
            expert_adapters: self.expert_adapters.clone(),
            allied_swarms: self.allied_swarms.clone(),
        };

        if let Ok(json) = serde_json::to_string_pretty(&persisted) {
            // Atomic rewrite: write to .tmp and rename
            if std::fs::write(&tmp_path, json).is_ok() {
                let _ = std::fs::rename(tmp_path, state_path);
            }
        }

        // Persist semantic cache
        let cache_dir = state_dir.join("cache");
        if !cache_dir.exists() {
            let _ = std::fs::create_dir_all(&cache_dir);
        }
        let _ = self.semantic_cache.save(&cache_dir.join("semantic.json"));
    }

    pub fn trigger_evolution(&mut self) {
        let proposals = self
            .evolution
            .analyze_performance(&self.config.intelligence);
        for p in proposals {
            if !self.proposals.contains_key(&p.id) {
                self.publish_event("optimization_proposed", serde_json::json!(&p));
                self.proposals.insert(p.id.clone(), p);
            }
        }
        self.save();
    }

    pub fn apply_optimization(&mut self, proposal_id: &str) -> Result<(), String> {
        let mut proposal = self
            .proposals
            .get(proposal_id)
            .cloned()
            .ok_or("Proposal not found")?;
        if proposal.status != crate::engine::OptimizationStatus::Pending {
            return Err("Optimization already processed".to_string());
        }

        proposal.status = crate::engine::OptimizationStatus::Applied;
        self.proposals
            .insert(proposal_id.to_string(), proposal.clone());

        self.publish_event(
            "evolution_applied",
            serde_json::json!({
                "id": proposal_id,
                "template": proposal.template_name,
            }),
        );

        self.save();
        Ok(())
    }

    pub fn rollback_optimization(&mut self, proposal_id: &str) -> Result<(), String> {
        let mut proposal = self
            .proposals
            .get(proposal_id)
            .cloned()
            .ok_or("Proposal not found")?;
        if proposal.status != crate::engine::OptimizationStatus::Applied {
            return Err("Only applied optimizations can be rolled back".to_string());
        }

        proposal.status = crate::engine::OptimizationStatus::RolledBack;
        self.proposals
            .insert(proposal_id.to_string(), proposal.clone());

        self.publish_event(
            "evolution_rolled_back",
            serde_json::json!({
                "id": proposal_id,
            }),
        );

        self.save();
        Ok(())
    }

    pub fn next_conv_id(&mut self) -> String {
        self.conv_counter += 1;
        format!("conv-{}", self.conv_counter)
    }

    /// Create a new conversation.
    pub fn create_conversation(&mut self, title: &str) -> String {
        let id = self.next_conv_id();
        let now = timestamp();
        let conv = Conversation {
            id: id.clone(),
            title: title.to_string(),
            messages: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
            workspace: self.workspace_root.to_string_lossy().to_string(),
            team_id: None,
            summary: None,
        };
        self.conversations.insert(id.clone(), conv);
        self.save();
        id
    }

    /// Add a message to a conversation.
    pub fn add_message(&mut self, conv_id: &str, msg: ChatMessage) -> bool {
        if let Some(conv) = self.conversations.get_mut(conv_id) {
            conv.messages.push(msg);
            conv.updated_at = timestamp();

            // Trigger Summary Agent if conversation is getting long
            if conv.messages.len() > 15 && conv.messages.len() % 5 == 0 {
                self.summarize_conversation(conv_id);
            }

            self.save();
            true
        } else {
            false
        }
    }

    /// Run the Summary Agent to compress the conversation history.
    pub fn summarize_conversation(&mut self, conv_id: &str) {
        let Some(conv) = self.conversations.get_mut(conv_id) else {
            return;
        };

        let json_messages: Vec<serde_json::Value> = conv
            .messages
            .iter()
            .filter_map(|m| serde_json::to_value(m).ok())
            .collect();

        let summary = intelligence::SummaryManager::summarize(&json_messages);
        conv.summary = Some(summary);

        self.audit_logger.log(&audit::AuditEvent::new(
            "daemon",
            audit::AuditEventKind::SessionCompacted,
            format!("Conversation {conv_id} context compressed by Summary Agent"),
        ));
    }

    /// Queue a patch for human approval.
    pub fn queue_pending_patch(&mut self, patch: FilePatch, reason: String) -> String {
        self.patch_counter += 1;
        let id = format!("patch-{}", self.patch_counter);
        self.pending_patches.push(PendingPatch {
            id: id.clone(),
            patch,
            reason,
            created_at: timestamp(),
        });
        self.save();
        id
    }

    /// Approve a pending patch — apply it to disk.
    pub fn approve_patch(&mut self, patch_id: &str) -> Result<String, String> {
        let idx = self
            .pending_patches
            .iter()
            .position(|p| p.id == patch_id)
            .ok_or_else(|| format!("patch '{patch_id}' not found"))?;
        let pending = self.pending_patches.remove(idx);
        // Apply the patch
        std::fs::write(&pending.patch.file_path, &pending.patch.new_content)
            .map_err(|e| format!("failed to apply patch: {e}"))?;
        self.audit_logger.log(
            &AuditEvent::new(
                "daemon",
                AuditEventKind::PermissionGranted,
                format!(
                    "patch {} approved and applied: {}",
                    patch_id, pending.patch.file_path
                ),
            )
            .with_agent(&pending.patch.agent_id),
        );
        self.save();
        Ok(pending.patch.file_path)
    }

    /// Reject a pending patch — discard it.
    pub fn reject_patch(&mut self, patch_id: &str) -> Result<String, String> {
        let idx = self
            .pending_patches
            .iter()
            .position(|p| p.id == patch_id)
            .ok_or_else(|| format!("patch '{patch_id}' not found"))?;
        let pending = self.pending_patches.remove(idx);
        self.audit_logger.log(
            &AuditEvent::new(
                "daemon",
                AuditEventKind::PermissionDenied,
                format!("patch {} rejected: {}", patch_id, pending.patch.file_path),
            )
            .with_agent(&pending.patch.agent_id)
            .with_severity(audit::AuditSeverity::Warning),
        );
        self.save();
        Ok(pending.patch.file_path)
    }

    /// Transition a plan from `SafePlanReady` to `InProgress`.
    pub fn approve_plan(&mut self, template_name: &str) -> Result<(), String> {
        if let Some(_template) = self.run_templates.get_mut(template_name) {
            // Find any task in SafePlanReady and move to InProgress (or Created if not started)
            // For now, we assume the whole template run is being approved.
            self.audit_logger.log(&AuditEvent::new(
                "daemon",
                AuditEventKind::PermissionGranted,
                format!("plan for template {template_name} approved for execution"),
            ));
            self.save();
            Ok(())
        } else {
            Err(format!("template '{template_name}' not found"))
        }
    }

    /// Fork a session at a specific cryptographic hash (Pillar 1: State Reconstruction).
    /// Pinpoints the exact event, finds its position in the session, and creates a new branch.
    pub fn fork_session_at_hash(
        &mut self,
        session_id: &str,
        event_hash: &str,
    ) -> Result<String, String> {
        let audit_path = self.workspace_root.join(".tachy").join("audit.jsonl");
        let content = std::fs::read_to_string(&audit_path)
            .map_err(|e| format!("Failed to read audit log: {e}"))?;

        let mut events = Vec::new();
        for line in content.lines() {
            if let Ok(event) = serde_json::from_str::<audit::AuditEvent>(line) {
                if event.session_id == session_id {
                    events.push(event);
                }
            }
        }

        // 1. Find the target event and verify it belongs to this session
        let target_idx = events
            .iter()
            .position(|e| e.hash == event_hash)
            .ok_or_else(|| format!("Hash {event_hash} not found in session {session_id}"))?;

        // 2. Count messages up to this event to find the fork point
        let mut message_count = 0;
        for event in events.iter().take(target_idx + 1) {
            match event.kind {
                audit::AuditEventKind::UserMessage | audit::AuditEventKind::AssistantMessage => {
                    message_count += 1;
                }
                _ => {}
            }
        }

        // 3. Load the session and create a new fork
        let session_path = self
            .workspace_root
            .join(".tachy")
            .join("sessions")
            .join(format!("{session_id}.json"));
        let session_json = std::fs::read_to_string(&session_path)
            .map_err(|e| format!("Failed to read session {session_id}: {e}"))?;
        let session: runtime::Session = serde_json::from_str(&session_json)
            .map_err(|e| format!("Failed to parse session {session_id}: {e}"))?;

        let new_session_id = format!(
            "{}-fork-{}",
            session_id,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        );
        let mut forked_messages = session.messages.clone();
        forked_messages.truncate(message_count);

        let forked_session = runtime::Session {
            version: session.version,
            messages: forked_messages,
            branches: Vec::new(),
            current_branch: "main".to_string(),
            success: false,
            human_override: false,
            team_id: session.team_id.clone(),
        };

        // 4. Persist and Log
        let new_session_path = self
            .workspace_root
            .join(".tachy")
            .join("sessions")
            .join(format!("{new_session_id}.json"));
        let new_session_json = serde_json::to_string_pretty(&forked_session)
            .map_err(|e| format!("Failed to serialize forked session: {e}"))?;
        std::fs::write(&new_session_path, new_session_json)
            .map_err(|e| format!("Failed to write forked session {new_session_id}: {e}"))?;

        self.audit_logger.log(&audit::AuditEvent::new(
            &new_session_id,
            audit::AuditEventKind::SessionStart,
            format!("Forked from {session_id} at hash {event_hash}"),
        ));

        Ok(new_session_id)
    }

    /// Select the best available model for a given agent role.
    /// Returns the specialized adapter if promoted, otherwise falls back to the template default.
    #[must_use]
    pub fn get_expert_model_for_role(&self, template_name: &str) -> String {
        if let Some(template) = self.run_templates.get(template_name) {
            // Find the first task and return its model, or a default
            template
                .tasks
                .first()
                .and_then(|t| t.model.clone())
                .unwrap_or_else(|| "gemma4:26b".to_string())
        } else {
            "gemma4:26b".to_string()
        }
    }

    /// Hot-swap a model adapter for a specific template.
    /// This is used by the autonomous feedback loop to promote verified fine-tuned models.
    pub fn promote_model_adapter(
        &mut self,
        template_name: &str,
        new_model_name: &str,
    ) -> Result<(), String> {
        if let Some(template) = self.run_templates.get_mut(template_name) {
            let old_model = template
                .tasks
                .iter_mut()
                .find(|t| t.template == template_name || t.template == "default")
                .and_then(|t| {
                    let old = t.model.clone();
                    t.model = Some(new_model_name.to_string());
                    old
                });

            self.audit_logger.log(
                &AuditEvent::new("daemon", AuditEventKind::ModelSwitch,
                    format!("promoted model for template {template_name}: {old_model:?} -> {new_model_name}"))
            );
            self.save();
            Ok(())
        } else {
            // Also check global config templates if not in run_templates
            let mut found = false;
            for t in &mut self.config.agent_templates {
                if t.name == template_name {
                    t.model = new_model_name.to_string();
                    found = true;
                }
            }
            if found {
                self.audit_logger.log(&AuditEvent::new(
                    "daemon",
                    AuditEventKind::ModelSwitch,
                    format!(
                        "promoted global model for template {template_name}: -> {new_model_name}"
                    ),
                ));
                self.save();
                return Ok(());
            }
            Err(format!("template '{template_name}' not found"))
        }
    }

    /// Publish a structured event to the live SSE bus.
    ///
    /// Subscribers on `GET /api/events` receive it immediately.
    /// If no subscribers are connected the send silently no-ops.
    pub fn publish_event(&self, kind: &str, payload: serde_json::Value) {
        let msg = format!(
            "event: {kind}\ndata: {}\n\n",
            serde_json::json!({ "kind": kind, "payload": payload, "ts": timestamp() })
        );
        // Ignore send errors — no subscribers is fine
        let _ = self.event_bus.send(msg);
    }

    /// Fire webhooks for an event.
    /// Outbound payloads are HMAC-SHA256 signed when a `secret` is configured.
    pub fn fire_webhooks(&self, event_type: &str, payload: &serde_json::Value) {
        for webhook in &self.connectivity.webhooks {
            if !webhook.enabled {
                continue;
            }
            if !webhook.events.contains(&event_type.to_string())
                && !webhook.events.contains(&"*".to_string())
            {
                continue;
            }

            let url = webhook.url.clone();
            let body_json = serde_json::json!({
                "event": event_type,
                "payload": payload,
                "timestamp": timestamp(),
            });
            let body_str = body_json.to_string();

            // Compute HMAC-SHA256 signature if a secret is configured
            let signature_header = webhook.secret.as_deref().map(|secret| {
                let sig = hmac_sha256(secret.as_bytes(), body_str.as_bytes());
                format!("sha256={sig}")
            });

            let sig_hdr = signature_header.clone();
            // Fire and forget — don't block on webhook delivery
            std::thread::spawn(move || {
                let mut args = vec!["-s", "-X", "POST", "-H", "Content-Type: application/json"];
                let sig_arg;
                if let Some(sig) = &sig_hdr {
                    sig_arg = format!("X-Tachy-Signature: {sig}");
                    args.push("-H");
                    args.push(&sig_arg);
                }
                args.extend(["-d", &body_str, &url]);
                let _ = std::process::Command::new("curl").args(&args).output();
            });
        }
    }

    /// Validate an inbound webhook payload against its registered secret.
    /// Returns `Ok(())` if the signature matches or no secret is configured.
    pub fn verify_webhook_signature(
        &self,
        webhook_url: &str,
        payload: &[u8],
        signature_header: &str,
    ) -> Result<(), String> {
        let webhook = self
            .connectivity
            .webhooks
            .iter()
            .find(|w| w.url == webhook_url);
        let secret = match webhook.and_then(|w| w.secret.as_deref()) {
            Some(s) => s,
            None => return Ok(()), // no secret configured → accept all
        };

        let expected = format!("sha256={}", hmac_sha256(secret.as_bytes(), payload));
        if constant_time_eq(expected.as_bytes(), signature_header.as_bytes()) {
            Ok(())
        } else {
            Err("webhook signature mismatch".to_string())
        }
    }

    /// Create an agent instance from a template name and prompt.
    pub fn create_agent(&mut self, template_name: &str, prompt: &str) -> Result<String, String> {
        let template = self
            .config
            .agent_templates
            .iter()
            .find(|t| t.name == template_name)
            .cloned()
            .ok_or_else(|| format!("unknown template: {template_name}"))?;

        let agent_id = self.next_agent_id();
        let session_id = format!("sess-{agent_id}");

        let config = AgentConfig {
            template,
            session_id: session_id.clone(),
            working_directory: self.workspace_root.to_string_lossy().to_string(),
            environment: BTreeMap::new(),
            team_id: None,
        };

        let mut instance = AgentInstance::new(&agent_id, config);
        instance.result_summary = Some(format!("prompt: {prompt}"));

        self.audit_logger.log(
            &AuditEvent::new(&session_id, AuditEventKind::SessionStart, "agent created")
                .with_agent(&agent_id),
        );

        self.agents.insert(agent_id.clone(), instance);
        self.save();
        Ok(agent_id)
    }

    /// Schedule an agent to run on a trigger.
    pub fn schedule_agent(
        &mut self,
        template_name: &str,
        schedule: ScheduleRule,
        name: &str,
    ) -> Result<String, String> {
        let template = self
            .config
            .agent_templates
            .iter()
            .find(|t| t.name == template_name)
            .cloned()
            .ok_or_else(|| format!("unknown template: {template_name}"))?;

        let task_id = self.next_task_id();
        let session_id = format!("sess-{task_id}");

        let config = AgentConfig {
            template,
            session_id,
            working_directory: self.workspace_root.to_string_lossy().to_string(),
            environment: BTreeMap::new(),
            team_id: None,
        };

        let task = ScheduledTask::new(&task_id, name, config, schedule);
        self.scheduler.add_task(task);
        self.save();
        Ok(task_id)
    }

    pub fn record_inference(&mut self, ttft_ms: u32, tokens_per_sec: f32, tokens: u64) {
        let s = &mut self.inference_stats;
        s.total_requests += 1;
        s.total_tokens += tokens;
        s.last_ttft_ms = ttft_ms;
        s.last_tokens_per_sec = tokens_per_sec;

        // Simple moving average
        let n = s.total_requests as f32;
        s.avg_ttft_ms = (s.avg_ttft_ms * (n - 1.0) + ttft_ms as f32) / n;
        s.avg_tokens_per_sec = (s.avg_tokens_per_sec * (n - 1.0) + tokens_per_sec) / n;
    }

    /// Get a conversation by ID.
    #[must_use]
    pub fn get_conversation(&self, conv_id: &str) -> Option<&Conversation> {
        self.conversations.get(conv_id)
    }

    /// Delete a conversation. Returns true if it existed.
    pub fn delete_conversation(&mut self, conv_id: &str) -> bool {
        let removed = self.conversations.remove(conv_id).is_some();
        if removed {
            self.save();
        }
        removed
    }

    /// Delete (stop / forget) an agent. Returns true if it existed.
    pub fn delete_agent(&mut self, agent_id: &str) -> bool {
        let removed = self.agents.remove(agent_id).is_some();
        if removed {
            self.save();
        }
        removed
    }

    /// Update an agent's status to Failed (used to surface cancellation).
    pub fn cancel_agent(&mut self, agent_id: &str) -> bool {
        if let Some(agent) = self.agents.get_mut(agent_id) {
            agent.status = platform::AgentStatus::Failed;
            self.save();
            true
        } else {
            false
        }
    }

    /// Background utility to clean up old snapshots.
    pub fn clean_vision_cache(&self) {
        let vision_dir = self.workspace_root.join(".tachy").join("vision");
        match runtime::clean_old_snapshots(&vision_dir, 86400) {
            Ok(count) if count > 0 => {
                platform::log_info(&format!("[VISION] Cleaned up {count} old snapshots."));
            }
            _ => {}
        }
    }

    pub fn vote_on_proposal(
        &mut self,
        proposal_id: &str,
        agent_id: &str,
        vote: bool,
        rationale: &str,
    ) -> Result<(), String> {
        let (new_status, should_execute) = {
            let proposal = self
                .active_proposals
                .get_mut(proposal_id)
                .ok_or("Governance proposal not found")?;
            if proposal.status != platform::governance::ProposalStatus::Pending {
                return Err("Proposal is no longer active".to_string());
            }

            if vote {
                proposal.votes_yes += 1;
            } else {
                proposal.votes_no += 1;
            }

            let engine = platform::governance::ConsensusEngine::new(0.66);
            let eligible_voters = self.agents.len().max(3); // Assume at least 3 voters for simulation
            proposal.status = engine.evaluate(proposal, eligible_voters);

            let new_status = proposal.status.clone();
            let should_execute = new_status == platform::governance::ProposalStatus::Passed;
            (new_status, should_execute)
        };
        self.publish_event(
            "governance_vote",
            serde_json::json!({
                "proposal_id": proposal_id,
                "agent_id": agent_id,
                "vote": vote,
                "rationale": rationale,
                "new_status": new_status,
            }),
        );

        if should_execute {
            self.execute_governance_proposal(proposal_id)?;
        }

        self.save();
        Ok(())
    }

    pub fn execute_governance_proposal(&mut self, proposal_id: &str) -> Result<(), String> {
        {
            let proposal = self
                .active_proposals
                .get_mut(proposal_id)
                .ok_or("Governance proposal not found")?;
            if proposal.status != platform::governance::ProposalStatus::Passed {
                return Err("Only passed proposals can be executed".to_string());
            }

            // Apply the change based on type
            match proposal.change_type.as_str() {
                "investment_policy" => {
                    if let Ok(new_policy) = serde_json::from_value(proposal.payload.clone()) {
                        self.investment_policy = new_policy;
                    }
                }
                _ => return Err(format!("Unknown proposal type: {}", proposal.change_type)),
            }

            proposal.status = platform::governance::ProposalStatus::Executed;
        }
        self.publish_event(
            "governance_executed",
            serde_json::json!({ "proposal_id": proposal_id }),
        );
        self.save();
        Ok(())
    }

    pub fn veto_proposal(&mut self, proposal_id: &str) -> Result<(), String> {
        {
            let proposal = self
                .active_proposals
                .get_mut(proposal_id)
                .ok_or("Governance proposal not found")?;
            proposal.status = platform::governance::ProposalStatus::Vetoed;
        }
        self.publish_event(
            "governance_vetoed",
            serde_json::json!({ "proposal_id": proposal_id }),
        );
        self.save();
        Ok(())
    }

    pub fn syndicate_knowledge(&mut self, agent_id: &str) {
        let tachy_dir = self.workspace_root.join(".tachy");
        let local_memory = intelligence::AgentMemory::load(&tachy_dir);

        intelligence::MemorySyndicator::syndicate(&local_memory, &mut self.hive_mind, 0.9);

        self.publish_event(
            "hive_mind_updated",
            serde_json::json!({
                "agent_id": agent_id,
                "total_insights": self.hive_mind.shared_insights.len(),
            }),
        );

        self.save();
    }

    pub fn update_reputation(&mut self, agent_id: &str, mission_success: bool, reward: f32) {
        let (new_mode, trust_index) = {
            let score = self
                .agent_reputations
                .entry(agent_id.to_string())
                .or_default();

            score.mission_count += 1;
            if mission_success {
                score.success_rate = (score.success_rate * 0.9) + 0.1;
            } else {
                score.success_rate *= 0.9;
            }
            score.token_efficiency =
                (score.token_efficiency * 0.8) + (reward.clamp(0.0, 1.0) * 0.2);

            let new_mode = platform::reputation::TrustElevator::evaluate_autonomy(score);
            let trust_index = score.calculate_trust_index();
            (new_mode, trust_index)
        };

        self.publish_event(
            "reputation_updated",
            serde_json::json!({
                "agent_id": agent_id,
                "new_trust_index": trust_index,
                "permission_mode": new_mode,
            }),
        );

        self.save();
    }

    pub fn provision_infrastructure(
        &mut self,
        requirements: &platform::compute::NodeSpecs,
    ) -> Result<String, String> {
        let orchestrator = platform::compute::ResourceOrchestrator::new(2.0); // $2/hr max budget
        let node = orchestrator.provision_node(requirements)?;

        self.active_nodes.push(node.clone());
        self.publish_event("infrastructure_provisioned", serde_json::json!(&node));
        self.save();
        Ok(node.id)
    }

    pub fn terminate_infrastructure(&mut self, node_id: &str) -> Result<(), String> {
        let index = self
            .active_nodes
            .iter()
            .position(|n| n.id == node_id)
            .ok_or("Node not found")?;
        let mut node = self.active_nodes.remove(index);
        node.status = platform::compute::NodeStatus::Terminating;

        self.publish_event(
            "infrastructure_terminated",
            serde_json::json!({ "node_id": node_id }),
        );
        self.save();
        Ok(())
    }

    pub fn replicate_daemon(&mut self, node_id: &str) -> Result<String, String> {
        let node = self
            .active_nodes
            .iter()
            .find(|n| n.id == node_id)
            .ok_or("Infrastructure node not found")?;
        let spawner = platform::replication::DaemonSpawner;
        let linker = platform::replication::SwarmLinker;

        let target_addr = format!("{}-{}", node.provider.to_lowercase(), node.id);
        let new_daemon = spawner.spawn_instance(node_id, &target_addr)?;
        linker.link_node("parent-daemon", &new_daemon)?;

        self.swarm_nodes.push(new_daemon.clone());
        self.publish_event("swarm_replicated", serde_json::json!(&new_daemon));
        self.save();
        Ok(new_daemon.id)
    }

    pub fn sync_swarm_health(&mut self) {
        let dead_nodes = platform::replication::HealthMonitor::monitor_swarm(&self.swarm_nodes);
        for id in dead_nodes {
            if let Some(index) = self.swarm_nodes.iter().position(|n| n.id == id) {
                self.swarm_nodes.remove(index);
                self.publish_event("swarm_node_offline", serde_json::json!({ "node_id": id }));
            }
        }
        self.save();
    }

    pub fn trigger_red_alert(&mut self, telemetry: &str) -> Result<(), String> {
        let anomalies = intelligence::AnomalyDetector::scan_telemetry(telemetry);
        if let Some(anomaly) = anomalies
            .into_iter()
            .find(|a| matches!(a.severity, intelligence::CrisisSeverity::RedAlert))
        {
            let playbook = intelligence::PlaybookEngine::select_playbook(&anomaly);
            self.active_crisis = Some(anomaly.clone());

            self.publish_event(
                "red_alert_triggered",
                serde_json::json!({
                    "anomaly": anomaly,
                    "playbook": playbook,
                }),
            );

            // Execute mock playbook: De-risk assets
            self.publish_event("playbook_executing", serde_json::json!({ "action": "Liquidating high-risk positions to EmergencyVault" }));

            self.save();
        }
        Ok(())
    }

    pub fn resolve_crisis(&mut self) {
        if let Some(anomaly) = self.active_crisis.take() {
            self.publish_event("crisis_resolved", serde_json::json!({ "id": anomaly.id }));
            self.save();
        }
    }

    pub fn start_autonomous_tuning(&mut self, domain: &str) -> Result<String, String> {
        let orchestrator = intelligence::TrainerOrchestrator;
        let job_id = orchestrator.start_tuning_job("mock-dataset", domain)?;

        let adapter_id = self.adapter_registry.register("llama-3.2-3b", domain, "");
        self.expert_adapters.push(intelligence::ExpertAdapter {
            id: adapter_id.clone(),
            base_model: "llama-3.2-3b".to_string(),
            domain: domain.to_string(),
            lift_score: 0.0,
            status: intelligence::AdapterStatus::Training,
            adapter_path: String::new(),
            registered_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        });

        self.publish_event(
            "tuning_job_started",
            serde_json::json!({
                "job_id": job_id,
                "domain": domain,
            }),
        );

        self.save();
        Ok(job_id)
    }

    pub fn establish_diplomacy(&mut self, swarm_id: &str, signature: &str) -> Result<(), String> {
        let auth = platform::diplomacy::SwarmAuthenticator;
        let swarm = auth.authenticate_swarm(swarm_id, signature)?;

        self.allied_swarms.push(swarm.clone());
        self.publish_event("diplomacy_established", serde_json::json!(&swarm));
        self.save();
        Ok(())
    }

    pub fn sync_hyper_swarm(&mut self) {
        let discovered = platform::diplomacy::HyperSwarmLinker::discover_allies();
        for swarm in discovered {
            if !self.allied_swarms.iter().any(|s| s.id == swarm.id) {
                self.publish_event("hyper_swarm_discovery", serde_json::json!(&swarm));
            }
        }
    }
}

/// Pure-Rust HMAC-SHA256 — no external crypto crate required.
/// Uses SHA-256 via a compact implementation (fits in < 80 lines).
fn hmac_sha256(key: &[u8], msg: &[u8]) -> String {
    const BLOCK: usize = 64;
    // Prepare key
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        let h = sha256(key);
        k[..32].copy_from_slice(&h);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    // ipad / opad
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    // inner hash
    let mut inner = ipad.to_vec();
    inner.extend_from_slice(msg);
    let inner_hash = sha256(&inner);
    // outer hash
    let mut outer = opad.to_vec();
    outer.extend_from_slice(&inner_hash);
    let result = sha256(&outer);
    // hex encode
    result.iter().map(|b| format!("{b:02x}")).collect()
}

/// Minimal SHA-256 implementation (RFC 6234 compliant).
#[allow(clippy::unreadable_literal)]
fn sha256(msg: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    // Padding
    let bit_len = (msg.len() as u64) * 8;
    let mut padded = msg.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// Constant-time byte slice comparison to prevent timing attacks.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

fn timestamp() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}s", d.as_secs())
}

fn load_persisted_state(path: &PathBuf) -> Option<PersistedState> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::env::temp_dir().join(format!("tachy-daemon-test-{}-{}", std::process::id(), id,))
    }

    #[test]
    fn initializes_daemon_state() {
        let root = temp_root();
        let state = DaemonState::init(root.clone()).expect("should init");
        assert!(!state.config.agent_templates.is_empty());
        assert!(state.agents.is_empty());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn creates_agents_from_templates() {
        let root = temp_root();
        let mut state = DaemonState::init(root.clone()).expect("should init");

        let id = state
            .create_agent("code-reviewer", "review my code")
            .expect("should create");
        assert_eq!(id, "agent-1");
        assert!(state.agents.contains_key("agent-1"));

        let id2 = state
            .create_agent("test-runner", "run tests")
            .expect("should create");
        assert_eq!(id2, "agent-2");

        assert!(state.create_agent("nonexistent", "x").is_err());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn persists_and_restores_agents() {
        let root = temp_root();

        // Create agents and save
        {
            let mut state = DaemonState::init(root.clone()).expect("should init");
            state
                .create_agent("code-reviewer", "review")
                .expect("create");
            state.create_agent("test-runner", "test").expect("create");
            assert_eq!(state.agents.len(), 2);
        }

        // Restore from disk
        {
            let state = DaemonState::init(root.clone()).expect("should init");
            assert_eq!(state.agents.len(), 2);
            assert!(state.agents.contains_key("agent-1"));
            assert!(state.agents.contains_key("agent-2"));
            assert_eq!(state.agent_counter, 2);
        }

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn counter_continues_after_restart() {
        let root = temp_root();

        {
            let mut state = DaemonState::init(root.clone()).expect("should init");
            state.create_agent("code-reviewer", "a").expect("create");
            state.create_agent("code-reviewer", "b").expect("create");
        }

        {
            let mut state = DaemonState::init(root.clone()).expect("should init");
            let id = state.create_agent("code-reviewer", "c").expect("create");
            assert_eq!(id, "agent-3"); // continues from 2, not resets to 1
        }

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn schedules_agents() {
        let root = temp_root();
        let mut state = DaemonState::init(root.clone()).expect("should init");

        let id = state
            .schedule_agent(
                "security-scanner",
                ScheduleRule::Interval { seconds: 3600 },
                "hourly scan",
            )
            .expect("should schedule");
        assert_eq!(id, "task-1");
        assert_eq!(state.scheduler.list_tasks().len(), 1);
        std::fs::remove_dir_all(root).ok();
    }
}
