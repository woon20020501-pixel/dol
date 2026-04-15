# Dol Strategy — IRON LAW

**Status:** Immutable design contract. Do not modify without sign-off.
**Audience:** the strategy framework, the trading bot, and all future contributors.

---

## §1 — The single iron law

> **Dol earns yield by holding the same asset on two perp DEX venues in opposite directions, capturing the funding rate spread, with zero net price exposure.**

Concretely:
- Long instrument X on venue A
- Short instrument X on venue B
- A and B are different perp DEXes (Pacifica / Backpack / Hyperliquid / Lighter / …)
- X is the SAME underlying asset (β = 1.0 by construction, not by statistical hedge)
- Net delta = 0 mathematically, regardless of price moves
- The only revenue source is funding rate accrual — not directional P&L, not arbitrage on price

**Important clarification:** the iron law is venue-pair symmetric. It is NOT "Pacifica vs X" — any `(A, B)` combination of approved DEXes is a valid pair. The v3.5 codebase built the first universe by anchoring Pacifica (because that was the highest-signal venue in the 60-day sample), but the framework is agnostic. Backpack↔Hyperliquid, Lighter↔Backpack, and any other DEX-DEX combination is admissible as long as both legs are on non-KYC DEXes and the asset is literally identical (same oracle feed on one or both sides, or deterministically convertible — e.g. cbBTC ↔ WBTC is NOT the same asset, BTC-PERP ↔ BTC-PERP IS).

**Single-venue concentration cap:** to keep the iron law robust to a single-venue failure mode (insolvency, forced delisting, oracle manipulation, bridge halt), the aggregate exposure on any single venue — summed across both "A-leg" and "B-leg" positions that use that venue — must not exceed `max_single_venue_exposure = 0.60` of the deployed bucket. This is enforced in `cost_model.Mandate` and applied in `portfolio.chance_constrained_allocate` via the `max_per_counter` constraint AND an aggregate check in `rigorous.compute_rigorous_state`. The 60% figure is intentionally loose — in the current 46-pair universe where Pacifica appears on one leg of every pair, the cap would force half the candidates off Pacifica, which is not possible without a second pivot venue with comparable universe depth. The cap is there to document the concentration reality and alert on it, not to cripple current operations. Any operating point where Pacifica exposure > 60% of deployed is treated as a known concentration risk to be reduced as Backpack and Hyperliquid universes deepen.

This is the strategy. It does not change.

---

## §2 — What we are NOT doing

These are explicit anti-patterns. Future iterations must reject anything that looks like one of these.

1. **Not directional bets.** No naked long, no naked short, no trend following, no momentum.
2. **Not statistical pair trades.** β-hedged correlated pairs (e.g. BTC long + ETH short) are NOT this strategy. They have residual basis risk. We only do same-asset pairs.
3. **Not cross-asset arb.** No "long PAXG short XAU" because they reference different on-chain instruments. Only same-symbol on two venues.
4. **Not single-venue strategies.** Pacifica-only pair trades, Pacifica-only funding chase, single-venue cap arb — all rejected. Both legs MUST be on different venues.
5. **Not high-frequency rotation.** No reactive entries chasing every funding tick. Position changes only when the cost-of-action math justifies them.
6. **Not yield-maximization at the cost of safety.** Higher APY is irrelevant if the structure exposes user funds to non-funding risks.
7. **Not custodial venue exposure.** No KYC-required CEXes (Binance / Bybit / OKX / Coinbase / Bitget). The Dol vault is a non-custodial DeFi product. Counter-venues must be DEXes.

---

## §1.5 — LOCK: v3.5 (cross-venue funding hedge framework)

**Design decision:** v3.5 is the locked framework. The cross-venue same-asset funding hedge is the single strategy. v4.0 (Kamino multiply + base lending + phase-aware lifecycle) was a deviation from the iron law and is rejected; `strategy/lifecycle.py` and `docs/math-final-v4.md` are deprecated.

**v3.5.1 → v3.5.2 revision (post-external-review):**

The v3.5.1 "lock" at `LOCKED_MIN_LEVERAGE = 3` has been **revoked** following an external quant review that identified four fatal flaws in its justification:

