#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output_directory="${repository_root}/target/sbom"
package_list="$(mktemp "${TMPDIR:-/tmp}/workbench-sbom-packages.XXXXXX")"

cleanup() {
  while IFS=$'\t' read -r manifest_directory package_name; do
    generated="${manifest_directory}/${package_name}.cdx.json"
    if [[ -f "${generated}" ]]; then
      rm -f -- "${generated}"
    fi
  done < "${package_list}"
  rm -f -- "${package_list}"
}
trap cleanup EXIT

cd "${repository_root}"
CARGO_NET_OFFLINE=true cargo metadata --locked --offline --format-version 1 |
  python3 -c '
import json
import pathlib
import sys

metadata = json.load(sys.stdin)
workspace = set(metadata["workspace_members"])
for package in sorted(metadata["packages"], key=lambda item: item["name"]):
    if package["id"] in workspace:
        manifest_directory = pathlib.Path(package["manifest_path"]).parent
        print("{}\t{}".format(manifest_directory, package["name"]))
' > "${package_list}"

while IFS=$'\t' read -r manifest_directory package_name; do
  generated="${manifest_directory}/${package_name}.cdx.json"
  if [[ -e "${generated}" ]]; then
    echo "Refusing to overwrite existing generated path: ${generated}" >&2
    exit 1
  fi
done < "${package_list}"

mkdir -p "${output_directory}"
find "${output_directory}" -maxdepth 1 -type f -name '*.cdx.json' -delete

SOURCE_DATE_EPOCH=0 CARGO_NET_OFFLINE=true cargo cyclonedx \
  --all \
  --all-features \
  --target all \
  --format json \
  --spec-version 1.5

while IFS=$'\t' read -r manifest_directory package_name; do
  generated="${manifest_directory}/${package_name}.cdx.json"
  destination="${output_directory}/${package_name}.cdx.json"
  if [[ ! -f "${generated}" ]]; then
    echo "Expected CycloneDX output is missing: ${generated}" >&2
    exit 1
  fi
  mv -- "${generated}" "${destination}"
done < "${package_list}"

python3 "${repository_root}/scripts/normalize-sbom.py" \
  "${repository_root}" \
  "${output_directory}"
python3 "${repository_root}/scripts/validate-sbom.py" "${output_directory}"
