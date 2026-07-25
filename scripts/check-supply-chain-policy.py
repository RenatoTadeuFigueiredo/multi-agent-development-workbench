#!/usr/bin/env python3
"""Validate committed supply-chain policy without network access."""

from __future__ import annotations

import pathlib
import re
import sys
import tomllib


REPOSITORY_ROOT = pathlib.Path(__file__).resolve().parent.parent
APPROVED_LICENSES = {
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "BSL-1.0",
    "CC0-1.0",
    "ISC",
    "MIT",
    "MIT-0",
    "Unicode-3.0",
    "Unlicense",
    "Zlib",
}
CRATES_IO_INDEX = "https://github.com/rust-lang/crates.io-index"
EXACT_VERSION = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+\Z")
ACTION_REFERENCE = re.compile(r"[^@\s]+@[0-9a-fA-F]{40}\Z")
USES_LINE = re.compile(r"^\s*(?:-\s+)?uses:\s+(\S+)")


def fail(message: str) -> None:
    print(f"Supply-chain policy error: {message}", file=sys.stderr)
    raise SystemExit(1)


def load_policy() -> dict[str, object]:
    policy_path = REPOSITORY_ROOT / "deny.toml"
    try:
        return tomllib.loads(policy_path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        fail(f"cannot parse deny.toml: {error}")


def validate_policy(policy: dict[str, object]) -> None:
    advisories = policy.get("advisories")
    licenses = policy.get("licenses")
    sources = policy.get("sources")
    if not isinstance(advisories, dict):
        fail("deny.toml must define [advisories]")
    if not isinstance(licenses, dict):
        fail("deny.toml must define [licenses]")
    if not isinstance(sources, dict):
        fail("deny.toml must define [sources]")

    if advisories.get("ignore") != []:
        fail("advisory exceptions require an explicit policy review")
    for key in ("yanked", "unmaintained", "unsound"):
        expected = "deny" if key == "yanked" else "all"
        if advisories.get(key) != expected:
            fail(f"advisories.{key} must be {expected!r}")

    allowed = licenses.get("allow")
    if not isinstance(allowed, list) or set(allowed) != APPROVED_LICENSES:
        fail("licenses.allow differs from the reviewed permissive-license set")

    if sources.get("unknown-registry") != "deny":
        fail("sources.unknown-registry must be 'deny'")
    if sources.get("unknown-git") != "deny":
        fail("sources.unknown-git must be 'deny'")
    if sources.get("allow-registry") != [CRATES_IO_INDEX]:
        fail("only the crates.io registry may be allowed")
    if sources.get("allow-git") != []:
        fail("git dependencies require an explicit source-policy change")


def validate_tool_versions() -> None:
    versions_path = REPOSITORY_ROOT / "security" / "supply-chain-tools.env"
    versions: dict[str, str] = {}
    try:
        lines = versions_path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        fail(f"cannot read pinned tool versions: {error}")
    for line in lines:
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if not separator or not key or not EXACT_VERSION.fullmatch(value):
            fail(f"tool version must be an exact semantic version: {line!r}")
        versions[key] = value
    required = {"CARGO_DENY_VERSION", "CARGO_CYCLONEDX_VERSION"}
    if versions.keys() != required:
        fail(f"pinned tool set must be exactly {sorted(required)}")


def validate_action_pins() -> None:
    workflow = REPOSITORY_ROOT / ".github" / "workflows" / "ci.yml"
    try:
        lines = workflow.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        fail(f"cannot read GitHub workflow: {error}")
    found = 0
    for line_number, line in enumerate(lines, start=1):
        match = USES_LINE.match(line)
        if match is None:
            continue
        reference = match.group(1)
        if reference.startswith("./"):
            continue
        found += 1
        if ACTION_REFERENCE.fullmatch(reference) is None:
            fail(
                f"{workflow.relative_to(REPOSITORY_ROOT)}:{line_number} "
                "must pin the action to a 40-character commit SHA"
            )
    if found == 0:
        fail("GitHub workflow must contain at least one pinned action")


def main() -> None:
    if not (REPOSITORY_ROOT / "Cargo.lock").is_file():
        fail("Cargo.lock is required")
    validate_policy(load_policy())
    validate_tool_versions()
    validate_action_pins()
    print("Supply-chain policy is internally consistent.")


if __name__ == "__main__":
    main()
