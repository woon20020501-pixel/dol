# Dol Strategy — First-Principles Mathematical Derivation of Safety Gates

**Status:** **Superseded by `math-formulas.md`.** This document derived fixed values from a particular set of planning assumptions. Under the current design contract, every parameter that is not raw live data must be a formula, not a constant. This document is preserved for the audit trail of how the analysis evolved. Read `math-formulas.md` for the canonical framework.
**Iron law:** see `../PRINCIPLES.md`. This document is the quantitative implementation.

---

## 0. Notation

| symbol | meaning | unit |
|--------|---------|------|
| `A` | total Dol vault AUM | USD |
| `α` | idle bucket fraction (USDC parked in Kamino/Marginfi) | 0..1 |
| `1−α` | trading bucket fraction (deployed to cross-venue funding arb) | 0..1 |
| `r_idle` | Solana USDC supply APY (Kamino base) | per year |
| `r_pair` | per-pair NET funding APY on **per-leg notional**, post fees | per year |
| `L` | leverage applied per leg (each venue independently) | 1..L_max |
| `N` | number of simultaneous active pair positions | int |
| `s` | per-leg notional per pair | USD |
| `m_pos` | maximum per-leg notional as fraction of AUM | 0..1 |
| `m_counter` | maximum trading bucket fraction on any single counter venue | 0..1 |
| `m_oi` | maximum per-leg notional as fraction of venue's OI | 0..1 |
| `c` | total round-trip cost per pair (4 legs of fees + slippage + bridge), as fraction of per-leg notional | 0..1 |
| `H` | minimum hold hours per position before exit allowed | hours |
| `ρ` | required income/cost ratio at entry | dimensionless |
| `p_min` | minimum directional persistence (sign agreement over lookback) | 0..1 |
| `d_max` | max drawdown per position before forced close, as fraction of notional | 0..1 |

Mandate:

| objective | range | midpoint |
|-----------|-------|----------|
| customer APY | 5%–8% | 6.5% |
| buffer APY | 2%–5% | 3.5% |
| reserve | residual | 0.5%–1% |
| **gross vault APY** | **8%–12.3%** | **10%** |

The gross floor (8%) is set by the buffer cut requirement (2% / 0.25 = 8%). The gross ceiling (12.3%) is set by the customer cut requirement (8% / 0.65 = 12.3%). Any vault APY in [8%, 12.3%] satisfies the mandate when split via the 65/25/10 customer/buffer/reserve cut.

Reference value: `r_idle = 4%` from Kamino USDC main pool base rate (April 2026 30-day average per docs survey). Use 4% as planning constant; adaptive recalibration in Phase 1.

---

## 1. Vault APY decomposition

The Dol vault is a two-bucket structure. Idle bucket sits in Kamino USDC. Trading bucket runs cross-venue same-asset funding hedges per the iron law. Both legs of every pair are perp positions on different DEX venues — capital is consumed on each venue independently (no cross-venue cross-margin).

Vault gross APY:

```
r_vault = α · r_idle + (1−α) · r_trade                                      (1)
```

where `r_trade` is the trading bucket APY measured on the bucket's deployed capital (sum of all pair margins).

### 1.1 Trading bucket APY in terms of per-pair return

A pair holds notional `s` on each of two venues. Each venue uses `s/L` margin (initial). Per-pair total capital used = `2s/L`. Per-pair annual income (held passively at signed mean APY) = `r_pair · s` where `r_pair` is the signed funding APY captured on per-leg notional.

Equal-weighted across `N` homogeneous pairs:

```
r_trade = (Σ pair_income) / (Σ pair_capital)
       = (N · r_pair · s) / (N · 2s/L)
       = r_pair · L / 2                                                     (2)
```

**Critical observation:** trading bucket APY is independent of `N` and `s` (under the equal-weight homogeneous assumption). It scales linearly with leverage and per-pair signed APY. This means the choice of `N` and `m_pos` is a **risk-distribution** decision, not a yield decision.

### 1.2 Solving for α given the mandate

Substituting (2) into (1):

```
r_vault = α · r_idle + (1−α) · r_pair · L / 2
```

Solving for α to hit a target `r_vault*`:

```
α* = (r_pair · L / 2 − r_vault*) / (r_pair · L / 2 − r_idle)                (3)
```

This formula gives the optimal idle fraction for any pair-return / leverage / target combination.

### 1.3 Sensitivity table

