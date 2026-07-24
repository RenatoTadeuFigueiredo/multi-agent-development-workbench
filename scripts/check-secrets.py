#!/usr/bin/env python3
"""Scan tracked and unignored working-tree text for high-confidence secrets."""

from __future__ import annotations

import math
import pathlib
import re
import subprocess
import sys


REPOSITORY_ROOT = pathlib.Path(__file__).resolve().parent.parent
MAX_FILE_BYTES = 2 * 1024 * 1024
PLACEHOLDER_WORDS = ("example", "placeholder", "redacted", "dummy", "replace", "changeme")
TOKEN_PATTERNS = (
    re.compile(r"\bsk-(?:proj-|svcacct-)?[A-Za-z0-9_-]{20,}\b"),
    re.compile(r"\bsk-ant-(?:api\d{2}-)?[A-Za-z0-9_-]{20,}\b"),
    re.compile(r"\bsk-or-v1-[0-9a-fA-F]{32,}\b"),
    re.compile(r"\bgsk_[A-Za-z0-9]{20,}\b"),
    re.compile(r"\bxai-[A-Za-z0-9_-]{20,}\b"),
    re.compile(r"\bhf_[A-Za-z0-9]{30,}\b"),
    re.compile(r"\bsk_live_[A-Za-z0-9]{24,}\b"),
    re.compile(r"\bgh[pousr]_[A-Za-z0-9]{30,}\b"),
    re.compile(r"\bgithub_pat_[A-Za-z0-9_]{50,}\b"),
    re.compile(r"\bglpat-[A-Za-z0-9_-]{20,}\b"),
    re.compile(r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
    re.compile(r"\bAIza[0-9A-Za-z_-]{35}\b"),
    re.compile(r"\bxox[baprs]-[A-Za-z0-9-]{20,}\b"),
)
PRIVATE_KEY = re.compile(
    r"-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----"
)


def candidate_files() -> list[pathlib.Path]:
    result = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=REPOSITORY_ROOT,
        check=True,
        capture_output=True,
    )
    return [
        REPOSITORY_ROOT / pathlib.Path(raw.decode("utf-8"))
        for raw in result.stdout.split(b"\0")
        if raw
    ]


def looks_like_placeholder(candidate: str) -> bool:
    if any(word in candidate.lower() for word in PLACEHOLDER_WORDS):
        return True
    payload = re.sub(r"^[A-Za-z-]+(?:v1-)?", "", candidate)
    payload = re.sub(r"[^A-Za-z0-9]", "", payload)
    if not payload or len(set(payload.lower())) < 8:
        return True
    frequencies = {character: payload.count(character) for character in set(payload)}
    entropy = -sum(
        (count / len(payload)) * math.log2(count / len(payload))
        for count in frequencies.values()
    )
    return entropy < 2.5


def scan(path: pathlib.Path) -> list[tuple[int, str]]:
    try:
        if not path.is_file() or path.stat().st_size > MAX_FILE_BYTES:
            return []
        data = path.read_bytes()
    except OSError:
        return []
    if b"\0" in data:
        return []
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        return []

    findings: list[tuple[int, str]] = []
    lines = text.splitlines()
    for line_number, line in enumerate(lines, start=1):
        context = "\n".join(lines[max(0, line_number - 2) : line_number + 1])
        for pattern in TOKEN_PATTERNS:
            for match in pattern.finditer(line):
                if not looks_like_placeholder(match.group(0)):
                    findings.append((line_number, "provider or cloud token"))
        if PRIVATE_KEY.search(line) and not any(
            word in context.lower() for word in PLACEHOLDER_WORDS
        ):
            findings.append((line_number, "private key"))
    return findings


def main() -> None:
    findings: list[tuple[pathlib.Path, int, str]] = []
    for path in candidate_files():
        for line_number, kind in scan(path):
            findings.append((path.relative_to(REPOSITORY_ROOT), line_number, kind))
    if findings:
        for path, line_number, kind in findings:
            print(f"Potential {kind}: {path}:{line_number}", file=sys.stderr)
        raise SystemExit(1)
    print("No high-confidence plaintext secrets found.")


if __name__ == "__main__":
    main()
