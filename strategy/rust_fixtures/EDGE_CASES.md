# Edge Case Coverage Checklist for Rust Parity Tests

**Purpose:** Ensure `rust_fixtures/*.json` covers every important boundary and
singular case of each math function. The bugs that actually bite a Rust
implementation are at **boundaries**, on **degenerate input**, at
**sign boundaries**, and under **numerical overflow**.

**Status legend:**
- covered — covered in current fixtures
- partial — partially covered (some conditions only)
- missing — missing
- n/a — not applicable (intentionally excluded)

---

## §1. phi(x) — Absorption function

| condition | expected | Status |
|---|---|---|
| `x = 0` exact | result = 1.0 (Taylor limit) | covered |
| `x = 1e-15` (Taylor regime) | uses Taylor truncation, second-order accuracy | covered |
| `x = 1e-12` (boundary) | Taylor vs exp branch-point test | covered |
| `x ∈ [1e-8, 1e-3]` (small positive) | Taylor and exp agree | covered |
| `x ∈ [0.1, 10]` (operating range) | exp path, 6-decimal parity | covered |
| `x ∈ [20, 500]` (tail) | φ → 0, no underflow | covered |
| `x` negative (e.g., -1) | computes normally (the formula is well-defined) | partial |
| `x = NaN` | NaN propagation verified | missing |
| `x = inf` | result → 0 | missing |
| Monotonicity property test | φ(0.1) > φ(0.5) > ... > φ(10) | covered |
| sign of `phi_derivative(x)` | φ′(x) < 0 everywhere | covered |
| `phi_derivative(0) = -1/2` | Taylor limit | covered |

**Rust implementation note:** the Taylor/exp branch point `|x| < 1e-12` must
match Python exactly. Near the boundary, `(1-e^(-x))/x` suffers catastrophic
cancellation. Taylor is taken to `1 - x/2 + x^2/6`.

---

## §2. ou_time_averaged_spread

| condition | expected | Status |
|---|---|---|
| `tau → 0` | result → D_0 (φ(0)=1) | covered |
| `tau → ∞` | result → μ (φ(∞)=0) | covered |
| `D_0 = μ` (any tau) | result = μ invariant | covered |
| `D_0 > μ` (positive edge) | result in (μ, D_0), monotone decreasing in tau | covered |
| `D_0 < μ` (negative edge) | result in (D_0, μ) | covered |
| Negative `D_0`, negative `μ` | short direction, sign handled | partial |
| `theta_OU = 0` | result = D_0 invariant (no reversion) | missing |
| `theta_OU < 0` (unstable OU) | mathematically undefined; error path? | n/a |
| Very large `theta_OU · tau` (> 1000) | φ underflow, result → μ | partial |

**Rust implementation note:** this is a simple linear combination with no
singularities; only extreme tau where `phi(theta·tau) = 0` needs care.

---

## §3. effective_spread_with_impact

| condition | expected | Status |
|---|---|---|
| `n = 0` | pure OU average (impact 0) | covered |
| `n` typical | cost gate applied | covered |
| `n` large | 1 - theta·n/Pi ≤ 0 → early return 0 | partial |
| `Pi_pac ≤ 0` | error (invalid input) | missing (needs error-path test) |

**Rust implementation note:** when `1 - theta·n/Pi ≤ 0`, **early return 0**.
A negative result is strictly forbidden.

---

## §4. break_even_hold_*

| condition | expected | Status |
|---|---|---|
| `mu ≤ 0` | return inf or error | partial |
| large `rho` | tau^BE grows linearly | parity check covered |
| fixed-point iteration converges | typical convergence case | covered |

**Rust implementation note:** fixed-point iteration does not guarantee
convergence. Pin the policy to `max_iter = 100`, `tol = 1e-6`.

---

## §5. optimal_*

| condition | expected | Status |
|---|---|---|
| L dependence (Regime II) | T* independent of L (Theorem J.2) | partial |

**Rust implementation note:** the Regime I/II transition is handled by the
caller via `min(w_i*, m_pos)`; the math function only returns the interior
optimum.

