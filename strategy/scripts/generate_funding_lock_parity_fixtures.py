"""
generate_funding_lock_parity_fixtures.py — produce fixtures for Rust port parity test

Runs `strategy/funding_cycle_lock.py` reference functions against ~25 synthetic
inputs and serializes `(input, expected_output)` pairs to
`output/rust_parity/funding_cycle_lock_fixtures.json`.

the parity harness in Rust loads this file and asserts the Rust
implementation produces identical outputs:
  - exact integer equality on integer fields (`cycle_index`, `h_c`)
  - 6 decimal place equality on float fields (`N_c`, `opened_at`, `cycle_phase`,
    `seconds_to_cycle_end`)

This closes v0-punchlist T0-1 "Rust port is parity-tested against Python reference"
and integration-spec §2.1 I-LOCK invariant. Referenced in the bot team the integration spec
and  ack.

Run:
    PYTHONIOENCODING=utf-8 python scripts/generate_funding_lock_parity_fixtures.py

Pure stdlib.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from strategy.funding_cycle_lock import (  # noqa: E402
    CycleState,
    cycle_index,
    cycle_phase,
    enforce,
    is_locked,
    open_cycle,
    seconds_to_cycle_end,
    would_violate_lock,
)


def cs_to_dict(s: CycleState | None) -> dict | None:
    """Convert CycleState to a stable JSON-friendly dict. None passes through."""
    if s is None:
        return None
    return {
        "cycle_index": s.cycle_index,
        "h_c": s.h_c,
        "N_c": s.N_c,
        "opened_at": s.opened_at,
        "cycle_seconds": s.cycle_seconds,
    }


def build_cases() -> list[dict]:
    cases: list[dict] = []

    def case(
        cid: str,
        fn: str,
        inp: dict,
        expected: dict,
        note: str = "",
    ) -> None:
        entry = {
            "id": cid,
            "function": fn,
            "input": inp,
            "expected": expected,
        }
        if note:
            entry["note"] = note
        cases.append(entry)

    # ─────────────────────────────────────────────────────────────────────────
    # cycle_index — integer output, must be exact equality
    # ─────────────────────────────────────────────────────────────────────────
    case(
        "cycle_index_zero", "cycle_index",
        {"t": 0.0, "cycle_seconds": 3600},
        {"value": cycle_index(0.0, 3600)},
        "origin",
    )
    case(
        "cycle_index_just_before_boundary", "cycle_index",
        {"t": 3599.999, "cycle_seconds": 3600},
        {"value": cycle_index(3599.999, 3600)},
    )
    case(
        "cycle_index_at_boundary", "cycle_index",
        {"t": 3600.0, "cycle_seconds": 3600},
        {"value": cycle_index(3600.0, 3600)},
        "exact boundary — must round down to next cycle",
    )
    case(
        "cycle_index_high_timestamp", "cycle_index",
        {"t": 1_776_000_000.0, "cycle_seconds": 3600},
        {"value": cycle_index(1_776_000_000.0, 3600)},
        "realistic 2026 unix epoch",
    )
    case(
        "cycle_index_venue_alt_cadence", "cycle_index",
        {"t": 28_800.0, "cycle_seconds": 28_800},
        {"value": cycle_index(28_800.0, 28_800)},
        "8h venue cadence (e.g. historical HL)",
    )

    # ─────────────────────────────────────────────────────────────────────────
    # cycle_phase — float output, 6 decimals
    # ─────────────────────────────────────────────────────────────────────────
    case(
        "cycle_phase_start", "cycle_phase",
        {"t": 0.0, "cycle_seconds": 3600},
        {"value": cycle_phase(0.0, 3600)},
    )
    case(
        "cycle_phase_mid", "cycle_phase",
        {"t": 1800.0, "cycle_seconds": 3600},
        {"value": cycle_phase(1800.0, 3600)},
    )
    case(
        "cycle_phase_near_end", "cycle_phase",
        {"t": 3500.0, "cycle_seconds": 3600},
        {"value": cycle_phase(3500.0, 3600)},
    )

    # ─────────────────────────────────────────────────────────────────────────
    # seconds_to_cycle_end — float output
    # ─────────────────────────────────────────────────────────────────────────
    case(
        "sec_to_end_start", "seconds_to_cycle_end",
        {"t": 0.0, "cycle_seconds": 3600},
        {"value": seconds_to_cycle_end(0.0, 3600)},
    )
    case(
        "sec_to_end_mid", "seconds_to_cycle_end",
        {"t": 1800.0, "cycle_seconds": 3600},
        {"value": seconds_to_cycle_end(1800.0, 3600)},
    )
    case(
        "sec_to_end_near_end", "seconds_to_cycle_end",
        {"t": 3500.0, "cycle_seconds": 3600},
        {"value": seconds_to_cycle_end(3500.0, 3600)},
    )

    # ─────────────────────────────────────────────────────────────────────────
    # is_locked — bool output
    # ─────────────────────────────────────────────────────────────────────────
    state_long = open_cycle(now=360_000.0, h_c=1, N_c=50_000.0)
    case(
        "is_locked_none_state", "is_locked",
        {"state": None, "now": 1000.0},
        {"value": is_locked(None, 1000.0)},
    )
    case(
        "is_locked_inside_cycle", "is_locked",
        {"state": cs_to_dict(state_long), "now": 360_500.0},
        {"value": is_locked(state_long, 360_500.0)},
    )
    case(
        "is_locked_after_expiry", "is_locked",
        {"state": cs_to_dict(state_long), "now": 365_000.0},
        {"value": is_locked(state_long, 365_000.0)},
        "cycle expired — state is stale, returns False",
    )

    # ─────────────────────────────────────────────────────────────────────────
    # open_cycle — state output
    # ─────────────────────────────────────────────────────────────────────────
    opened_long = open_cycle(now=100_000.0, h_c=1, N_c=25_000.0)
    case(
        "open_cycle_basic_long", "open_cycle",
        {"now": 100_000.0, "h_c": 1, "N_c": 25_000.0, "cycle_seconds": 3600},
        {"state": cs_to_dict(opened_long)},
    )
    opened_short = open_cycle(now=200_000.0, h_c=-1, N_c=75_000.0)
    case(
        "open_cycle_basic_short", "open_cycle",
        {"now": 200_000.0, "h_c": -1, "N_c": 75_000.0, "cycle_seconds": 3600},
        {"state": cs_to_dict(opened_short)},
    )
    opened_flat = open_cycle(now=300_000.0, h_c=0, N_c=0.0)
    case(
        "open_cycle_flat", "open_cycle",
        {"now": 300_000.0, "h_c": 0, "N_c": 0.0, "cycle_seconds": 3600},
        {"state": cs_to_dict(opened_flat)},
        "intentionally flat cycle — lock still applies",
    )
    opened_alt = open_cycle(now=500_000.0, h_c=1, N_c=10_000.0, cycle_seconds=28_800)
    case(
        "open_cycle_alt_cadence", "open_cycle",
        {"now": 500_000.0, "h_c": 1, "N_c": 10_000.0, "cycle_seconds": 28_800},
        {"state": cs_to_dict(opened_alt)},
    )

    # ─────────────────────────────────────────────────────────────────────────
    # enforce — (h_eff, n_eff) output
    # ─────────────────────────────────────────────────────────────────────────
    st1 = open_cycle(now=1_000_000.0, h_c=1, N_c=30_000.0)

    h1, n1 = enforce(st1, now=1_000_500.0, proposed_h=-1, proposed_N=100_000.0)
    case(
        "enforce_locked_rejects_flip", "enforce",
        {
            "state": cs_to_dict(st1),
            "now": 1_000_500.0,
            "proposed_h": -1,
            "proposed_N": 100_000.0,
            "emergency_override": False,
        },
        {"h_eff": h1, "n_eff": n1},
        "locked cycle with h_c=+1 rejects proposed flip to -1",
    )

    h2, n2 = enforce(st1, now=1_001_000.0, proposed_h=1, proposed_N=999_999.0)
    case(
        "enforce_locked_keeps_notional", "enforce",
        {
            "state": cs_to_dict(st1),
            "now": 1_001_000.0,
            "proposed_h": 1,
            "proposed_N": 999_999.0,
            "emergency_override": False,
        },
        {"h_eff": h2, "n_eff": n2},
        "same direction but different notional — still returns locked N_c",
    )

    h3, n3 = enforce(
        st1, now=1_002_000.0, proposed_h=-1, proposed_N=88_000.0, emergency_override=True,
    )
    case(
        "enforce_emergency_override", "enforce",
        {
            "state": cs_to_dict(st1),
            "now": 1_002_000.0,
            "proposed_h": -1,
            "proposed_N": 88_000.0,
            "emergency_override": True,
        },
        {"h_eff": h3, "n_eff": n3},
        "emergency_override=True passes proposed values through",
    )

    h4, n4 = enforce(st1, now=1_010_000.0, proposed_h=-1, proposed_N=60_000.0)
    case(
        "enforce_expired_passes_through", "enforce",
        {
            "state": cs_to_dict(st1),
            "now": 1_010_000.0,
            "proposed_h": -1,
            "proposed_N": 60_000.0,
            "emergency_override": False,
        },
        {"h_eff": h4, "n_eff": n4},
        "cycle expired — enforce passes proposed through without lock",
    )

    h5, n5 = enforce(None, now=100.0, proposed_h=1, proposed_N=10_000.0)
    case(
        "enforce_no_state_passes_through", "enforce",
        {
            "state": None,
            "now": 100.0,
            "proposed_h": 1,
            "proposed_N": 10_000.0,
            "emergency_override": False,
        },
        {"h_eff": h5, "n_eff": n5},
    )

    # ─────────────────────────────────────────────────────────────────────────
    # would_violate_lock — bool output
    # ─────────────────────────────────────────────────────────────────────────
    case(
        "would_violate_flip_inside_lock", "would_violate_lock",
        {"state": cs_to_dict(st1), "now": 1_000_500.0, "proposed_h": -1},
        {"value": would_violate_lock(st1, 1_000_500.0, -1)},
    )
    case(
        "would_violate_same_direction", "would_violate_lock",
        {"state": cs_to_dict(st1), "now": 1_000_500.0, "proposed_h": 1},
        {"value": would_violate_lock(st1, 1_000_500.0, 1)},
    )
    case(
        "would_violate_flat_inside_lock", "would_violate_lock",
        {"state": cs_to_dict(st1), "now": 1_000_500.0, "proposed_h": 0},
        {"value": would_violate_lock(st1, 1_000_500.0, 0)},
        "flat proposal inside long lock is still a violation (h_c != 0)",
    )
    case(
        "would_violate_none_state", "would_violate_lock",
        {"state": None, "now": 1_000_500.0, "proposed_h": -1},
        {"value": would_violate_lock(None, 1_000_500.0, -1)},
    )
    case(
        "would_violate_expired", "would_violate_lock",
        {"state": cs_to_dict(st1), "now": 1_100_000.0, "proposed_h": -1},
        {"value": would_violate_lock(st1, 1_100_000.0, -1)},
        "expired lock — no violation",
    )

    return cases


def main() -> int:
    repo_root = Path(__file__).resolve().parent.parent
    out_dir = repo_root / "output" / "rust_parity"
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / "funding_cycle_lock_fixtures.json"

    cases = build_cases()
    doc = {
        "fixture_version": "1.0",
        "framework_revision": "aurora-omega-1.1.4",
        "generated_from": "strategy/funding_cycle_lock.py",
        "reference_spec_sections": [
            "aurora-omega-spec.md §3.1",
            "integration-spec.md §2.1 (I-LOCK)",
            "v0-punchlist.md T0-1",
        ],
        "parity_requirements": {
            "integer_fields": "exact equality",
            "float_fields": "6 decimal place equality",
            "bool_fields": "exact equality",
            "none_passthrough": "None state must serialize and deserialize as null/None",
        },
        "n_cases": len(cases),
        "cases": cases,
    }

    out_path.write_text(json.dumps(doc, indent=2))
    print(f"Wrote {len(cases)} parity cases to:")
    print(f"  {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
