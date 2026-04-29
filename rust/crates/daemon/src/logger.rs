use platform::TachyLogger;
use tokio::sync::broadcast::Sender;
use serde_json::json;

pub struct DashboardLogger {
    pub event_bus: Sender<String>,
}

impl DashboardLogger {
    pub fn new(event_bus: Sender<String>) -> Self {
        Self { event_bus }
    }

    fn publish(&self, level: &str, message: &str) {
        let event = json!({
            "kind": "log",
            "payload": {
                "level": level,
                "message": message,
                "timestamp": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            }
        });
        let _ = self.event_bus.send(format!("event: log\ndata: {}\n\n", event.to_string()));
    }
}

impl TachyLogger for DashboardLogger {
    fn info(&self, message: &str) {
        self.publish("info", message);
        println!("[DASHBOARD][INFO] {}", message);
    }

    fn warn(&self, message: &str) {
        self.publish("warn", message);
        eprintln!("[DASHBOARD][WARN] {}", message);
    }

    fn error(&self, message: &str) {
        self.publish("error", message);
        eprintln!("[DASHBOARD][ERROR] {}", message);
    }
}
