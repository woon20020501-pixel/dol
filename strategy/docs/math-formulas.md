# Dol Strategy — Live-Adaptive Formula Framework

**Status:** Canonical. Replaces `math-derivation.md` (which fixed values from a single backtest snapshot). **Design contract:** every parameter except raw live data must be a closed-form formula evaluated per tick, not a backtested constant.
**Iron law:** `../PRINCIPLES.md`. This document is the live-adaptive implementation of that law.

---

## 0. The split between live data and formulas

There are exactly two kinds of numbers in this system.

### Live data (read each tick from the world)

These are the only "real" numbers. Everything else is computed from them.

| symbol | source | refresh | meaning |
|--------|--------|---------|---------|
| `A` | vault contract balance | each tick | total Dol AUM in USD |
| `r_idle` | Kamino USDC supply rate API | hourly | current Kamino USDC supply APY |
| `f(s,v)` | venue funding endpoint | each funding tick | per-hour funding rate, signed, for symbol `s` on venue `v` |
| `OI(s,v)` | venue API | each tick | open interest USD for `s` on `v` |
| `Vol(s,v)` | venue API | each tick | rolling 24h volume USD for `s` on `v` |
| `fee_m(v)`, `fee_t(v)` | venue API | daily | maker / taker fee per leg, fraction of notional |
| `bridge(v_a, v_b)` | bridge oracle | hourly | round-trip bridge cost between two venues, fraction of notional |
| `f_hist(s,v,t)` | rolling DB | continuous | history of `f(s,v)` for persistence stats |
| `δ_hist(s,t)` | rolling per-position | each tick | history of basis divergence for symbol `s` (oracle gap between venues holding our pair) |
| `R_vault(d)` | rolling | daily | daily vault return, last `D` days |

### Mandate constants (the only true fixed values)

These come from policy decisions and statistical conventions. They do not change.

| symbol | value | source |
|--------|-------|--------|
| `cust_min` | 0.05 | mandate floor |
| `cust_max` | 0.08 | mandate ceiling |
| `buf_min` | 0.02 | mandate floor |
| `buf_max` | 0.05 | mandate ceiling |
| `cut_cust` | 0.65 | gross→customer split |
| `cut_buf` | 0.25 | gross→buffer split |
| `cut_res` | 0.10 | gross→reserve split |
| `α_floor` | 0.50 | hard buffer policy from PRINCIPLES |
| `α_cap` | 0.95 | don't be 100% idle, defeats purpose |
| `Z_pers` | 5.0 | persistence confidence (5σ vs random walk) |
| `Z_draw` | 3.0 | drawdown confidence (3σ basis envelope) |
| `Z_pnl_warn`, `Z_pnl_halve`, `Z_pnl_kill` | 1, 2, 3 | daily P&L circuit breaker ladder |
| `Z_ratio` | 1.65 | 5% one-tailed downside protection on income/cost ratio |
| `ε_op` | 0.01 | operational risk premium above r_idle |
| `T_pers_max` | 720h | max persistence lookback (avoid stale regimes) |
| `T_pers_min` | 168h | min persistence lookback (sample-size floor) |
| `T_basis` | 168h | basis-divergence lookback for d_max |
| `D_pnl` | 30 | days of vault returns for circuit breakers |
| `k_safety_L` | 10 | leverage safety multiplier (drawdown stop fires 10× before liquidation) |

Everything else is a formula evaluated each tick from these inputs.

---

## 1. Target gross vault APY — `r_target(t)`

### Derivation

The mandate gives a band, not a point. The target should sit at the geometric center of the achievable band:

```
r_target_floor = max(cust_min / cut_cust, buf_min / cut_buf)
r_target_ceil  = min(cust_max / cut_cust, buf_max / cut_buf)
r_target_mid   = (r_target_floor + r_target_ceil) / 2
```

With current cut values:
- `r_target_floor = max(0.05/0.65, 0.02/0.25) = max(0.0769, 0.08) = 0.08`
- `r_target_ceil  = min(0.08/0.65, 0.05/0.25) = min(0.1231, 0.20) = 0.1231`
- `r_target_mid   = 0.1015`

**Live formula:**

```
r_target(t) = (max(cust_min/cut_cust, buf_min/cut_buf) + min(cust_max/cut_cust, buf_max/cut_buf)) / 2
```

If the cut percentages change later, `r_target` recomputes automatically.

---

## 2. Per-trade cost — `c(s, v_p, v_c, n)`

