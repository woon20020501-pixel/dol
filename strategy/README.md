# Dol Strategy Framework

Python reference framework for the Dol cross-venue funding harvester. Reads
Pacifica funding rates, evaluates every `(symbol, counter_venue)` candidate
against an explicit cost model and a set of hard safety gates, and emits
rebalance signal JSON for the bot or operator to consume.

The framework never submits trades — it produces signals; a separate bot or
operator acts on them.

## What this package provides

- **`strategy/cost_model.py`** — explicit fees, slippage, bridge, and
  funding-accrual math.
- **`strategy/aurora_omega/`** — the Aurora-Ω decision layer: hysteresis
  gates, funding-cycle lock, risk stack, fair-value oracle, forecast scoring.
- **`scripts/`** — standalone runners for backtesting, live polling,
  fixture generation, and validation.
- **`rust_fixtures/`** — JSON parity fixtures consumed by `bot-rs/bot-tests`
  so the Rust port matches the Python reference to bit level.
- **`docs/`** — the integration spec, Aurora-Ω master spec, and math
  derivation documents.

## Quickstart

```bash
cd strategy
python -m venv .venv
.venv/Scripts/activate   # Windows
# or: source .venv/bin/activate
pip install -r requirements.txt

# Run the test suite
python -m pytest tests/ -q

# Regenerate Rust parity fixtures
python scripts/generate_rust_fixtures.py

# Inspect the Pacifica aggregated funding endpoint
python scripts/poll_aggregated.py
```

## Directory layout

```
strategy/
├── PRINCIPLES.md              # Iron law and anti-patterns (immutable)
├── README.md
├── requirements.txt
├── strategy/                  # Python package
│   ├── cost_model.py          # Fees, slippage, bridge, funding accrual
│   ├── aurora_omega/          # Decision layer
│   └── ...
├── scripts/                   # Standalone runners
├── tests/                     # pytest suite
├── rust_fixtures/             # JSON parity fixtures for bot-rs
└── docs/                      # Specs and math documentation
    ├── aurora-omega-spec.md
    ├── integration-spec.md
    ├── math-aurora-omega-appendix.md
    └── ...
```

## How signals are consumed

An operator bot or human operator polls `output/signals/` for the latest
signal JSON, validates it against current on-chain state, and executes the
recommended actions. The signal schema and integration contract are
documented in `docs/integration-spec.md`.

## Boundaries

- No on-chain transactions — signals only.
- Read-only access to historical funding data; never writes to upstream
  datasets.
- All output is files in this directory.
