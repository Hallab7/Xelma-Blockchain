#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Parse adversarial suite test output and emit markdown + JSON reports (Issue #372)."""

from __future__ import annotations

import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

RESULT_RE = re.compile(r"^ADVERSARIAL_RESULT:(\{.*\})\s*$")
SUITE_RE = re.compile(r"^ADVERSARIAL_SUITE:(\{.*\})\s*$")
TEST_OK_RE = re.compile(r"^test .+ \.\.\. ok$")
TEST_FAIL_RE = re.compile(r"^test .+ \.\.\. FAILED$")


def parse_log(log_path: Path) -> tuple[list[dict], dict | None, list[str], list[str]]:
    scenarios: list[dict] = []
    suite_meta: dict | None = None
    passed_tests: list[str] = []
    failed_tests: list[str] = []

    for line in log_path.read_text(encoding="utf-8").splitlines():
        if m := RESULT_RE.match(line):
            scenarios.append(json.loads(m.group(1)))
        elif m := SUITE_RE.match(line):
            suite_meta = json.loads(m.group(1))
        elif m := TEST_OK_RE.match(line):
            passed_tests.append(m.group(0))
        elif m := TEST_FAIL_RE.match(line):
            failed_tests.append(m.group(0))

    return scenarios, suite_meta, passed_tests, failed_tests


def build_report(
    scenarios: list[dict],
    suite_meta: dict | None,
    failed_tests: list[str],
) -> dict:
    critical = [s for s in scenarios if s.get("ci_critical")]
    return {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "issue": "#372",
        "seed": (suite_meta or {}).get("seed"),
        "scenario_count": len(scenarios),
        "critical_scenario_count": len(critical),
        "all_passed": len(failed_tests) == 0,
        "scenarios": scenarios,
        "critical_scenarios": critical,
        "failed_tests": failed_tests,
    }


def to_markdown(report: dict) -> str:
    lines = [
        "# Adversarial Simulation Suite Report",
        "",
        f"**Generated:** {report['generated_at']}",
        f"**Seed:** `{report.get('seed', 'n/a')}`",
        f"**Scenarios:** {report['scenario_count']} (≥8 required)",
        f"**CI-critical:** {report['critical_scenario_count']} (≥3 required)",
        f"**Status:** {'PASS' if report['all_passed'] else 'FAIL'}",
        "",
        "## Scenario Results",
        "",
        "| Scenario | Defense | Residual Risk | Severity | CI-critical |",
        "|----------|---------|---------------|----------|-------------|",
    ]

    for s in report["scenarios"]:
        lines.append(
            f"| {s['scenario']} | {s['defense']} | {s.get('residual_risk', 'none')} "
            f"| {s.get('severity', 'n/a')} | {'yes' if s.get('ci_critical') else 'no'} |"
        )

    if report["failed_tests"]:
        lines.extend(["", "## Failed Tests", ""])
        lines.extend(f"- `{t}`" for t in report["failed_tests"])

    lines.extend(
        [
            "",
            "## Residual Risks",
            "",
            "- **exposure_cap_boundary**: per-user cap bypassable via sybil addresses (accepted).",
            "- **oracle_heartbeat_griefing**: admin heartbeat override is the recovery path.",
            "",
        ]
    )
    return "\n".join(lines)


def main() -> int:
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <test-output.log> <output-dir>", file=sys.stderr)
        return 2

    log_path = Path(sys.argv[1])
    out_dir = Path(sys.argv[2])
    out_dir.mkdir(parents=True, exist_ok=True)

    scenarios, suite_meta, _passed, failed = parse_log(log_path)
    report = build_report(scenarios, suite_meta, failed)

    json_path = out_dir / "adversarial-report.json"
    md_path = out_dir / "adversarial-report.md"

    json_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    md_path.write_text(to_markdown(report), encoding="utf-8")

    print(f"Wrote {json_path}")
    print(f"Wrote {md_path}")
    print(f"Scenarios: {report['scenario_count']}, critical: {report['critical_scenario_count']}")

    if not report["all_passed"]:
        return 1
    if report["scenario_count"] < 8:
        print("ERROR: fewer than 8 scenarios recorded", file=sys.stderr)
        return 1
    if report["critical_scenario_count"] < 3:
        print("ERROR: fewer than 3 CI-critical scenarios recorded", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