The total round-trip cost of opening and closing a same-asset pair (one leg on Pacifica, one on counter venue `v_c`), at notional `n` per leg, as a fraction of single-leg notional.

```
c(s, v_p, v_c, n) = 
    fee_m(v_p) · 2                        # Pacifica fees, open + close
  + fee_m(v_c) · 2                        # counter fees, open + close
  + slip(s, v_p, n) · 2                   # Pacifica slippage, in + out
  + slip(s, v_c, n) · 2                   # counter slippage, in + out
  + bridge(v_p, v_c) · 2                  # bridge cost, in + out
```

Slippage estimator (square-root market-impact model with depth proxy):

```
depth_proxy(s, v) = max(0.10 · OI(s,v), 0.01 · Vol_24h(s,v), $1k)
slip(s, v, n)     = clamp(0.0008 · √(n / depth_proxy(s, v)), 0.0001, 0.02)
```

The constants 0.10 and 0.01 reflect that ~10% of OI and ~1% of 24h volume are accessible without major price impact. The 0.0008 multiplier and the [1bp, 200bp] clamp are calibrated from typical perp DEX microstructure literature (Almgren-Chriss style square-root impact). These three numbers are **measurable in Phase 1 dry run** by placing test orders and observing realized slippage; they are the only "tuning constants" in the cost model and will be replaced by live-fitted values once available.

Bridge cost:
- Same chain (e.g., Pacifica + Backpack both Solana): `bridge = 0`
- Cross-chain (e.g., Pacifica Solana + Hyperliquid L1 via CCTP): `bridge ≈ 0.0010` round trip, **read from a live bridge price oracle in production**, not hardcoded

---

## 3. Symbol persistence — `p̂(s, v_c, T)` and `μ̂(s, v_c, T)`

For each candidate (symbol, counter venue), measure two stats from the rolling lookback window:

```
T(s, v_c) = clamp(available_history_hours(s, v_c), T_pers_min, T_pers_max)
spread_h(s, v_c, t) = f(s, v_c, t) - f(s, v_p, t)     # signed, per hour
```

