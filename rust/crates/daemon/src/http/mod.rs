//! HTTP server module.
//!
//! Sub-module layout:
//!   types   — shared response/payload types
//!   utils   — timestamps, auth, RBAC, HTTP parsing
//!   server  — TCP accept loop + simple inline handlers
//!   router  — request dispatcher + dynamic parameterised routes
//!   agent / auth / governance / intel / runs / webhooks / workers
//!           — domain-specific handler sub-modules

mod agent;
mod auth;
mod feedback;
mod governance;
mod intel;
mod runs;
mod webhooks;
mod workers;
mod yaya;
mod swarm;
mod sync;
mod vision;

mod types;
mod utils;
mod server;
mod router;

// Re-export the public server entry point.
pub use server::serve;

// Re-export shared types and utilities so existing sub-modules can continue
// to use `use super::{Response, ErrorResponse, chrono_now_secs, ...}` unchanged.
pub(crate) use types::{AgentInfo, ErrorResponse, Response};
pub(crate) use utils::{
    chrono_now_secs, chrono_now_str, csv_response, truncate_completion, urlencoding_decode,
};

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use audit::TieredRateLimiter;

    use crate::state::DaemonState;
    use super::router::handle_request;
    use super::types::Response;
    use super::utils::chrono_now_secs;
    use audit::{User, Role, hash_api_key};


    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn health_endpoint_works() {
        let root = std::env::temp_dir().join(format!("tachy-test-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(TieredRateLimiter::new()));
        let res = handle_request("GET /health HTTP/1.1\r\n\r\n", &state, &limiter, "127.0.0.1").await;
        if let Response::Full { body, .. } = res {
            let body = String::from_utf8_lossy(&body);
            assert!(body.contains("\"status\":\"ok\"") || body.contains("\"status\": \"ok\""));
        } else {
            panic!("not full");
        }
    }

    #[test]
    fn decode_plain_string() {
        use super::utils::urlencoding_decode;
        assert_eq!(urlencoding_decode("hello"), "hello");
    }

    #[test]
    fn decode_plus_as_space() {
        use super::utils::urlencoding_decode;
        assert_eq!(urlencoding_decode("hello+world"), "hello world");
    }

    #[test]
    fn decode_percent_encoded() {
        use super::utils::urlencoding_decode;
        assert_eq!(urlencoding_decode("hello%20world"), "hello world");
        assert_eq!(urlencoding_decode("a%2Fb"), "a/b");
    }

    #[test]
    fn decode_mixed_encoding() {
        use super::utils::urlencoding_decode;
        assert_eq!(urlencoding_decode("fn+main%28%29"), "fn main()");
    }

    #[test]
    fn decode_incomplete_percent_sequence_kept_as_is() {
        use super::utils::urlencoding_decode;
        let result = urlencoding_decode("abc%");
        assert!(result.starts_with("abc"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn search_missing_query_returns_400() {
        let root = std::env::temp_dir().join(format!("tachy-search-test-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(TieredRateLimiter::new()));
        let res = handle_request(
            "GET /api/search HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, body, .. } = res {
            assert_eq!(status, 400);
            let body = String::from_utf8_lossy(&body);
            assert!(body.contains("missing query"));
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_policy_returns_200_with_defaults() {
        let root = std::env::temp_dir().join(format!("tachy-policy-test-{}", chrono_now_secs()));
        let mut state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let admin_key = "admin-key";
        state.identity.user_store.add_user(User {
            id: "admin".to_string(),
            name: "Admin".to_string(),
            role: Role::Admin,
            api_key_hash: hash_api_key(admin_key),
            created_at: "now".to_string(),
            enabled: true, active_team_id: None,
        });
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(TieredRateLimiter::new()));
        let res = handle_request(
            &format!("GET /api/policy HTTP/1.1\r\nAuthorization: Bearer {admin_key}\r\n\r\n"),
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, body, .. } = res {
            assert_eq!(status, 200);
            assert!(serde_json::from_slice::<serde_json::Value>(&body).is_ok());
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn set_policy_invalid_json_returns_400() {
        let root = std::env::temp_dir().join(format!("tachy-set-policy-test-{}", chrono_now_secs()));
        let mut state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let admin_key = "admin-key";
        state.identity.user_store.add_user(User {
            id: "admin".to_string(),
            name: "Admin".to_string(),
            role: Role::Admin,
            api_key_hash: hash_api_key(admin_key),
            created_at: "now".to_string(),
            enabled: true, active_team_id: None,
        });
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(TieredRateLimiter::new()));
        let raw = format!("POST /api/policy HTTP/1.1\r\nAuthorization: Bearer {admin_key}\r\nContent-Length: 11\r\n\r\nnot-valid{{");
        let res = handle_request(&raw, &state, &limiter, "127.0.0.1").await;
        if let Response::Full { status, .. } = res {
            assert_eq!(status, 400);
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_conversation_unknown_returns_404() {
        let root = std::env::temp_dir().join(format!("tachy-get-conv-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(TieredRateLimiter::new()));
        let res = handle_request(
            "GET /api/conversations/conv-999 HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, .. } = res {
            assert_eq!(status, 404);
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_conversation_existing_returns_200() {
        let root = std::env::temp_dir().join(format!("tachy-get-conv2-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            let mut s = DaemonState::init(root).expect("init");
            let _id = s.create_conversation("test conv");
            s
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(TieredRateLimiter::new()));
        let res = handle_request(
            "GET /api/conversations/conv-1 HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, body, .. } = res {
            assert_eq!(status, 200);
            let body = String::from_utf8_lossy(&body);
            assert!(body.contains("conv-1") || body.contains("test conv"));
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn delete_conversation_unknown_returns_404() {
        let root = std::env::temp_dir().join(format!("tachy-del-conv-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(TieredRateLimiter::new()));
        let res = handle_request(
            "DELETE /api/conversations/conv-999 HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, .. } = res {
            assert_eq!(status, 404);
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn delete_conversation_existing_returns_204() {
        let root = std::env::temp_dir().join(format!("tachy-del-conv2-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            let mut s = DaemonState::init(root).expect("init");
            let _id = s.create_conversation("to delete");
            s
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(TieredRateLimiter::new()));
        let res = handle_request(
            "DELETE /api/conversations/conv-1 HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, .. } = res {
            assert_eq!(status, 204);
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn delete_agent_unknown_returns_404() {
        let root = std::env::temp_dir().join(format!("tachy-del-ag-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(TieredRateLimiter::new()));
        let res = handle_request(
            "DELETE /api/agents/agent-999 HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, .. } = res {
            assert_eq!(status, 404);
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn delete_agent_existing_returns_204() {
        let root = std::env::temp_dir().join(format!("tachy-del-ag2-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            let mut s = DaemonState::init(root).expect("init");
            let _id = s.create_agent("code-reviewer", "do stuff").expect("create");
            s
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(TieredRateLimiter::new()));
        let res = handle_request(
            "DELETE /api/agents/agent-1 HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, .. } = res {
            assert_eq!(status, 204);
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_agent_unknown_returns_404() {
        let root = std::env::temp_dir().join(format!("tachy-cancel-ag-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(TieredRateLimiter::new()));
        let res = handle_request(
            "POST /api/agents/agent-999/cancel HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, .. } = res {
            assert_eq!(status, 404);
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_agent_existing_returns_200() {
        let root = std::env::temp_dir().join(format!("tachy-cancel-ag2-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            let mut s = DaemonState::init(root).expect("init");
            let _id = s.create_agent("code-reviewer", "cancellable task").expect("create");
            s
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(TieredRateLimiter::new()));
        let res = handle_request(
            "POST /api/agents/agent-1/cancel HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, body, .. } = res {
            assert_eq!(status, 200);
            let body = String::from_utf8_lossy(&body);
            assert!(body.contains("Failed") || body.contains("agent-1"));
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn index_status_returns_200() {
        let root = std::env::temp_dir().join(format!("tachy-index-status-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(TieredRateLimiter::new()));
        let res = handle_request(
            "GET /api/index HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, body, .. } = res {
            assert_eq!(status, 200);
            let body = String::from_utf8_lossy(&body);
            assert!(body.contains("status"));
        } else {
            panic!("expected Full response");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn event_stream_returns_sse_content_type() {
        let root = std::env::temp_dir().join(format!("tachy-events-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(TieredRateLimiter::new()));
        let res = handle_request(
            "GET /api/events HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        match res {
            Response::Stream { status, content_type, .. } => {
                assert_eq!(status, 200);
                assert_eq!(content_type, "text/event-stream");
            }
            _ => panic!("expected Stream response for /api/events"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn publish_event_reaches_subscriber() {
        let root = std::env::temp_dir().join(format!("tachy-pub-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let mut rx = state.event_bus.subscribe();
        state.publish_event("test_event", serde_json::json!({"x": 1}));
        let msg = rx.try_recv().expect("should have buffered message");
        assert!(msg.contains("test_event"));
        assert!(msg.contains("\"x\":1") || msg.contains("\"x\": 1"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_template_save_and_list() {
        let root = std::env::temp_dir().join(format!("tachy-tpl-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(TieredRateLimiter::new()));

        let body = r#"{"name":"refactor","description":"Standard refactor","tasks":[{"template":"chat","prompt":"refactor src/lib.rs"}],"max_concurrency":2}"#;
        let res = handle_request(
            &format!("POST /api/run-templates HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}", body.len()),
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, body, .. } = res {
            assert_eq!(status, 201);
            let body = String::from_utf8_lossy(&body);
            assert!(body.contains("refactor"));
        } else { panic!("expected Full response"); }

        let res = handle_request(
            "GET /api/run-templates HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, body, .. } = res {
            assert_eq!(status, 200);
            let body = String::from_utf8_lossy(&body);
            assert!(body.contains("refactor"));
            assert!(body.contains("\"count\":1"));
        } else { panic!("expected Full response"); }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_template_get_and_delete() {
        let root = std::env::temp_dir().join(format!("tachy-tpl2-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(TieredRateLimiter::new()));

        let body = r#"{"name":"myflow","tasks":[{"template":"chat","prompt":"do x"}]}"#;
        handle_request(
            &format!("POST /api/run-templates HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}", body.len()),
            &state, &limiter, "127.0.0.1",
        ).await;

        let res = handle_request(
            "GET /api/run-templates/myflow HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, body, .. } = res {
            assert_eq!(status, 200);
            let body = String::from_utf8_lossy(&body);
            assert!(body.contains("myflow"));
        } else { panic!("expected Full"); }

        let res = handle_request(
            "DELETE /api/run-templates/myflow HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, .. } = res { assert_eq!(status, 200); }
        else { panic!("expected Full"); }

        let res = handle_request(
            "GET /api/run-templates/myflow HTTP/1.1\r\n\r\n",
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, .. } = res { assert_eq!(status, 404); }
        else { panic!("expected Full"); }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_template_missing_name_returns_400() {
        let root = std::env::temp_dir().join(format!("tachy-tpl3-{}", chrono_now_secs()));
        let state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(TieredRateLimiter::new()));
        let body = r#"{"name":"","tasks":[{"template":"chat","prompt":"x"}]}"#;
        let res = handle_request(
            &format!("POST /api/run-templates HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}", body.len()),
            &state, &limiter, "127.0.0.1",
        ).await;
        if let Response::Full { status, .. } = res { assert_eq!(status, 400); }
        else { panic!("expected Full"); }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn high_risk_operations_are_gated() {
        let root = std::env::temp_dir().join(format!("tachy-gov-gate-{}", chrono_now_secs()));
        let mut state = tokio::task::spawn_blocking(move || {
            DaemonState::init(root).expect("init")
        }).await.expect("spawn_blocking");
        
        let viewer_key = "viewer-key";
        state.identity.user_store.add_user(User {
            id: "viewer".to_string(),
            name: "Viewer".to_string(),
            role: Role::Viewer,
            api_key_hash: hash_api_key(viewer_key),
            created_at: "now".to_string(),
            enabled: true, active_team_id: None,
        });
        
        let state = Arc::new(Mutex::new(state));
        let limiter = Arc::new(Mutex::new(TieredRateLimiter::new()));
        let client_ip = "127.0.0.1";

        // 1. /api/policy (ManageGovernance)
        let req = format!("GET /api/policy HTTP/1.1\r\nAuthorization: Bearer {viewer_key}\r\n\r\n");
        let res = handle_request(&req, &state, &limiter, client_ip).await;
        if let Response::Full { status, body, .. } = res {
            assert_eq!(status, 403);
            let body = String::from_utf8_lossy(&body);
            assert!(body.contains("Viewer cannot perform ManageGovernance"));
        } else { panic!("not full"); }

        // 2. /api/models/pull (ManageModels)
        let req = format!("POST /api/models/pull HTTP/1.1\r\nAuthorization: Bearer {viewer_key}\r\n\r\n{{}}");
        let res = handle_request(&req, &state, &limiter, client_ip).await;
        if let Response::Full { status, body, .. } = res {
            assert_eq!(status, 403);
            let body = String::from_utf8_lossy(&body);
            assert!(body.contains("Viewer cannot perform ManageModels"));
        } else { panic!("not full"); }

        // 3. /api/webhooks (ManageWebhooks)
        let req = format!("GET /api/webhooks HTTP/1.1\r\nAuthorization: Bearer {viewer_key}\r\n\r\n");
        let res = handle_request(&req, &state, &limiter, client_ip).await;
        if let Response::Full { status, body, .. } = res {
            assert_eq!(status, 403);
            let body = String::from_utf8_lossy(&body);
            assert!(body.contains("Viewer cannot perform ManageWebhooks"));
        } else { panic!("not full"); }
    }
}
