# Versioned contracts

These machine-readable manifests make the implemented database, local
application API, and suite hand-off file explicit. The Rust contract tests
verify them against the live migration set, the exact Tauri command registry,
and real exported files.

- `v1/data-model.json` describes the migration boundary plus every live table's
  columns, required fields, primary key, foreign keys, and declared SQL checks.
- `v1/local-api.json` describes every implemented command's mode, named inputs,
  output type, stable errors, and foundational search wire types.
- `v1/handoff-envelope.json` freezes the opportunity hand-off envelope written
  by `export_handoff_envelope` — schema version, kind, product stamp, and the
  full opportunity, contact, company, money, and reference shapes, including
  which fields may be null (docs/HANDOFF.md).

The contract tests open a migrated database and compare the data manifest to
SQLite metadata, foreign-key metadata, and constraint SQL. They also compare the API command names to
the exact Tauri registry and verify descriptor completeness and serialized
search-result fields. For the envelope they export real files from a seeded
database and match them field for field: an unpinned field fails the test, so
envelope additions are always deliberate. These are compatibility artifacts,
not status lists.

Compatible additions update the v1 manifests and their implementation
together. Breaking wire changes require a new version directory and a
migration guide. Database migration numbers remain independently
forward-only.