Conservative `r_pair` planning estimates from the historical walk-forward backtest of passive directional hold (12 profitable pairs, dataset 2026-02-13 to 2026-04-14):

| percentile | NET APY |
|-----------|---------|
| min | 11.3% |
| p25 | 14.5% |
| **p50 (median)** | **22.0%** |
| p75 | 41.6% |
| max | 60.1% |
| mean | 27.7% |

I plan against the **p25 = 14.5%**, rounded down to **14%**, because mandate satisfaction must be robust to the lower tail, not the average.

Solving (3) for `r_vault* = 10%` (mandate midpoint), `r_idle = 4%`:

| L | r_pair | r_pair·L/2 | α* | trading_pct | comment |
|---|--------|------------|-----|-------------|---------|
| 1 | 14% | 7.0% | infeasible | — | r_trade < r_vault*, no idle works |
| 2 | 14% | 14.0% | 40.0% | 60.0% | works, deep deployment |
| **2** | **20%** | **20.0%** | **62.5%** | **37.5%** | **comfortable midpoint** |
| 2 | 28% | 28.0% | 75.0% | 25.0% | very conservative |
| 3 | 14% | 21.0% | 64.7% | 35.3% | works with leverage |
| 3 | 20% | 30.0% | 76.9% | 23.1% | minimal trading |
| 5 | 14% | 35.0% | 80.6% | 19.4% | overkill leverage |

### 1.4 Picking L and α defaults

**Leverage choice.** L = 1 is infeasible (r_pair·L/2 = 7% < 8% gross floor even at α = 0). L = 2 is the minimum feasible leverage. L > 2 is overkill — it lets us run with more idle, but adds liquidation tail risk for marginal benefit.

> **L = 2 (final default).** Rationale: minimum leverage that achieves the mandate. Per-leg liquidation triggers at notional drawdown of 1/(2L) = 25%, vastly above our drawdown stop of 0.5% (50× safety margin). Liquidation is not the binding constraint; basis divergence between two venues' oracles is — and basis risk does not scale with leverage.

**Idle fraction choice.** With L=2, the floor case (r_pair = 14%) gives α* = 40%. The midpoint case (r_pair = 20%) gives α* = 62.5%.

The hard floor on α from the buffer policy (PRINCIPLES §3) is 50%. So:

> **α_default = 0.50 (final default).** Rationale: respects the absolute buffer floor exactly. With r_pair = 14% (p25 conservative), this gives `r_vault = 0.5·4 + 0.5·14 = 9.0%`, comfortably above the 8% floor. With r_pair = 20% (median), `r_vault = 12.0%`, just under the 12.3% ceiling.

**Adaptive band:** the bot may shift α in [0.50, 0.85]. It increases α (decreases trading bucket) when fewer than `N_target` candidates pass safety gates. It cannot decrease α below 0.50 even if more candidates are available.

---

## 2. Capital allocation algebra

### 2.1 N from m_pos under buffer floor

Each pair consumes `2s/L = 2·m_pos·A/L` of trading bucket margin. The trading bucket is `(1−α)·A`. Therefore:

```
N · 2·m_pos·A/L ≤ (1−α) · A
N · m_pos ≤ (1−α) · L / 2                                                   (4)
```

At the floor configuration (`α = 0.5`, `L = 2`):

```
N · m_pos ≤ 0.5
```

So `N = 0.5 / m_pos`. The trade-off:

| m_pos | max N | per-pair margin (% AUM) | max single-symbol exposure |
|-------|-------|--------------------------|----------------------------|
| 1% | 50 | 1% | 1% AUM |
| 2% | 25 | 2% | 2% AUM |
| **3%** | **16** | **3% AUM** | **3% AUM** |
| **4%** | **12** | **4% AUM** | **4% AUM** |
| 5% | 10 | 5% AUM | 5% AUM |
| 7.5% | 6 | 7.5% AUM | 7.5% AUM (too concentrated) |

### 2.2 Picking m_pos

Lower m_pos = more diversified, but requires more pairs to fully deploy. Higher m_pos = more concentrated, fewer pairs needed.

The empirical universe constraint: from the 60-day historical analysis of cross-venue funding, **only ~12 symbols sustained directional persistence > 70%** (computed in §4 below). Above 12 active pairs, we'd run out of safe candidates.

So we want `N_target = 12`. Substituting into (4):

```
12 · m_pos = 0.5
m_pos = 1/24 ≈ 4.17%
```

