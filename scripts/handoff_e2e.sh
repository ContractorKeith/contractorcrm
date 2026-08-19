#!/usr/bin/env bash
# Exercises the won-opportunity -> ContractorProject hand-off end to end:
# builds the sibling's handoff-import binary, then runs the ignored CRM
# integration test against it. Requires a ContractorProject checkout.
#
# Usage: scripts/handoff_e2e.sh [CONTRACTORPROJECT_DIR=../contractorproject]
set -euo pipefail

crm_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
sibling_dir="${CONTRACTORPROJECT_DIR:-${crm_dir}/../contractorproject}"

if [[ ! -f "${sibling_dir}/src-tauri/Cargo.toml" ]]; then
  echo "handoff_e2e: no ContractorProject checkout at ${sibling_dir}" >&2
  echo "handoff_e2e: set CONTRACTORPROJECT_DIR to its path" >&2
  exit 1
fi

echo "==> building handoff-import in ${sibling_dir}"
cargo build --manifest-path "${sibling_dir}/src-tauri/Cargo.toml" --bin handoff-import

binary="${sibling_dir}/src-tauri/target/debug/handoff-import"
if [[ ! -x "${binary}" ]]; then
  echo "handoff_e2e: built binary not found at ${binary}" >&2
  exit 1
fi

echo "==> running the end-to-end hand-off test"
export HANDOFF_IMPORT_BIN="${binary}"
cargo test --manifest-path "${crm_dir}/src-tauri/Cargo.toml" \
  --test handoff_e2e -- --ignored --nocapture

echo "==> hand-off end to end: PASS"