---

## §6. critical_aum / Bernstein

| condition | expected | Status |
|---|---|---|
| `epsilon = 0` or `epsilon = 1` | invalid, error | missing |

**Rust implementation note:** prevent `epsilon = 0` inside `log(1/epsilon)`.
Returns an integer floor.

---

## §7. bernstein_leverage_bound

**Rust implementation note:** prevents negative `sqrt` via an early return
when `ratio < 0`.

---

## §11. cap_routing

**Rust implementation note:** the conservation invariant is a
`debug_assert!`. A `1e-12` tolerance is allowed due to float rounding.

---

## §12. mandate_floor

| condition | expected | Status |
|---|---|---|
| Symmetric bind (cust_min adjusted) | both equal | covered |

---

## §15. fit_ou

**Rust implementation note:** Python `fit_ou` uses the Phillips 1972 SE
formula (`stochastic.py` lines 93-99). Different from the AR(1) intercept SE.

---

## §16. adf_test

**Rust implementation note:** uses MacKinnon 1996 critical values with
`adf_test(with_constant=True)`.

---

## §19. fit_drift

| condition | expected | Status |
|---|---|---|
| `hold_h → ∞` | drift term dominates | missing |

---

## §20. End-to-end `compute_rigorous_state`

| condition | expected | Status |
|---|---|---|
| Historical 60-day data + default Mandate | 6 decimal parity | structural stub only |
| Empty LiveInputs | returns empty state | missing |
| Only Class M pairs (Backpack) | rejected, zero candidates | missing |
| Mandate infeasible | notes includes error | missing |

**Note for the Rust implementation:** the full E2E fixture is generated by
a separate script `generate_e2e_fixture.py`. It is a large fixture, managed
independently.

---

## Priority to fill missing

Among the currently-uncovered items, in priority order:

### Priority 1 (directly affects correctness)
1. **§1 phi** inf/NaN propagation — confirm Rust's IEEE 754 handling matches Python.
2. **§2 ou** `theta_OU = 0` → D_0 invariant check.
3. **§4 break_even** `c = 0` degenerate case.
4. **§7 bernstein** `MMR = 0`, `sigma = 0`, `Delta = 0` tested independently.
5. **§11 cap_routing** negative `R` guard.

### Priority 2 (numerical robustness)
6. **§5 optimal_*** `A → 0`, `A → ∞` extremes.
7. **§8 mfg** `Pi → 0`, `theta_impact → 0`.
8. **§14 round_trip** `legging_window = 0`.

### Priority 3 (edge semantics)
9. **§15 fit_ou** sample < 30 → None.
10. **§16 adf** sample < 30 → None.
11. **§20 E2E** full dry_run fixture generation script.

---

## Fixture request protocol

If the Rust implementation hits an uncovered edge case:

1. **Do not append new cases directly to `rust_fixtures/<section>_extras.json`.**
   Those files must be regenerable from the Python reference.
2. Instead, add the new case to `scripts/generate_rust_fixtures.py`, regenerate,
   and commit the updated fixture alongside the Rust test.

**No ad-hoc hard-coded values on the Rust side.** Every parity test must be
based on a Python-generated fixture.

---

## Fixture regeneration policy

Whenever code under `strategy/` changes, run:

```bash
cd strategy
.venv/Scripts/python.exe scripts/generate_rust_fixtures.py
```

and commit the updated `rust_fixtures/*.json` files. These files are the
single source of truth.

---

## Target fixture count

Current: **131 cases**

With priority 1-3 filled: expected **~180 cases**.

With the full E2E fixture: expected **~200 cases**.

Each fixture corresponds to one Rust `#[test]` (or a `#[test_case]` group).

---

## Rust-side loading example

```rust
let fixtures = bot_tests::load_fixtures::<PhiInput, PhiExpected>("phi");
for case in fixtures {
    let got = phi(case.input.x);
    bot_tests::assert_close(got, case.expected.value, case.tolerance, &case.name);
}
```

Inf/NaN string handling is done via `bot_tests::parse_float_or_special`.