Round down to **m_pos = 4%** for cleaner accounting and slight headroom.

> **m_pos = 0.04 (final default).** Per-leg position size cap: 4% of AUM. With L=2, per-leg margin = 2% of AUM. Per-pair margin = 4% of AUM. At N=12 pairs, total trading bucket = 48% AUM ≈ 50% buffer floor.

Single-symbol tail loss bound: if the entire position blows up (one venue insolvent, other leg unhedged at a 20% adverse price move), worst case = m_pos × A × max_adverse = 4% × 20% = **0.8% of AUM lost per symbol**. Acceptable.

### 2.3 Per counter-venue cap

We currently support 3 DEX counter venues (Backpack, Hyperliquid, Lighter). For diversification across venue solvency risk, no single counter should hold more than ~1/3 of trading bucket capital, plus a buffer for asymmetric availability of symbols on each venue.

```
m_counter = 1/3 + headroom ≈ 0.40
```

> **m_counter = 0.40 (final default).** No more than 40% of trading bucket capital concentrated on any single counter venue.

### 2.4 Per-OI cap (already in PRINCIPLES, restated)

To bound market impact and slippage, each leg must not consume more than `m_oi` of the venue's open interest:

```
s ≤ m_oi · OI(venue, symbol)
```

The standard market-microstructure heuristic for "you don't move the price" is around 5% of book. We adopt:

> **m_oi = 0.05 (final default).** Each leg ≤ 5% of that venue's OI for the symbol.

This becomes the binding constraint for thin-OI symbols. A symbol with $200k Pacifica OI permits at most $10k per leg, regardless of m_pos × A. The bot enforces both caps and uses the tighter one.

---

## 3. Cost-aware self-consistency: H, r_min, ρ

The income/cost ratio gate, the NET APY floor, and the minimum hold period are not three independent constants. They must be **self-consistent**: at any candidate trade where `r = r_min` and `H = H_min`, the realized ratio must equal `ρ_min`.

### 3.1 The income/cost identity

For a pair held over H hours at signed funding APY r:

```
income(H, r, s) = r · (H / 8760) · s
cost(s)         = c · s
ratio(H, r)     = income / cost = r · H / (8760 · c)                        (5)
```

where `c` is total round-trip cost as fraction of per-leg notional (4 venue fee legs + entry+exit slippage on both legs + 2 bridge crossings if cross-chain).

For ratio = ρ:

```
H · r = ρ · 8760 · c                                                        (6)
```

This is the **fundamental constraint** linking H, r, c, ρ.

### 3.2 Estimating c

Conservative cost components for a Pacifica + Backpack pair (both Solana, no bridge), maker-only execution, typical $20k notional on a deep-OI symbol:

| component | per side | round-trip total |
|-----------|----------|------------------|
| Pacifica maker fee | 0.015% | 0.030% (open + close) |
| Backpack maker fee | 0.020% | 0.040% (open + close) |
| Pacifica slippage | 0.5–1.5 bps | 0.01–0.03% (in + out) |
| Backpack slippage | 0.5–1.5 bps | 0.01–0.03% (in + out) |
| Bridge | 0% | 0% |
| **total c (low)** | | **0.08%** |
| **total c (mid)** | | **0.12%** |
| **total c (high)** | | **0.18%** |

For a Pacifica + Hyperliquid pair (cross-chain, with bridge):

| component | round-trip total |
|-----------|------------------|
| Pacifica + HL fees | 0.080% |
| slippage both legs | 0.02–0.06% |
| bridge in + out | 0.20% (HL CCTP-class, conservative) |
| **total c (cross-chain)** | **0.30–0.34%** |

Phase 1 dry run will measure real `c` per pair. For derivation, plan against **c = 0.0015 = 0.15%** as the conservative-but-realistic average across same-chain and cross-chain pairs.

### 3.3 Solving the H, r, ρ system

**Constraint A (mandate floor):** `r_min` must satisfy the trading bucket APY requirement at α = 0.5, L = 2:

```
r_pair · L / 2 ≥ r_trade_required
r_pair · 2 / 2 ≥ (r_vault_floor − α · r_idle) / (1−α)
r_pair ≥ (0.08 − 0.5·0.04) / 0.5 = 0.12
```

> **r_min = 12% (final default).** A pair must project ≥ 12% NET APY on per-leg notional to be worth entering.