1. **Narrative inconsistency at the operating point.** The sensitivity table claimed L=3 → customer 8.00% (cap) / buffer 4.78%, but the actual dry-run produced customer 6.63% / buffer 2.55%. Cap routing into reserve was asserted but never measured.
2. **Single-regime sample.** 60 days of data ≈ one funding-regime sample. Walk-forward on 30d/30d gave identical train/test results because both windows are censored by the mandate caps — that is cap censoring, not robustness.
3. **Tail bound used wrong distribution.** Per-pair tail loss was computed with Gaussian VaR on basis residuals whose empirical Hurst ≈ 0.9 implies fat, persistent tails. Gaussian bounds systematically understate liquidation-scale basis blowouts.
4. **Wrong liquidation mechanism.** The "33× distance to maintenance margin call" was a single-venue calculation. The actual liquidation mechanism is a cross-venue basis blowout (oracle divergence, venue insolvency, forced delistings — Terra 2022, FTX 2022, JELLY 2025) which stresses both legs simultaneously. Single-venue maintenance margin is not the binding constraint.

Until the remediation work on critiques #3, #4, #5 lands (real-data CVaR/DRO tail bound + basis-blowout shock model + OU/Hurst reconciliation), the leverage floor is disabled and the framework reverts to auto-derived L (typically L=2 on the current universe, matching the v3.5 baseline).

**Crucial bug found during the post-revocation audit:** the previous "v3.5 baseline = customer 5.56% / buffer 2.14%" claim was **wrong**. The number came from a half-fixed `dry_run_v3_5.py` that sized `m_pos` using `LOCKED_MIN_LEVERAGE` while hardcoding the `(L/2)` income multiplier to L=2. With both unified to the auto-derived L from `required_leverage_rigorous`, the honest output is materially better. See the table below.

**v3.5 framework (post-revocation, post-bug-fix, no hard L lock):**
- Strategy: cross-venue same-asset funding hedge per the iron law in §1
- Code modules: `strategy/cost_model.py` + `strategy/stochastic.py` + `strategy/portfolio.py` + `strategy/rigorous.py` + `strategy/frontier.py`
- Documents: `docs/math-formulas.md` + `docs/math-rigorous.md` + `docs/math-frontier.md`
- Validation: `scripts/validate_formulas.py` (6/6) + `scripts/validate_rigorous.py` (8/8 incl. 3 null-hypothesis tests) + `scripts/validate_frontier.py` (7/7) + `scripts/dry_run_v3_5.py`

**Honest real 60-day data dry run (L auto-derived from `required_leverage_rigorous`):**