**Sample signed mean APY (the trade's expected funding income rate):**

```
μ̂(s, v_c) = mean over T past hours of spread_h(s, v_c, ·)  · 8760
```

**Sample sign-dominance (persistence):**

```
p̂(s, v_c) = (count of past hours where sign(spread_h) == sign(μ̂)) / T
```

**Sample standard deviation (for ratio gate):**

```
σ̂(s, v_c) = stdev over T past hours of spread_h · 8760
```

These are pure rolling statistics. Nothing fixed.

---

## 4. Persistence threshold — `p_min(T)`

Under the null hypothesis "spread sign is fair-coin random walk", the sample sign-proportion has standard error:

```
σ_p(T) = √(0.25 / T)
```

The threshold for `Z_pers`-σ rejection of the random-walk null:

```
p_min(T) = 0.5 + Z_pers · σ_p(T) = 0.5 + Z_pers · √(0.25 / T)
```

Sensitivity:

| T (hours) | days | σ_p | p_min (Z=5) |
|-----------|------|-----|-------------|
| 168 | 7 | 0.0386 | 0.6930 |
| 336 | 14 | 0.0273 | 0.6364 |
| 504 | 21 | 0.0223 | 0.6113 |
| 720 | 30 | 0.0186 | 0.5932 |

Longer lookbacks give lower thresholds (more samples → tighter null). The bot uses the actual T it has, not a fixed assumption.

A candidate (s, v_c) passes the persistence gate iff `p̂(s, v_c) ≥ p_min(T(s, v_c))`.

---

## 5. Income / cost ratio — `ρ(s, v_c)`

The ratio gate exists to ensure the realized income beats cost with downside protection. Required ratio depends on the candidate's signal-to-noise ratio:

```
SNR(s, v_c) = |μ̂(s, v_c)| / σ̂(s, v_c)
ρ(s, v_c)   = 1 / (1 - Z_ratio · σ̂(s, v_c) / |μ̂(s, v_c)|)
            = 1 / (1 - Z_ratio / SNR(s, v_c))
```

Floor: if `SNR < Z_ratio + 0.05` (the discount becomes ≤ 5%), the candidate is rejected (return ρ = ∞).

Sensitivity (Z_ratio = 1.65):

| SNR | σ/μ | ρ required |
|-----|-----|-----------|
| 10 | 0.10 | 1.197 |
| 5 | 0.20 | 1.493 |
| 3.33 | 0.30 | 1.981 |
| 2.5 | 0.40 | 2.928 |
| 2 | 0.50 | 5.747 |
| 1.65 | 0.606 | ∞ (reject) |

So a candidate with high signal-to-noise needs only a modest ratio (1.2x); a noisy candidate needs a large one (3-5x); a candidate with `SNR ≤ Z_ratio` is rejected outright.

---

## 6. Drawdown stop — `d_max(s)`

Compute per-symbol basis volatility from the rolling oracle divergence history:

```
σ_basis(s) = stdev over T_basis hours of δ_hist(s, ·)
d_max(s)   = Z_draw · σ_basis(s)
```

Where `δ(s, t) = (oracle_p(s, t) - oracle_c(s, t)) / oracle_p(s, t)` is the proportional gap between Pacifica's oracle and the counter venue's oracle for the symbol at time `t`.

For `Z_draw = 3` and typical `σ_basis = 0.0015`:

```
d_max ≈ 3 · 0.0015 = 0.0045 ≈ 0.5% notional
```

This recovers the planning value when basis is "typical". When basis volatility spikes (e.g., during venue stress), `d_max` automatically widens, preventing premature exit during temporary basis blowouts. Conversely, when basis is unusually quiet, `d_max` tightens, catching subtle hedge breakage earlier.

Bootstrap fallback: if there is less than 24h of basis history for the symbol, use `d_max = 0.005` until enough data accumulates.

---

## 7. Daily P&L circuit breakers — `(k_warn, k_halve, k_kill)`

Compute from rolling vault daily returns:

```
σ_vault = stdev over D_pnl days of R_vault(·)
k_warn   = -Z_pnl_warn  · σ_vault   [= -1σ]
k_halve  = -Z_pnl_halve · σ_vault   [= -2σ]
k_kill   = -Z_pnl_kill  · σ_vault   [= -3σ]
```

Bootstrap fallback (until D_pnl days accumulate): use planning values `(-0.01, -0.02, -0.03)` of AUM.

When `σ_vault` is small (calm market), the breakers tighten — small losses fire early warnings. When `σ_vault` widens (volatile market), the breakers widen — false positives are avoided. The system self-calibrates.

---

## 8. Required leverage — `L*(t)`

Leverage is chosen as the smallest integer that lets the system hit `r_target(t)` at the buffer floor `α_floor`, given the currently-observed median candidate APY `μ̂_med(t)`:

```
μ̂_med(t) = median over passing candidates c of μ̂(c)

L*(t) = ⌈ 2 · (r_target(t) - α_floor · r_idle(t)) / ((1 - α_floor) · μ̂_med(t)) ⌉
```

Subject to caps:
- `L*(t) ≤ L_venue_max(s)` per symbol (Pacifica says max 50x for crypto, 10x for RWAs)
- `L*(t) ≤ L_safe = 1 / (k_safety_L · d̄_max(t))` where `d̄_max(t)` is the median per-symbol drawdown stop
- `L*(t) ≥ 1` (no shorting leverage)

Sensitivity at `r_idle = 0.04`, `α = 0.50`, `r_target = 0.1015`:

| `μ̂_med` | `L*` | comment |
|---------|----|---------|
| 0.06 | 5 | barely-viable signals, max leverage |
| 0.10 | 3 | weak signals, need leverage |
| 0.14 | 3 | walk-forward p25, mid-range |
| 0.20 | 2 | walk-forward median, comfortable |
| 0.30 | 2 | strong signals |
| 0.50 | 1 | very strong signals, no leverage needed |

The bot picks `L` once per cycle from the live `μ̂_med(t)`. As market conditions improve (more high-quality candidates), leverage drops. As they deteriorate, leverage rises (within the safety cap).

---

## 9. Idle bucket fraction — `α(t)`

Given the chosen `L*(t)` and the trading bucket APY it produces:

```
X(t) = μ̂_med(t) · L*(t) / 2   (expected trading bucket APY)
```

Solve equation (1) for the idle fraction that hits `r_target(t)`:

```
r_target = α · r_idle + (1 - α) · X
α* = (X - r_target) / (X - r_idle)
```

Apply hard floor and cap:

```
α(t) = clamp(α*, α_floor, α_cap)
```

If `α* < α_floor`, the buffer policy binds and we stay at 0.50; vault APY ends up below target, but safety wins.

If `α* > α_cap`, signals are so good that target is over-hit. We go max idle (95%); excess return flows to buffer/reserve.

---

## 10. Number of active pairs — `N_active(t)`

Determined by candidate availability, not assumed:

```
candidates(t) = {(s, v_c) : 
                  v_c ∈ DEX_counters
                  AND p̂(s, v_c) ≥ p_min(T(s, v_c))
                  AND SNR(s, v_c) > Z_ratio + 0.05
                  AND μ̂(s, v_c) ≠ 0
                  AND OI(s, v_p) > 0 AND OI(s, v_c) > 0}

N_active(t) = |candidates(t)|
```

This is just the live-counted number of (symbol, counter) pairs that passed the cheap pre-filter. Could be 0 (degenerate market — bot stays all-idle), could be 30+ (rich market).

---

## 11. Per-symbol AUM cap — `m_pos(t)`

Derived from capital allocation algebra: trading bucket capital = `N · 2s/L`, so:

```
m_pos(t) = (1 - α(t)) · L*(t) / (2 · N_target(t))
```

Where `N_target(t)` is bounded above by candidate availability:

```
N_target(t) = min(N_active(t), N_max_simultaneous)
N_max_simultaneous = 30   (operational ceiling — too many positions = manageable basis tracking)
```

Sensitivity at `α = 0.5`, `L = 2`:

| `N_target` | `m_pos` |
|-----------|---------|
| 6 | 8.33% |
| 10 | 5.00% |
| 12 | 4.17% |
| 16 | 3.13% |
| 20 | 2.50% |
| 30 | 1.67% |

When few good candidates exist, each one gets a larger position (less diversification). When many good candidates exist, each gets smaller (more diversification). Both extremes are bounded — by `α_floor` from below and by `N_max_simultaneous` from above.

Floor: if `N_target = 0`, `m_pos = 0` (no trades).

---

## 12. Per counter-venue cap — `m_counter(t)`

Counter venues active at time t are those with at least one passing candidate:

```
counters_active(t) = unique counter venues across candidates(t)
N_counter_active(t) = |counters_active(t)|
m_counter(t) = clamp(1.20 / N_counter_active(t), 0.0, 1.0)
```

The `1.20` provides 20% headroom — when 3 counters are available, each can hold up to 40% (slightly more than the 33% pure split, allowing concentration into the best one). When only 1 counter is available, `m_counter = 1.0` (no constraint, the venue diversification gate is naturally absent).

---

## 13. Per-leg OI cap — `m_oi(s, v)`

Adapts to per-symbol-per-venue turnover:

```
turnover(s, v) = Vol_24h(s, v) / max(OI(s, v), $1k)
m_oi(s, v)     = 0.05 · clamp(√turnover(s, v), 0.5, 1.4)
```

| turnover | m_oi |
|----------|------|
| 0.25 (illiquid) | 0.025 (2.5%) |
| 0.50 | 0.0354 |
| 1.0 (typical perp) | 0.05 |
| 2.0 | 0.0707 |
| 3.0+ (high churn) | 0.07 (capped) |

Illiquid symbols get tighter OI caps (preserves slippage profile). High-turnover symbols get slightly wider caps (capacity to enter/exit cleanly).

---

## 14. Per-trade min hold — `H_min(s, v_c, n)`

Compute from the candidate's actual cost and expected APY at this notional:

```
c_pair(s, v_c, n) = cost from §2 above with notional n
ρ_pair(s, v_c)    = ratio from §5 above
μ̂_signed(s, v_c)  = signed APY from §3 above

H_min(s, v_c, n) = ρ_pair(s, v_c) · 8760 · c_pair(s, v_c, n) / |μ̂_signed(s, v_c)|
```

If `H_min > T_pers_max`, the trade is rejected (we can't justify holding longer than our persistence horizon).

This is the crucial change vs the old fixed `H_min = 168h`: each candidate has its OWN min hold based on its OWN cost and signal strength. A high-APY low-cost candidate might need only 24h; a marginal candidate might need 200h. The bot accepts only those it can actually commit to.

---

## 15. NET APY floor — `r_min(t)`

```
r_min(t) = max(
    r_idle(t) + ε_op,                                              # operational risk premium
    2 · (r_target_floor - α(t) · r_idle(t)) / ((1 - α(t)) · L*(t)) # mandate floor
)
```

A trade is rejected if its projected NET APY is below this floor. Both floors adapt to live `r_idle`, current `α`, and current `L*`.

---

## 16. Putting it together — the per-tick algorithm

At each funding tick (every 1 hour):

1. **Refresh live inputs** — pull `A`, `r_idle`, `f(s,v)`, `OI(s,v)`, `Vol(s,v)`, fees, bridge costs, funding history, basis history, vault returns.
2. **Compute target** — `r_target(t)` from §1.
3. **Build candidate list** — for every `(s, v_c)` with `v_c ∈ DEX`:
   - compute `μ̂`, `σ̂`, `p̂` from rolling history (§3)
   - compute `T(s, v_c) = clamp(available_history_hours, T_pers_min, T_pers_max)`
   - compute `p_min(T)` (§4) and reject if `p̂ < p_min`
   - compute `SNR` and reject if `SNR ≤ Z_ratio + 0.05`
   - reject if instantaneous `sign(f(s,v_c) - f(s,v_p))` differs from `sign(μ̂)`
4. **Estimate `μ̂_med(t)`** — median of passing candidates' signed APY.
5. **Compute leverage** — `L*(t)` from §8 using `μ̂_med` and venue caps.
6. **Compute idle fraction** — `α(t)` from §9 using `X = μ̂_med · L*/2`.
7. **Compute position cap** — `m_pos(t)` from §11.
8. **Compute counter cap** — `m_counter(t)` from §12.
9. **Compute drawdown stops** — per-symbol `d_max(s)` from §6 using basis history.
10. **Compute circuit breakers** — `(k_warn, k_halve, k_kill)` from §7 using vault returns.
11. **Compute NET APY floor** — `r_min(t)` from §15.
12. **For each candidate** in priority order (sorted by `μ̂ × p̂ × OI_min`):
    - choose notional `n = min(m_pos · A, m_oi(s,v_p) · OI(s,v_p), m_oi(s,v_c) · OI(s,v_c))`
    - compute `c(s, v_p, v_c, n)` (§2)
    - compute `ρ(s, v_c)` (§5) and `H_min(s, v_c, n)` (§14)
    - compute projected NET APY = `(|μ̂| · H_min/8760 - c) / 1 · 8760/H_min · 1` (per-leg notional)
    - check: `income/cost ≥ ρ` AND `NET APY ≥ r_min(t)` AND would-not-breach `m_counter`
    - if all gates pass, mark as enterable
13. **Allocate** — fill the trading bucket with enterable candidates, top down by NET APY, until `(1-α(t))·A` capital is consumed or candidate list exhausted.
14. **Re-evaluate existing positions** — for each open position:
    - if `t_held < H_min(at_entry)`: hold
    - elif `current_drawdown ≥ d_max(s)`: forced close (basis blew out)
    - elif `p̂_current < p_min(T)`: forced close (regime broke)
    - elif `|μ̂_current| < r_min(t) / 2`: forced close (signal decayed)
    - else: hold
15. **Daily check** — if `daily_pnl_pct < k_kill(t)`: close all + alert operator. `< k_halve(t)`: close Tier-B/C. `< k_warn(t)`: halve all open.
16. **Emit signal JSON** — target portfolio with reasons.

Every value used in steps 5–15 is computed from the live inputs in step 1. There are no backtest-derived constants in the decision path.

---

## 17. Self-consistency under typical live inputs

Plug in plausible 2026-04-14 values to check the framework produces sensible numbers:

```
A = $1,000,000
r_idle = 0.044  (Kamino mid-April 2026)
μ̂_med = 0.20    (walk-forward median observed)
N_active = 12
N_counter_active = 3
typical T(s, v_c) = 720h (30d max lookback)
typical SNR = 5  (σ/μ ≈ 0.20)
typical σ_basis = 0.0015
σ_vault not yet measured (use bootstrap)
```

Running the chain:

| variable | formula | computed value |
|---|---|---|
| `r_target` | §1 | 0.1015 |
| `p_min(720h)` | §4 | 0.5932 |
| `ρ(SNR=5)` | §5 | 1.493 |
| `d_max(σ_basis=0.0015)` | §6 | 0.0045 |
| circuit breakers | §7 | (-0.01, -0.02, -0.03) bootstrap |
| `L*` | §8 | `⌈2·(0.1015 - 0.5·0.044) / (0.5·0.20)⌉ = ⌈1.59⌉ = 2` |
| `α` | §9 | `(0.20 - 0.1015) / (0.20 - 0.044) = 0.631` (above floor, accept) |
| `m_pos` | §11 | `(1 - 0.631) · 2 / (2 · 12) = 0.0307` (3.07% AUM per leg) |
| `m_counter` | §12 | `1.20 / 3 = 0.40` |
| `r_min` | §15 | `max(0.044+0.01, 2·(0.08 - 0.631·0.044)/((1-0.631)·2)) = max(0.054, 0.143) = 0.143` |

Compare to v1 fixed values (which I derived from a single backtest):

| variable | v1 fixed | v2 live (typical conditions) | match? |
|---|---|---|---|
| `α` | 0.50 | 0.63 | v2 chose more idle because median signal is good |
| `L` | 2 | 2 | ✓ |
| `m_pos` | 4.0% | 3.07% | v2 chose smaller per-symbol because more capital is idle |
| `m_counter` | 40% | 40% | ✓ |
| `p_min` | 0.70 (assumed T=168h) | 0.593 (T=720h available) | v2 uses larger sample → tighter null |
| `ρ` | 1.5 | 1.49 | ✓ (essentially identical) |
| `r_min` | 12% | 14.3% | v2 more conservative because α is higher |
| `d_max` | 0.005 | 0.0045 | ✓ (essentially identical) |
| circuit breakers | (-1%, -2%, -3%) | (-1%, -2%, -3%) bootstrap | ✓ |

The v2 live framework recovers v1 values (within ±20%) under typical inputs. The difference: v2 ADAPTS. If `r_idle` drops to 2%, if median signal weakens to 10%, if a counter venue goes offline, if basis volatility spikes — every value updates without intervention.

This is the live-adaptive framework. The math doesn't change; only the inputs do.

---

## 18. What the cost_model.py refactor does

The functions in `strategy/cost_model.py` are reorganized into:

```python
@dataclass
class LiveInputs:
    aum_usd: float
    r_idle: float
    funding_rate_h: dict[(str, str), float]
    open_interest_usd: dict[(str, str), float]
    volume_24h_usd: dict[(str, str), float]
    fee_maker: dict[str, float]
    fee_taker: dict[str, float]
    bridge_fee_round_trip: dict[(str, str), float]
    funding_history_h: dict[(str, str), list[(int, float)]]
    basis_divergence_history: dict[str, list[(int, float)]]
    vault_daily_returns: list[float]

@dataclass(frozen=True)
class Mandate:
    customer_apy_min, customer_apy_max
    buffer_apy_min, buffer_apy_max
    cut_customer, cut_buffer, cut_reserve
    aum_buffer_floor, aum_idle_cap
    Z_persistence, Z_drawdown
    Z_pnl_warn, Z_pnl_halve, Z_pnl_kill
    Z_ratio_downside
    operational_risk_premium
    persistence_lookback_hours_min, persistence_lookback_hours_max
    basis_lookback_hours
    pnl_lookback_days
    leverage_safety_multiplier
    max_simultaneous_pairs

# Pure formula functions
def target_vault_apy(m: Mandate) -> float
def slippage(notional, oi, vol_24h) -> float
def round_trip_cost(symbol, v_p, v_c, notional, inputs) -> float
def signed_mean_apy(symbol, counter, inputs) -> float
def sample_std_apy(symbol, counter, inputs) -> float
def persistence_pct(symbol, counter, inputs) -> float
def lookback_hours(symbol, counter, inputs, m) -> int
def persistence_threshold(T_h: int, m: Mandate) -> float
def required_ratio(snr: float, m: Mandate) -> float
def drawdown_stop(symbol, inputs, m) -> float
def pnl_breakers(inputs, m) -> tuple
def required_leverage(median_pair_apy, inputs, m) -> int
def idle_fraction(median_pair_apy, leverage, inputs, m) -> float
def position_aum_cap(idle_frac, leverage, n_active, m) -> float
def counter_venue_cap(n_counters_active, m) -> float
def oi_cap(symbol, venue, inputs) -> float
def per_trade_min_hold(symbol, counter, notional, ratio, inputs) -> float
def net_apy_floor(idle_frac, leverage, inputs, m) -> float

# Composition
def compute_system_state(inputs, m) -> SystemState
def evaluate_trade_live(candidate, system_state, inputs, m) -> TradeDecision
```

All formulas live in pure functions with no internal state. They take `LiveInputs` and `Mandate` and return a number. The bot calls `compute_system_state(inputs, mandate)` once per tick to get the current operating gates, then `evaluate_trade_live(...)` for each candidate.
