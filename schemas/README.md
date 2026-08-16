# Versioned contracts

These machine-readable manifests make the implemented database and local
application API explicit. The Rust contract tests verify them against the
live migration set and the exact Tauri command registry.

- `v1/data-model.json` describes the migration boundary plus every live table's
  columns, required fields, primary key, and declared SQL checks.
- `v1/local-api.json` describes every implemented command's mode, named inputs,
  output type, stable errors, and foundational search wire types.

The contract tests open a migrated database and compare the data manifest to
SQLite metadata and constraint SQL. They also compare the API command names to
the exact Tauri registry and verify descriptor completeness and serialized
search-result fields. These are compatibility artifacts, not status lists.

Compatible additions update the v1 manifests and their implementation
together. Breaking wire changes require a new version directory and a
migration guide. Database migration numbers remain independently
forward-only.
