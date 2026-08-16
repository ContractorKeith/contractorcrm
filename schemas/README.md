# Versioned contracts

These machine-readable manifests make the implemented database and local
application API explicit. The Rust contract tests verify them against the
live migration set and the exact Tauri command registry.

- `v1/data-model.json` describes the current database migration boundary and
  canonical table set.
- `v1/local-api.json` describes the implemented local command surface and its
  stable error kinds.

Compatible additions update the v1 manifests and their implementation
together. Breaking wire changes require a new version directory and a
migration guide. Database migration numbers remain independently
forward-only.
