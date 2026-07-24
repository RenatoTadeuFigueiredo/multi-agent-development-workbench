#!/usr/bin/env python3
"""Perform deterministic structural validation of CycloneDX JSON output."""

from __future__ import annotations

import json
import pathlib
import sys


REPOSITORY_ROOT = pathlib.Path(__file__).resolve().parent.parent
PINNED_TOOLS = REPOSITORY_ROOT / "security" / "supply-chain-tools.env"


def fail(path: pathlib.Path, message: str) -> None:
    print(f"Invalid CycloneDX SBOM {path}: {message}", file=sys.stderr)
    raise SystemExit(1)


def validate(path: pathlib.Path) -> None:
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(path, str(error))
    if document.get("bomFormat") != "CycloneDX":
        fail(path, "bomFormat must be CycloneDX")
    if document.get("specVersion") != "1.5":
        fail(path, "specVersion must be 1.5")
    if not isinstance(document.get("version"), int) or document["version"] < 1:
        fail(path, "version must be a positive integer")

    metadata = document.get("metadata")
    if not isinstance(metadata, dict) or not isinstance(metadata.get("component"), dict):
        fail(path, "metadata.component is required")
    if metadata.get("timestamp") != "1970-01-01T00:00:00.000000000Z":
        fail(path, "timestamp must use the reproducible SOURCE_DATE_EPOCH")
    if "serialNumber" in document:
        fail(path, "reproducible output must not contain a serialNumber")
    expected_version = next(
        line.removeprefix("CARGO_CYCLONEDX_VERSION=")
        for line in PINNED_TOOLS.read_text(encoding="utf-8").splitlines()
        if line.startswith("CARGO_CYCLONEDX_VERSION=")
    )
    tools = metadata.get("tools")
    if not isinstance(tools, list) or not any(
        isinstance(tool, dict)
        and tool.get("name") == "cargo-cyclonedx"
        and tool.get("version") == expected_version
        for tool in tools
    ):
        fail(path, f"cargo-cyclonedx tool version must be {expected_version}")
    components = document.get("components")
    dependencies = document.get("dependencies")
    if not isinstance(components, list) or not components:
        fail(path, "components must be a non-empty array")
    if not isinstance(dependencies, list):
        fail(path, "dependencies must be an array")

    all_components = [metadata["component"], *components]
    references: set[str] = set()
    for component in all_components:
        if not isinstance(component, dict):
            fail(path, "every component must be an object")
        reference = component.get("bom-ref")
        if not isinstance(reference, str) or not reference:
            fail(path, "every component must have a non-empty bom-ref")
        if reference.startswith("path+file://"):
            fail(path, "component references must not disclose checkout paths")
        if reference in references:
            fail(path, f"duplicate component bom-ref {reference!r}")
        references.add(reference)
        if not isinstance(component.get("name"), str) or not component["name"]:
            fail(path, "every component must have a name")
        if not isinstance(component.get("version"), str) or not component["version"]:
            fail(path, "every component must have a version")

    dependency_references: set[str] = set()
    for dependency in dependencies:
        if not isinstance(dependency, dict):
            fail(path, "every dependency must be an object")
        reference = dependency.get("ref")
        children = dependency.get("dependsOn", [])
        if isinstance(reference, str) and reference.startswith("path+file://"):
            fail(path, "dependency references must not disclose checkout paths")
        if reference not in references:
            fail(path, f"dependency ref {reference!r} has no component")
        if reference in dependency_references:
            fail(path, f"duplicate dependency ref {reference!r}")
        dependency_references.add(reference)
        if not isinstance(children, list) or any(
            child not in references for child in children
        ):
            fail(path, f"dependency {reference!r} has an unknown child")


def main() -> None:
    if len(sys.argv) != 2:
        print("Usage: validate-sbom.py <directory>", file=sys.stderr)
        raise SystemExit(2)
    directory = pathlib.Path(sys.argv[1])
    paths = sorted(directory.glob("*.cdx.json"))
    if not paths:
        fail(directory, "no *.cdx.json documents found")
    for path in paths:
        validate(path)
    print(f"Validated {len(paths)} CycloneDX 1.5 SBOM document(s).")


if __name__ == "__main__":
    main()
