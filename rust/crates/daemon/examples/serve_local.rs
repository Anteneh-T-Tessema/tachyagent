use std::env;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let addr = args
        .first()
        .cloned()
        .unwrap_or_else(|| "127.0.0.1:7777".to_string());
    let workspace = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().unwrap_or_default());

    let state = daemon::DaemonState::init(workspace.clone()).map_err(|e| e.to_string())?;
    let state = Arc::new(Mutex::new(state));

    eprintln!("Serving raw daemon without CLI license gate.");
    eprintln!("Workspace: {}", workspace.display());
    eprintln!("Listening on: {addr}");

    let rt = tokio::runtime::Runtime::new()?;
    let result = rt.block_on(daemon::serve(&addr, state.clone()));

    if let Ok(s) = state.lock() {
        s.save();
        s.audit_logger.flush();
    }

    result
}
