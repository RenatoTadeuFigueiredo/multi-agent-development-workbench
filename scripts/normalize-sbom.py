#!/usr/bin/env python3
"""Remove checkout-specific workspace paths from CycloneDX JSON documents."""

from __future__ import annotations

import json
import pathlib
import sys


def normalize(value: object, source_prefix: str) -> tuple[object, int]:
    if isinstance(value, dict):
        replacements = 0
        normalized: dict[str, object] = {}
        for key, child in value.items():
            normalized_child, child_replacements = normalize(child, source_prefix)
            normalized[key] = normalized_child
            replacements += child_replacements
        return normalized, replacements
    if isinstance(value, list):
        replacements = 0
        normalized_items: list[object] = []
        for child in value:
            normalized_child, child_replacements = normalize(child, source_prefix)
            normalized_items.append(normalized_child)
            replacements += child_replacements
        return normalized_items, replacements
    if isinstance(value, str) and value.startswith(source_prefix):
        return f"workspace:{value.removeprefix(source_prefix)}", 1
    return value, 0


def main() -> None:
    if len(sys.argv) != 3:
        print("Usage: normalize-sbom.py <repository-root> <directory>", file=sys.stderr)
        raise SystemExit(2)
    repository_root = pathlib.Path(sys.argv[1]).resolve()
    directory = pathlib.Path(sys.argv[2])
    source_prefix = f"path+{repository_root.as_uri()}/"
    paths = sorted(directory.glob("*.cdx.json"))
    if not paths:
        print("No CycloneDX documents to normalize.", file=sys.stderr)
        raise SystemExit(1)
    for path in paths:
        document = json.loads(path.read_text(encoding="utf-8"))
        normalized, replacements = normalize(document, source_prefix)
        if replacements == 0:
            print(f"No workspace references found in {path}.", file=sys.stderr)
            raise SystemExit(1)
        path.write_text(
            json.dumps(normalized, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )


if __name__ == "__main__":
    main()
