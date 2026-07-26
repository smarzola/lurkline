#!/usr/bin/env python3
"""Fail when tracked text contains credential-shaped, non-synthetic values."""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
FORBIDDEN_NAMES = {".env"}
FORBIDDEN_SUFFIXES = (".har", ".har.gz", ".pem", ".p12", ".pfx")
KNOWN_SYNTHETIC_SLACK_VALUES = frozenset(
    {
        "xoxb-bot",
        "xoxc-alpha",
        "xoxc-bad%Q1",
        "xoxc-before--boundary-after",
        "xoxc-beta",
        "xoxc-cli-test-secret",
        "xoxc-concurrent",
        "xoxc-first",
        "xoxc-hidden-detail",
        "xoxc-initial",
        "xoxc-mcp-test-secret",
        "xoxc-new",
        "xoxc-old-token",
        "xoxc-one",
        "xoxc-origin-secret",
        "xoxc-original",
        "xoxc-other",
        "xoxc-partial-secret",
        "xoxc-process-secret",
        "xoxc-refreshed",
        "xoxc-replacement",
        "xoxc-second",
        "xoxc-secret",
        "xoxc-should-never-render",
        "xoxc-super-secret",
        "xoxc-test",
        "xoxc-test--boundary--suffix",
        "xoxc-test-secret",
        "xoxc-test-token",
        "xoxc-two",
        "xoxc-zeroized",
        "xoxd-cli-test-secret",
        "xoxd-cookie-secret",
        "xoxd-mcp-test-secret",
        "xoxd-old-cookie",
        "xoxd-origin-secret",
        "xoxd-partial-secret",
        "xoxd-process-secret",
        "xoxd-should-never-render",
        "xoxd-test",
        "xoxd-test-cookie",
        "xoxd-test-secret",
    }
)
RULES = (
    (
        "possible Slack token",
        re.compile(r"\bxox[a-z]-[A-Za-z0-9_%=-]+\b"),
        0,
    ),
    (
        "possible Slack d cookie",
        re.compile(r"(?i)(?:^|[;,\s'\"])d=((?:xoxd-)?[A-Za-z0-9%._~-]{40,})"),
        1,
    ),
    (
        "possible private key",
        re.compile(r"-----BEGIN (?:[A-Z0-9 ]+ )?PRIVATE KEY-----"),
        None,
    ),
)


def tracked_paths() -> list[Path]:
    result = subprocess.run(
        ["git", "-C", str(ROOT), "ls-files", "-z"],
        check=True,
        capture_output=True,
    )
    return [
        ROOT / value.decode("utf-8")
        for value in result.stdout.split(b"\0")
        if value
    ]


def is_forbidden(path: Path) -> bool:
    relative = path.relative_to(ROOT)
    name = relative.name
    return (
        name in FORBIDDEN_NAMES
        or name.startswith(".env.")
        or str(relative).lower().endswith(FORBIDDEN_SUFFIXES)
    )


def scan_text(text: str) -> list[tuple[int, str]]:
    findings = []
    for line_number, line in enumerate(text.splitlines(), start=1):
        for label, pattern, value_group in RULES:
            for match in pattern.finditer(line):
                candidate = (
                    match.group(value_group)
                    if value_group is not None
                    else None
                )
                if candidate in KNOWN_SYNTHETIC_SLACK_VALUES:
                    continue
                findings.append((line_number, label))
    return findings


def scan(path: Path) -> list[tuple[int, str]]:
    data = path.read_bytes()
    if b"\0" in data:
        return []
    return scan_text(data.decode("utf-8", errors="strict"))


def self_test() -> None:
    allowed = "xoxc-test-token"
    embedded_marker = "xoxc-" + ("a" * 36) + "test" + ("b" * 36)
    multi_segment = "xo" + "xc-12-34-" + ("c" * 40)
    long_cookie = "d=" + ("d" * 24) + "example" + ("e" * 24)
    private_key = "-----BEGIN " + "PRIVATE " + "KEY-----"

    assert scan_text(allowed) == []
    assert scan_text(embedded_marker) == [(1, "possible Slack token")]
    assert scan_text(multi_segment) == [(1, "possible Slack token")]
    assert scan_text(long_cookie) == [(1, "possible Slack d cookie")]
    assert scan_text(private_key) == [(1, "possible private key")]


def main() -> int:
    self_test()
    if sys.argv[1:] == ["--self-test"]:
        print("credential scan self-test passed")
        return 0
    if sys.argv[1:]:
        print("usage: check-no-secrets.py [--self-test]", file=sys.stderr)
        return 2

    findings: list[str] = []
    for path in tracked_paths():
        relative = path.relative_to(ROOT)
        if is_forbidden(path):
            findings.append(f"{relative}: forbidden secret-bearing file type")
            continue
        try:
            matches = scan(path)
        except UnicodeDecodeError:
            continue
        for line_number, label in matches:
            findings.append(f"{relative}:{line_number}: {label}")

    if findings:
        print("credential scan failed:", file=sys.stderr)
        for finding in findings:
            print(f"- {finding}", file=sys.stderr)
        return 1
    print("credential scan passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
