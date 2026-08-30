#!/usr/bin/env python3
"""Record WASM sizes and fail when a contract exceeds hard limit or baseline tolerance."""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BASELINE_PATH = ROOT / "baselines" / "wasm_sizes.json"


def main() -> int:
    baseline = json.loads(BASELINE_PATH.read_text(encoding="utf-8"))
    tolerance = int(baseline.get("tolerance_bytes", 2048))
    report_path = os.environ.get("WASM_SIZE_REPORT", "")
    lines: list[str] = []
    failed = False

    for name, spec in baseline["contracts"].items():
        wasm_path = ROOT / spec["path"]
        max_bytes = int(spec["max_bytes"])
        recorded = int(spec.get("baseline_bytes", 0))

        if not wasm_path.exists():
            print(f"ERROR: missing {wasm_path}")
            failed = True
            continue

        size = wasm_path.stat().st_size
        delta = size - recorded if recorded > 0 else 0
        line = f"{name}: {size} bytes"
        if recorded > 0:
            line += f" (baseline {recorded}, delta {delta:+d})"
            if abs(delta) > tolerance:
                print(
                    f"ERROR: {name} size delta {delta:+d} exceeds tolerance {tolerance} bytes"
                )
                failed = True
        if size > max_bytes:
            print(f"ERROR: {name} exceeds hard limit ({size} > {max_bytes})")
            failed = True
        print(line)
        lines.append(line)

    if report_path:
        Path(report_path).write_text("\n".join(lines) + "\n", encoding="utf-8")

    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
