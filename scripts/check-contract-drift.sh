#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture_root="${repository_root}/crates/workbench-testkit/fixtures/generated"

sources=(
  "doc/arch/contracts/workbench-local-protocol.yaml"
  "doc/arch/datamodels/provider-capabilities.schema.json"
  "doc/arch/datamodels/session-event.schema.json"
  "doc/arch/datamodels/session-key-envelope.schema.json"
  "doc/arch/datamodels/workbench-configuration.schema.json"
  "doc/arch/datamodels/workbench-lock.schema.json"
  "doc/arch/schemas/build-the-workbench-orchestration-kernel-foundation-as-a.cue"
  "doc/arch/statecharts/session-lifecycle.md"
)

for source in "${sources[@]}"; do
  generated="${fixture_root}/$(basename "${source}")"
  if [[ ! -f "${generated}" ]]; then
    echo "Missing generated fixture: ${generated}" >&2
    exit 1
  fi
  if ! cmp --silent "${repository_root}/${source}" "${generated}"; then
    echo "Contract fixture drift: ${source}" >&2
    echo "Run scripts/generate-contract-fixtures.sh and review the result." >&2
    exit 1
  fi
done