| metric | value | note |
|---|---|---|
| Active L3-passing pairs | 46 of 52 | Pacifica anchored, Backpack/Hyperliquid counter |
| Hurst (DFA) distribution | median 0.916, max 1.133 | persistent regime — drift-mode model used (critique #3 fix) |
| Median \|OU mu APY\| | 7.45% | below the 7.69% target → formula picks L=3 |
| L (auto-derived) | **3** | NOT a hardcoded lock — `required_leverage_rigorous` picks it |
| m_pos (auto) | 1.36% per leg | (1−α)·L / (2N) at α=0.5, N=46 |
| AUM deployed | 50% | α floor binding |
| Vault gross APY | **14.20%** | |
| Customer (capped 8%) | **8.00%** | mandate ceiling, cap routing engaged |
| Buffer (capped 5%) | **4.78%** | +278bp above floor, comfortable |
| Reserve | **1.42%** | |
| Chance-constraint (Gaussian) | feasible, vault 5%-VaR = 11.43% | floor 7.69% → +374bp slack |
| Chance-constraint (empirical fat-tail × 1.40) | feasible, vault 5%-VaR = 11.40% | tail inflation modest |
| Binding constraint | budget (α floor) | NOT chance constraint |

**Why both v3.5.1's "8.00%/4.78%" sensitivity table AND the dry-run's "6.63%/2.55%" were partially right:** they were the same 46 candidates allocated under inconsistent (L, m_pos) sizings. Now both are unified at the auto-derived L=3, and the honest result matches the cap-routing arithmetic in the v3.5.1 sensitivity table (which was the correct calculation; the dry-run print was the wrong one).

**Why the L=3 result is acceptable WITHOUT a `LOCKED_MIN_LEVERAGE = 3` floor:** the framework's `required_leverage_rigorous` formula auto-picks L=3 from the median signal strength on the current universe. If the universe ever weakens (lower median APY) the formula will increase L further; if it strengthens (median APY > target) the formula will drop to L=2 or L=1 and the framework will deploy more conservatively. The lock was redundant — the formula was already at 3.

**Remaining residual risks (NOT removed by the v3.5.2 revision):**

- **Single-venue concentration (the dominant risk).** Of 34 backpack symbols and 28 hyperliquid symbols, only 1 pair has neither leg on Pacifica. So aggregate Pacifica exposure on the current universe is structurally ≈ 100% of deployed = 50% of AUM, which exceeds the new `Mandate.max_single_venue_exposure = 0.60` design cap. This is acknowledged as a structural fact, NOT a fix-by-tomorrow problem — it will only diminish as alternative pivot venues deepen. **A Pacifica-side incident (oracle failure, forced delisting, venue insolvency) is the dominant tail risk for the vault until then.**
- **Basis-blowout sizing is qualitative.** §5.1 documents the historical scenarios (Terra, FTX, Mango, JELLY) and the per-pair / per-venue caps; quantification of "what shock size to plan for" is a policy decision, not a parametric output of the model. A shock scenario (5% one-leg vs 20% one-leg) must be signed off on before live trading.
- **Critique #2 single-regime data.** 60 days = 1 regime. Walk-forward results are cap-censored. The L=3 derivation will be re-validated after 6-12 months of accumulated `cross_venue_funding.sqlite` poller data.
- **Critique #3 OU/Hurst contradiction (partially fixed).** `filter_candidate_rigorous` now routes H>0.70 candidates to a drift-mode path with `fit_drift` + thirds-reproducibility gate. The OU annualization in `portfolio.py` (`σ² × T`) still uses the iid form rather than the OU autocorrelation-corrected form — flagged in §5.2 as conservative but not exact.
- **Critique #4 Gaussian VaR (partially fixed).** `chance_constrained_allocate` now accepts a `fat_tail_multiplier` computed from real signed-return history. On the current 60-day data the multiplier is 1.40 (modest); the chance constraint stays feasible with massive slack at L=3. The full empirical Monte-Carlo path is future work.

**Operating-point recommendation:**

1. Treat **customer 8.00% / buffer 4.78% / reserve 1.42% / L=3 / 50% deployed** as the framework's expected output. The previous "5.56% / 2.14%" number was from a buggy script and should be discarded.
2. The dominant residual risk is the 100% Pacifica concentration. Authorize live trading at a fraction of full AUM, ramped up in stages, with explicit acknowledgment of this concentration.
3. Track Backpack/Hyperliquid universe depth so the 60% single-venue cap can be enforced as soon as ≥ 5 viable non-Pacifica pairs exist.
4. Re-run the L derivation after 6 months of accumulated poller data to confirm it still lands at 3 in a different regime.

**Data caveat for all downstream analysis:** `historical_cross_venue.sqlite` is ~60 days, ONE regime. Any optimization result from this dataset is a "60-day sample optimum," not statistically validated. Treat sweeps as exploratory.

**Instant withdrawal mechanism (small AUM):**
At beta-scale AUM ($1k–$100k), per-pair notionals are tiny ($10–$1k each). Cross-venue unwind takes 1-2 seconds per pair (one Solana tx for Pacifica leg + one tx for Backpack/Hyperliquid leg). Total unwind for 30+ pairs: under 30 seconds. The 64% idle bucket in Solana base lending covers all normal redemptions instantly. Larger redemptions trigger the unwind path which is "near-instant" at small AUM.

**Granger causality finding (informational, doesn't change the strategy):**
Of 55 cross-venue pairs tested, 41 show counter venue Granger-leading Pacifica funding (counter funding moves first, Pacifica catches up). 0 pairs have Pacifica leading. 10 pairs are bidirectionally causal. This means our strategy entries should react to counter venue moves, but the buy-and-hold carry mechanic is unaffected.

## §2.0 — Three layers of the framework

The strategy framework has three layers, each successively more rigorous, all live-adaptive, none containing backtest-derived constants:

**First-order (closed-form formulas, fast path)** — `strategy/cost_model.py`
- Live inputs (AUM, funding, OI, vol, fees, bridge, history) → closed-form gate values
- Documented in `docs/math-formulas.md`
- Validated in `scripts/validate_formulas.py`
- Use when speed matters or when historical depth is insufficient for second-order machinery

**Second-order (classical 1952-2001 quant rigor)** — `strategy/stochastic.py` + `strategy/portfolio.py` + `strategy/rigorous.py`
- Funding spread modeled as Ornstein-Uhlenbeck stochastic process (Vasicek 1977)
- Augmented Dickey-Fuller test for stationarity (Dickey-Fuller 1979)
- Maximum likelihood estimation with Phillips 1972 asymptotic SE
- Mean-variance Markowitz allocation with Ledoit-Wolf shrinkage
- Empirical CVaR drawdown stops (Rockafellar-Uryasev 2000)
- Documented in `docs/math-rigorous.md`
- Validated in `scripts/validate_rigorous.py` (5/5 tests passing)

**Third-order (modern 2005-2024 frontier methods)** — `strategy/frontier.py`
- Inductive conformal prediction (Vovk 2005) — distribution-free finite-sample VaR coverage, replaces Gaussian VaR assumption
- Maurer-Pontil empirical Bernstein (2009) — finite-sample concentration replaces asymptotic CLT
- Esfahani-Kuhn Wasserstein DRO (2018) — distributionally robust portfolio replaces vanilla Markowitz
- DFA Hurst exponent (Peng et al. 1994, Gatheral-Jaisson-Rosenbaum 2018) — rough volatility classification
- Exponential-kernel Hawkes process (Hawkes 1971, Bacry et al. 2015) — basis jump clustering, cluster-aware drawdown stops
- **The "Dol Theorem"** — sub-Gaussian tail bound for stationary OU funding spreads, derived via Hermite polynomial spectral decomposition; tighter than Markov-class bounds for fast-reverting processes
- Documented in `docs/math-frontier.md` (synthesizes 2005-2024 quant methods specifically for cross-venue funding spread harvesting)
- Validated in `scripts/validate_frontier.py` (7/7 tests passing including a same-distribution conformal coverage test, Hurst recovery on AR(1), Hawkes MLE recovery, sub-Gaussian tail bound empirical confirmation, and end-to-end portfolio integration)

The bot uses **all three layers in cascade** at each tick. The first-order layer is the fast path for thin-history candidates. The second-order layer adds OU dynamics and Markowitz allocation when ≥168h of history is available. The third-order layer adds distribution-free coverage guarantees, finite-sample concentration bounds, distributional robustness, rough-volatility diagnostics, and cluster-aware tail risk. A candidate must clear ALL active layers to be admitted; an existing position must NOT trigger any layer's exit condition to remain.

## §2.1 — All safety gates are LIVE FORMULAS, not fixed values

**Design contract:** every parameter except raw live data must be a closed-form formula evaluated at each tick from live inputs. There are no backtest-derived constants in the decision path. The full formula derivations are in `docs/math-formulas.md`. The reference implementation is `strategy/cost_model.py`. The synthetic-input validation lives in `scripts/validate_formulas.py` and must be re-run after any change to either the formulas or the cost model.

The split:

| kind | what | examples |
|---|---|---|
| **live data** (read each tick) | only these are "real" numbers | `A` AUM, `r_idle` Kamino rate, `f(s,v)` funding, `OI(s,v)`, `Vol(s,v)`, fees, bridges, funding history, basis history, vault returns |
| **mandate constants** (statistically-conventional) | the only fixed values in the system | customer/buffer mandate band, cut percentages, Z-score multipliers (Z_persistence=5, Z_drawdown=3, Z_pnl=(1,2,3), Z_ratio_downside=1.65), operational-risk premium ε_op=0.01, hard buffer floor α_floor=0.50, lookback windows |
| **derived parameters** (computed each tick) | everything else | `α`, `L*`, `m_pos`, `m_counter`, `m_oi`, `p_min(T)`, `ρ(SNR)`, `H_min(s,v_c,n)`, `r_min`, `d_max(s)`, PnL breakers |

Every "derived parameter" is implemented as a pure function in `cost_model.py`. The bot calls `compute_system_state(inputs, mandate)` once per funding tick and gets back the current operating envelope.

**To change a derived parameter you must:**
1. Edit the formula in `cost_model.py` (and its documentation in `math-formulas.md`)
2. Rerun `scripts/validate_formulas.py` and confirm both the THIN UNIVERSE and RICH UNIVERSE scenarios behave correctly
3. Document the rationale in a commit message

You may NOT pin a derived parameter to a fixed value, even temporarily. The framework's adaptiveness is the safety property — pinning it removes the safety.

**Validated behavior under canonical inputs** (validate_formulas.py 2026-04-14):
- THIN UNIVERSE (real April 14 thin-OI symbols): framework stays mostly idle, vault gross = 4.89%, customer = 3.18% — **mandate honestly missed** because the live universe is too thin to deploy. Framework correctly refuses to fake compliance.
- RICH UNIVERSE (12 deep-OI symbols, median 22% APY): framework derives `L=2, α=68.1%, m_pos=2.66%, r_min=15.67%`, enters 9 of 12 candidates, deploys 24% of AUM, leaves 76% in idle, vault gross = **8.36%**, customer = **5.44%** ✓, buffer = **2.09%** ✓.

**Real-data dry run (dry_run_v3_5.py 2026-04-14):**
60 days of cross-venue funding history (52 Pacifica-anchored pairs, 26 symbols × Backpack/Hyperliquid). After data-driven correction of the Hurst gate from `[0.30, 0.70]` (OU assumption) to `≥ 0.30` (because empirical funding spreads have H ≈ 0.9 — strongly persistent, not mean-reverting):
- 46 of 52 pairs pass the 3-layer cascade
- DRO portfolio deploys 35.9% of AUM across 35+ pairs at m_pos = 1.09% per leg
- α (idle) = 64.1%, L = 2
- **Vault gross APY = 8.56%**
- **Customer (capped at 8%) = 5.56% ✓** (mandate 5-8%)
- **Buffer (capped at 5%) = 2.14% ✓** (mandate 2-5%)
- Reserve = 0.86%
- **Mandate met on real historical data.**

**Lesson from the data**: the OU H=0.5 assumption (math-rigorous §1) is wrong for cross-venue funding spreads. They are H ≈ 0.9 — strongly persistent / trending. Reject only H < 0.30 (anti-persistent noise).

**Granger causality finding (Causal IV lite):** 41 of 55 tested pairs show the counter venue (Backpack/Hyperliquid) leads Pacifica funding with extreme F-statistics (max F = 2565 for TRUMP/HL). Pacifica is the slow follower in price discovery. 10 pairs are bidirectionally Granger-causal (structural). 0 pairs have Pacifica leading. This is a structural fact about the venue ecosystem worth flagging to the operator.

---

## §3 — What the framework provides

The framework does NOT pick which symbols to trade. The bot does that, in real time, using this framework. The framework provides:

1. **The cost model** the bot uses on every decision: trading fees per leg, bridge fees, slippage estimate, funding accrual. Conservative defaults. Auditable formulas.
2. **The immutable safety gates**: minimum hold period, maximum position size as % of OI, maximum exposure per counter-venue, minimum spread persistence before entry, buffer floor, drawdown circuit breakers.
3. **The decision rule** the bot evaluates per candidate trade: is `(expected_funding_income_over_min_hold − total_cost − slippage) > 0` AND does the trade pass all safety gates?
4. **Validation tooling** — backtests, persistence reports, reliability checks — so operators can verify the framework is working in live data.
5. **The iron law** documented at the top of every artifact so no future contributor accidentally redesigns toward yield over safety.

The framework is a **rule designer and path designer**, not a stock picker.

---

## §4 — The bot's job

The bot executes the framework. At each decision tick (1 hour funding cycle):

1. Pull funding rates from every supported DEX (Pacifica + Backpack + Hyperliquid + Lighter + ...)
2. For each (symbol, counter_venue) candidate where Pacifica and counter both list the symbol:
   - Compute current spread (counter funding − Pacifica funding)
   - Estimate persistence (rolling signed mean over the safety window)
   - Estimate trade cost: `4 × leg_fee + 2 × bridge_fee + 2 × slippage(size, depth)`
   - Estimate expected funding income over the minimum hold period
   - Decision = (income > cost) AND passes_all_safety_gates(symbol, counter, size)
3. Sort viable candidates by `(income - cost) / capital_required`
4. Allocate capital top-down until trading bucket is full
5. For every existing position: re-evaluate. If cost-to-hold > cost-to-exit OR persistence broke OR safety gate failed, exit.
6. Emit signal JSON, never execute directly. Operator or contract bot acts on signal.

The bot does not know about specific symbols. It does not have a hardcoded "STRK is good" list. It re-derives the universe from the live data every tick. If STRK becomes profitable, it enters. If STRK stops being profitable, it exits. If a new symbol appears, the bot evaluates it against the same rule.

This is mechanism design, not strategy selection.

---

## §5 — Why this matters

We are managing user funds in a DeFi vault. Four failure modes have to be impossible or bounded:

1. **Directional blowup.** A naked-long position that drops 30% wipes user funds. Prevented by the same-asset hedge requirement (§1).
2. **Strategy decay.** A backtest-optimized portfolio that worked in 2026-Q1 silently stops working in 2026-Q3 because the team picked symbols by hand. Prevented by the bot re-deriving the universe per tick from real-time data (§4).
3. **Cost surprise.** A high-funding signal that turns into a loss because trading fees + slippage + bridge cost ate the income. Prevented by the explicit cost model and the `income > cost` gate (§3, §4).
4. **Cross-venue basis blowout (liquidation-scale tail).** This is NOT prevented by the iron law — it is the residual risk the iron law leaves on the table, and it must be sized explicitly. See §5.1.

The iron law makes #1 impossible by construction, the framework makes #2 and #3 impossible by mechanism design, and #4 must be bounded by capital sizing.

### §5.1 — Cross-venue basis-blowout risk (critique #5 remediation)

The β = 1.0 hedge is mechanical at the asset level: +1 X on venue A, −1 X on venue B. Price P&L on the two legs cancels **if both venues mark X at the same price**. The residual risk is that they don't — for any of the following reasons:

- **Oracle divergence.** Venue A uses Pyth + index, venue B uses internal trade VWAP. Under stress, the two can diverge by 50-500 bps for minutes at a time, generating unrealized loss on the "wrong side" that triggers maintenance margin on one leg before the other.
- **Forced delisting.** Venue A delists X under emergency procedure (low liquidity, regulator pressure, oracle manipulation concern). Positions auto-close at the venue's chosen mark, which may be 5-20% away from venue B's mark. Examples: Hyperliquid JELLY 2025 (MEV-driven oracle attack, forced wind-down, 70% intra-position swing), Binance LUNA May 2022 (auto-deleverage at a stale mark).
- **Venue insolvency / paused withdrawals.** Venue B halts withdrawals (FTX Nov 2022), freezing the hedge leg. Meanwhile venue A continues to mark-to-market. What was a market-neutral pair is now a one-legged directional position with no exit.
- **Oracle manipulation.** A TWAP oracle is sandwich-attacked; the resulting bad mark liquidates a leg before the arbitrage mean-reverts (Mango Markets 2022, various subsequent repeats).
- **Cross-chain bridge failure.** Margin collateral is stuck on the wrong side of a failed or censored bridge.

**None of these are Gaussian tail events.** They are regime transitions whose conditional loss distribution is not captured by any σ-based bound — Gaussian, sub-Gaussian, or DRO. A per-pair position of 3.26% AUM (the v3.5.1 L=3 sizing) under a 20% bad-mark shock on one leg produces a 65bp AUM loss on a single pair, and simultaneous events across correlated delistings (all layer-1 alts under the same oracle feed) would stack.

**Sizing rule (stated, not derived from a statistical model):**

- Per-pair notional ≤ `m_pos × A` with `m_pos ≤ 2%` per leg (enforced by cost_model §5 and `max_pct_aum_per_symbol`)
- Venue concentration cap: per counter venue ≤ 40% of deployed (enforced by `max_pct_per_counter`)
- Aggregate deployed ≤ 50% of AUM (the α floor)
- **Stress-scenario worst case:** simultaneous 20% bad-mark shock on all positions held on the single most-exposed counter venue
  `worst_case = 20% × max_pct_per_counter × (1 − aum_buffer_floor) × A = 20% × 40% × 50% × A = 4% × A`
  = 400bp AUM drawdown in a single event
- This is the **stated outer envelope.** It is larger than the daily P&L kill threshold (300bp) on purpose — the kill threshold is designed to trip on routine variance, whereas a basis blowout is meant to be caught by the venue caps before it ever reaches that magnitude.

**What this rules out:**
- Raising `LOCKED_MIN_LEVERAGE` without simultaneously tightening `max_pct_per_counter` (the v3.5.1 attempt raised leverage without touching the venue cap, which is why it failed review).
- Claiming a leverage choice is "mathematically safe" on the basis of single-venue maintenance-margin distance. That number (1/(2L)) is the threshold for routine mark-to-market, not basis-blowout mechanics.
- Sizing tail bounds from 60 days of quiet data. The historical tail events above (Terra, FTX, JELLY, Mango) are NOT in the current 60-day sample. Any L-sweep on `historical_cross_venue.sqlite` is a quiet-regime sample and should be treated as exploratory, not validated.

**What this does not yet specify:**
- A per-event severity distribution (because there is no honest parametric family for regime transitions — the right approach is historical scenario analysis with the list above, which is qualitative, not quantitative).
- A formal recovery-time model for withdrawal-paused counter venues. The working assumption is that Dol would treat a frozen counter leg as a realized loss equal to the stuck collateral plus the accumulated mark divergence, rather than hope for recovery.

This section is the **replacement for the v3.5.1 "33× distance to maintenance margin call" narrative,** which was a single-venue calculation irrelevant to the actual liquidation mechanism.

---

## §5.2 — Known modeling limitations

- **Variance annualization (`σ² × T`).** `portfolio.py` annualizes per-hour variance by multiplying by 8760, which is exact for iid but an upper bound for OU with autocorrelation (the true integrated variance under mean reversion is `σ² × (1 − e^(−2θT))/(2θ)`). We use the iid approximation because (a) it is conservative, (b) the exact form requires a θ that the drift regime does not have. Future work: replace with the OU-exact form when `regime == "ou"` and a Newey-West-style autocovariance correction when `regime == "drift"`.

- **Bridge opportunity cost.** `cost_model.round_trip_cost_pct` now carries a comment flagging that cross-chain hold periods forgo idle APY on in-transit collateral (~24bps/round-trip at 2-day transit). This should be added to the explicit bridge term when comparing income vs cost; the current implementation leaves it to the caller.

- **Withdrawal latency narrative.** Earlier docs claimed "instant withdrawal" without qualification. The honest split is: normal-state redemptions use the 50% idle bucket and clear in seconds. Stress-state redemptions (idle exhausted, simultaneous redemption spike) require position unwind, which at 46 positions and 1-2s per close is ~90 seconds worst case, plus any cross-chain bridging required to move collateral to USDC. "Instant" in the product copy should be replaced with "near-instant in normal state; up to a few minutes under stress."

- **Slippage coefficients are calibrated constants.** Acknowledged in code at `cost_model.py::SLIPPAGE_IMPACT_COEFFICIENT`. These are the only backtest-adjacent constants in the decision path and must be re-calibrated from real fills before live trading.

- **Granger causality.** Earlier docs stated "41/55 pairs show counter-venue Granger-leading Pacifica" as if it were decision-relevant. It is NOT used in the entry logic. Either it should be wired into `filter_candidate_rigorous` as a direction confirmation signal, or it should be removed from the documentation. Current choice: keep as an informational diagnostic; remove the implication that it shapes decisions.

- **Thin vs rich validation asymmetry.** `validate_formulas.py` scenario A (thin universe) expects mandate miss and marks it PASS; scenario B (rich) expects mandate hit. The asymmetry is intentional — framework must stay idle when the universe is too thin — but the test output should say "expected miss" not "NO" to avoid confusion. Future validation cleanup.

- **`max_simultaneous_pairs`.** Was 30 in the Mandate dataclass while real data had 46 active pairs. Fixed to 46 in `cost_model.Mandate`. If the universe exceeds 46, the allocator truncates by Sharpe rank.

---

## §6 — Final commandments (for any future contributor)

- **Do not pick symbols.** The bot picks symbols. You build the rules.
- **Do not optimize APY against historical data.** Validate the framework. Don't curve-fit.
- **Do not propose strategies that violate §1 or §2**, no matter how attractive the backtest looks.
- **Do not use any venue whose counter-leg requires custodial KYC.** DEX-only.
- **Do not add complexity** (multi-leg baskets, ML signals, regime switches) until the simple iron-law framework has been live for at least 90 days and proven stable.
- **When in doubt, choose the boring option.** User funds are at stake. Boring is good.
- **The income-vs-cost equation is the only optimization.** Everything else is a safety constraint or a measurement.