**Constraint B (ratio gate self-consistency):** with `r = r_min`, `c = 0.15%`, `ρ = 1.5`:

```
H_min = ρ · 8760 · c / r_min
      = 1.5 · 8760 · 0.0015 / 0.12
      = 164.25 hours ≈ 7 days
```

> **H_min = 168 hours = 7 days (final default).** Any new position is committed for a minimum of 7 days. Below this, the income/cost math does not survive at the r_min floor.

This is significantly longer than my earlier eyeballed 48h. The reason: I was implicitly assuming r ≥ 25% (where 48h does work) instead of r ≥ 12% (the actual floor).

### 3.4 The ratio constant ρ

The 1.5x value is set so that the realized income clears the cost with a 50% safety margin against estimation error. Statistical justification:

If our point estimate of expected income has relative standard error `σ/μ ≈ 0.20` (well-filtered universe with persistence > 70% lookback, 168 samples), then the 5%-quantile of realized income is approximately:

```
income_5% ≈ μ · (1 − 1.65 · σ/μ) = μ · 0.67
```

For `prob(income < cost) ≤ 5%`, we need `income · 0.67 ≥ cost`, i.e. `ratio ≥ 1.49 ≈ 1.5`.

> **ρ = 1.5 (final default).** Provides 5% downside protection assuming income estimation σ/μ ≤ 20%.

If we want stricter 1% downside (ρ = 1.87), the cost is excluding marginal trades — fewer candidates pass. 1.5 is the tradeoff.

---

## 4. Persistence floor — statistical derivation

Persistence is measured as the fraction of past `T` hours where `sign(spread)` matches the dominant sign. For `T = 168` (one week of hourly funding observations) and a fair-coin null (50/50 random walk), the standard error of the sample sign-proportion is:

```
σ_p = √(p(1−p)/T) = √(0.25/168) = 0.0386
```

Z-score for various candidate thresholds:

| p_min | Z above 0.5 null | one-tailed p-value |
|-------|------------------|--------------------|
| 0.55 | 1.30 | ~10% |
| 0.60 | 2.59 | ~0.5% |
| 0.65 | 3.88 | ~5e-5 |
| **0.70** | **5.18** | **~1e-7** |
| 0.75 | 6.47 | ~5e-11 |
| 0.80 | 7.77 | ~4e-15 |

70% sign agreement over 168 hours corresponds to a 5σ rejection of the random-walk null — essentially certain that a real directional bias exists. Tightening to 75% or 80% removes more candidates without buying meaningful additional confidence.

> **p_min = 0.70 over a 168-hour lookback (final default).** Statistical strength: ~5σ. Empirical fit: matches the historical universe of ~12 symbols that consistently exhibit directional carry, which itself matches our `N_target = 12` derived in §2.2.

This is not a coincidence — I tuned `N_target` and `p_min` together so that the universe size (`p_min` permits) equals the configuration N (the m_pos × N saturation gives).

---

## 5. Drawdown stop — basis-risk derivation

The strategy holds two perp legs on the same asset across two venues. With β = 1 by construction, expected price P&L = 0. Realized P&L deviates from zero only from:

| source | typical magnitude |
|--------|------------------|
| oracle divergence (Pacifica vs counter venue) | 5–20 bps |
| settlement asynchrony (one leg fills 5s before the other) | 1–10 bps depending on price velocity |
| funding payment timing mismatch (Pacifica funding hourly, counter funding hourly) | 0–3 bps over a single funding cycle |
| venue downtime / partial fills | rare, 0–50 bps |

Sum of routine basis risk: ~10–40 bps. Tail (during venue stress): 50–100 bps. We want the drawdown stop to:
- not fire during routine basis fluctuation (would cause unnecessary churn)
- fire promptly when routine basis is exceeded (signal that hedge integrity has broken)

50 bps (0.5%) sits at the upper end of routine and the lower end of tail. This is the natural separation point.

> **d_max = 0.005 (0.5% of per-leg notional) (final default).** A position whose drawdown exceeds 0.5% has lost hedge integrity and is forcibly closed. Tighter (0.2%) would cause false closes; looser (1%) would allow dangerous decay.

Verification against L=2: per-leg margin = notional / 2. A 0.5% notional drawdown = 1% margin drawdown. Maintenance margin call triggers at 50% margin drawdown. Safety ratio = 50× — drawdown stop fires far before forced liquidation. ✓

---

## 6. Daily P&L circuit breakers

