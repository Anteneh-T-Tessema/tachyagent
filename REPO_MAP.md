# Claw-Code Repo Map

## Canonical Runtime

The canonical implementation is the Rust workspace under `rust/`.

Primary entrypoints:

- `rust/Cargo.toml`
- `rust/crates/daemon`
- `rust/crates/runtime`
- `rust/crates/audit`
- `rust/crates/backend`
- `rust/crates/platform`
- `rust/crates/intelligence`
- `rust/crates/tools`

## Compatibility and Porting Layer

The Python `src/` tree is not the primary production runtime. It should be treated as:

- compatibility and porting support
- metadata and mirror tooling
- historical workspace scaffolding

Primary files:

- `src/main.py`
- `src/runtime.py`
- `src/query_engine.py`
- `src/tools.py`

## Public Integration Surfaces

- HTTP API: `rust/openapi.json`
- Python SDK: `sdk/python/`
- VS Code extension: `vscode-extension/`

## Canonical Documentation

- High-level architecture: `ARCHITECTURE.md`
- System documentation: `SYSTEM_DOCUMENTATION.md`
- Deep subsystem memo: `docs/DEEP_ARCHITECTURE_MEMO.md`
- Yaya integration design: `docs/YAYA_TACHY_INTEGRATION.md`
- Cleanup and hardening plan: `docs/REPO_CLEANUP_HARDENING_PLAN.md`

## Archived Duplicate Artifacts

Duplicate docs and manifests that should not be treated as canonical are moved under:

- `archive/legacy_duplicates/`

## Notes

- If a top-level document conflicts with the Rust workspace, trust the Rust implementation.
- If the Python tree conflicts with the Rust tree, treat Rust as canonical.