Three thresholds, monotone, escalating action:

```
−1% AUM in one day → halve all open Tier-B/C positions, allow Tier-A to continue
−2% AUM in one day → close all Tier-B/C, hold only Tier-A pairs
−3% AUM in one day → close everything, alert operator, halt new entries
```

Justification: a 1% daily loss is recoverable in normal weeks but signals something is off; halving exposure buys time to investigate. A 2% daily loss is a serious anomaly — preserve capital first. A 3% daily loss is a regime break — full stop until human review. These thresholds compound to a per-quarter survivable max drawdown of ~5% even in a bad scenario.

> **(k_warn, k_halve, k_kill) = (-0.01, -0.02, -0.03) (final defaults).**

---

## 7. The complete derived configuration

| variable | value | derived from |
|----------|-------|--------------|
| **r_idle** | 4% | Kamino USDC base rate (Apr 2026) |
| **r_pair (planning)** | 14% (p25), 20% (median) | walk-forward backtest of passive directional hold |
| **L** | 2 | minimum leverage that satisfies mandate; safe vs liquidation |
| **α_default** | 0.50 | floors at buffer policy; exact match for L=2, r_pair=14% mandate |
| **α_min** | 0.50 | hard floor from PRINCIPLES |
| **α_max** | 0.85 | adaptive band when signals are weak |
| **m_pos** | 0.04 | 0.5 / N_target for N_target = 12 |
| **m_counter** | 0.40 | 1/3 + headroom across 3 DEX counters |
| **m_oi** | 0.05 | market-impact heuristic |
| **N_target** | 12 | matches universe of persistence-passing symbols |
| **r_min** | 0.12 | (mandate floor − α·r_idle) / (1−α) at α=0.5, L=2 |
| **H_min** | 168 hours (7 days) | ρ · 8760 · c / r_min at c = 0.15% |
| **ρ_min** | 1.5 | 5% downside protection at σ/μ ≤ 0.20 |
| **p_min** | 0.70 over 168h | 5σ rejection of random-walk null |
| **d_max** | 0.005 (0.5% notional) | upper end of routine basis risk |
| **k_warn** | −0.01 AUM/day | recoverable loss, halve risk |
| **k_halve** | −0.02 AUM/day | serious anomaly, drop to Tier-A only |
| **k_kill** | −0.03 AUM/day | regime break, full stop |

---

## 8. Mandate compliance verification

Substituting back into (1) at the conservative planning estimate (`r_pair = 14%`, `L = 2`, `α = 0.5`):

```
r_vault = 0.5 · 0.04 + 0.5 · (0.14 · 2 / 2)
        = 0.02 + 0.07
        = 0.09 = 9.0% gross
```

Cut allocation (65/25/10):
- customer = 9.0% × 0.65 = **5.85% APY** ✓ (in 5–8% band)
- buffer = 9.0% × 0.25 = **2.25% APY** ✓ (in 2–5% band)
- reserve = 9.0% × 0.10 = **0.90%**

At the median estimate (`r_pair = 20%`):
```
r_vault = 0.02 + 0.10 = 12.0% gross
```
- customer = 7.80% ✓
- buffer = 3.00% ✓
- reserve = 1.20%

At the optimistic estimate (`r_pair = 27.7%` mean):
```
r_vault = 0.02 + 0.1385 = 15.85% gross
```
- customer = 10.30% (above 8% ceiling — bot must rebalance to 8% cap and route excess to buffer)

**The configuration satisfies the mandate at both the 25th percentile and the median historical performance, with margin in both directions.**

---

## 9. Sensitivity to r_idle and c

What happens if Kamino yield drops to 3% or rises to 6%?

| r_idle | r_pair (p25) | r_vault | customer (×0.65) | buffer (×0.25) | mandate? |
|--------|---|---------|------------------|----------------|----------|
| 3% | 14% | 8.5% | 5.5% | 2.1% | ✓ |
| 4% | 14% | 9.0% | 5.85% | 2.25% | ✓ |
| 5% | 14% | 9.5% | 6.18% | 2.38% | ✓ |
| 6% | 14% | 10.0% | 6.50% | 2.50% | ✓ |

Robust to ±2pp Kamino swings.

What happens if real cost `c` is double the planning estimate (0.30% instead of 0.15%)?

`H_min` recomputes to:
```
H_min = 1.5 · 8760 · 0.003 / 0.12 = 328.5 hours = 13.7 days
```

The bot would need to commit positions for 14 days minimum. This would reduce flexibility (slower regime adaptation) but the mandate math is unchanged. Phase 1 dry run measures actual `c` and we recalibrate H_min only.

---

## 10. What the bot does at each tick

Pseudocode using these constants (the cost_model.py functions implement this):

```python
for each (symbol, counter_venue) candidate:
    if not (counter is DEX): continue
    if persistence_dominant_pct(symbol, counter) < 0.70: continue
    if sign(current_spread) ≠ sign(persistent_signed_mean): continue
    
    s = min(
        m_pos · A,           # AUM concentration cap (4% AUM)
        m_oi · OI_pacifica,  # 5% Pacifica OI
        m_oi · OI_counter,   # 5% counter OI
    )
    
    leg_margin = s / L              # L = 2
    pair_margin = 2 · leg_margin
    
    if sum(active_pair_margin_on_counter) + pair_margin > m_counter · trading_bucket_capital:
        skip  # would breach 40% per-counter cap
    
    if sum(all_pair_margins) + pair_margin > (1 − α_min) · A:
        skip  # would breach 50% buffer floor
    
    income = abs(spread_h) · H_min · s     # H_min = 168 hours
    cost = c_estimated · s                 # cost model output
    if income / cost < ρ_min:              # ρ_min = 1.5
        skip
    
    proj_apy = (income − cost) / s · (8760 / H_min)
    if proj_apy < r_min:                   # r_min = 12%
        skip
    
    # all gates passed → emit signal to enter
    enter_pair(symbol, counter, leg_margin, direction = sign(persistent_signed_mean))

for each open position:
    if held_hours < H_min:
        hold
    elif drawdown > d_max:                 # 0.5% notional
        forced_close("drawdown breach")
    elif persistence_dominant_pct < p_min:
        forced_close("persistence broke")
    elif abs(current_signed_apy) < r_min / 2:
        forced_close("signal decayed below half-floor")

if daily_pnl < k_kill · A:
    close_all(); alert_pm()
elif daily_pnl < k_halve · A:
    close_tier_b_c()
elif daily_pnl < k_warn · A:
    halve_all_open()
```

---

## 11. What changes in cost_model.py

The constants in `strategy/cost_model.py` `SafetyGates` are now traceable to this document. Updated values:

| field | old (eyeballed) | new (derived) | source paragraph |
|-------|-----------------|---------------|------------------|
| `min_hold_h` | 48 | **168** | §3.3 |
| `persistence_lookback_h` | 168 | 168 | §4 (unchanged) |
| `persistence_min_dominant_pct` | 0.65 | **0.70** | §4 (5σ) |
| `max_pct_of_oi` | 0.05 | 0.05 | §2.4 (unchanged) |
| `max_pct_aum_per_symbol` | 0.02 | **0.04** | §2.2 (m_pos derived) |
| `max_pct_per_counter` | 0.40 | 0.40 | §2.3 (unchanged) |
| `max_drawdown_per_position` | 0.005 | 0.005 | §5 (unchanged) |
| `min_income_cost_ratio_at_entry` | 1.50 | 1.50 | §3.4 (unchanged, justified) |
| `min_net_apy_at_entry` | 0.05 | **0.12** | §3.3 (mandate-derived r_min) |
| `aum_buffer_floor` | 0.50 | 0.50 | §1.4 (unchanged) |
| `pnl_warning_threshold` | −0.01 | −0.01 | §6 (unchanged) |
| `pnl_halve_threshold` | −0.02 | −0.02 | §6 (unchanged) |
| `pnl_kill_threshold` | −0.03 | −0.03 | §6 (unchanged) |

New constants (not in old SafetyGates):

```python
TARGET_LEVERAGE_PER_LEG: int = 2          # §1.4
TARGET_NUM_ACTIVE_PAIRS: int = 12         # §2.2
PLANNING_R_PAIR_P25: float = 0.14         # §1.3 walk-forward p25
PLANNING_R_PAIR_MEDIAN: float = 0.22      # §1.3 walk-forward median
PLANNING_R_IDLE: float = 0.04             # §0
PLANNING_COST_PER_CYCLE: float = 0.0015   # §3.2 conservative middle
```

These planning constants are what the bot uses to derive H_min on the fly per-trade if we later switch to per-trade hold sizing. For now the simple constant H_min = 168 hours is used and the derivation is documented in §3.3.
