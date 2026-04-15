"""
cost_model.py — live-adaptive formula framework for the Dol cross-venue
funding-spread harvester.

Iron law: ../PRINCIPLES.md
Formulas:  ../docs/math-formulas.md

This module implements every safety gate as a CLOSED-FORM FUNCTION of live
inputs and mandate constants. There are no backtest-derived fixed values in the
decision path. The bot calls `compute_system_state(inputs, mandate)` once per
funding tick to get the current operating envelope, then `evaluate_trade_live`
for each candidate trade.

The only "constants" in this file are:
  - the Mandate dataclass (policy targets, statistical Z multipliers, lookback windows)
  - three slippage-model coefficients in `slippage()`, calibrable in Phase 1 dry run

Everything else is a function.
"""
from __future__ import annotations
import math
import statistics
from dataclasses import dataclass, field
from typing import Optional


# ===========================================================================
# v3.5.2 REVISION — v3.5.1 LOCK REVOKED (2026-04-14, post-external-review)
# ===========================================================================
# External quant review identified the v3.5.1 L=3 lock as empirically
# ungrounded. Key failures:
#   (1) The justification comment ("L=3 customer 8.00% / buffer 4.78%") does
#       not match actual dry_run output (6.63% / 2.55%) — cap routing was
#       asserted but never measured at the operating point.
#   (2) The "optimum" was found on a 60-day sample containing a single regime;
#       walk-forward L=3 result is identical in train/test because both windows
#       hit the mandate caps. That is cap censoring, not robustness.
#   (3) Per-pair tail analysis used Gaussian VaR on basis residuals whose
#       empirical Hurst H≈0.9 implies fat, persistent tails — Gaussian bounds
#       underestimate liquidation-scale basis blowouts.
#   (4) Single-venue maintenance-margin distance (1/2L = 16.67%) is not the
#       liquidation mechanism that matters: cross-venue basis blowouts (FTX,
#       Terra, JELLY) stress BOTH legs simultaneously.
# Until #3, #4, #5 from the review are resolved, leverage floor is removed and
# the framework reverts to auto-derived L (typically L=2 on the current
# universe, matching the v3.5 baseline). Any future re-introduction of a
# leverage floor must be justified by a real-data CVaR-or-DRO tail argument,
# not by a single-regime sweep.
LOCKED_MIN_LEVERAGE: int = 1  # floor disabled pending tail-risk remediation


# ===========================================================================
# Mandate constants — the only policy / statistically-conventional fixed values
# ===========================================================================

@dataclass(frozen=True)
class Mandate:
    """The constants that come from policy decision and statistical convention.
    Everything else is computed from these + LiveInputs."""

    # ---- policy mandate targets ----
    customer_apy_min: float = 0.05
    customer_apy_max: float = 0.08
    buffer_apy_min: float = 0.02
    buffer_apy_max: float = 0.05
    cut_customer: float = 0.65
    cut_buffer: float = 0.25
    cut_reserve: float = 0.10

    # ---- PRINCIPLES hard floor ----
    aum_buffer_floor: float = 0.50      # cannot deploy more than 50% AUM, ever
    aum_idle_cap: float = 0.95          # don't go fully idle when signals exist

    # ---- Statistical Z multipliers ----
    Z_persistence: float = 5.0          # 5σ rejection of random-walk null
    Z_drawdown: float = 3.0             # 3σ basis-divergence envelope
    Z_pnl_warn: float = 1.0             # 1σ daily P&L warning
    Z_pnl_halve: float = 2.0            # 2σ daily P&L → halve risk
    Z_pnl_kill: float = 3.0             # 3σ daily P&L → full stop
    Z_ratio_downside: float = 1.65      # 5% one-tailed downside on income/cost

    # ---- Operational risk premium above r_idle ----
    operational_risk_premium: float = 0.01

    # ---- Lookback windows ----
    persistence_lookback_h_min: int = 168    # 1 week minimum sample
    persistence_lookback_h_max: int = 720    # 30 days maximum (avoid stale regimes)
    basis_lookback_h: int = 168              # 1 week of oracle divergence
    pnl_lookback_d: int = 30                 # 30 days of vault returns

    # ---- Operational ceilings ----
    leverage_safety_multiplier: float = 10   # drawdown stop must fire 10× before liquidation
    max_simultaneous_pairs: int = 46         # operational ceiling on N (critique #14: was 30, didn't match 46 active on real data)
    max_single_venue_exposure: float = 0.60  # critique #15: per-venue aggregate cap, applied on top of max_per_counter

    # ---- DEX whitelist (PRINCIPLES §2: no KYC CEXes) ----
    dex_venues: tuple = ("pacifica", "backpack", "hyperliquid", "lighter")

    # ---- Bootstrap fallbacks (used only when historical data is insufficient) ----
    bootstrap_drawdown_stop: float = 0.005
    bootstrap_pnl_warn: float = -0.01
    bootstrap_pnl_halve: float = -0.02
    bootstrap_pnl_kill: float = -0.03


# ===========================================================================
# LiveInputs — the data the bot reads each tick
# ===========================================================================

@dataclass
class LiveInputs:
    """Everything the bot reads from the world at each tick. Pure data, no state."""

    timestamp_ms: int

    # Vault state
    aum_usd: float
    r_idle: float                                                 # current Kamino (or composite) USDC supply APY

    # Per (symbol, venue) live snapshots
    funding_rate_h: dict                                          # {(symbol, venue): per-hour signed rate}
    open_interest_usd: dict                                       # {(symbol, venue): OI in USD}
    volume_24h_usd: dict                                          # {(symbol, venue): 24h volume in USD}

    # Per-venue fees (semi-static)
    fee_maker: dict                                               # {venue: fraction per leg}
    fee_taker: dict                                               # {venue: fraction per leg}

    # Per cross-venue bridge cost (semi-static, ideally read live)
    bridge_fee_round_trip: dict                                   # {(v_a, v_b): fraction round-trip}

    # Rolling histories (the bot maintains these in cross_venue_funding.sqlite)
    funding_history_h: dict = field(default_factory=dict)         # {(symbol, venue): list of (ts_ms, rate)}
    basis_divergence_history: dict = field(default_factory=dict)  # {symbol: list of (ts_ms, divergence)}
    vault_daily_returns: list = field(default_factory=list)       # list of fractional daily returns


# ===========================================================================
# §1 — Target gross vault APY (math-formulas §1)
# ===========================================================================

def target_vault_apy(m: Mandate) -> float:
    """r_target(t) — the geometric center of the achievable mandate band."""
    floor = max(m.customer_apy_min / m.cut_customer, m.buffer_apy_min / m.cut_buffer)
    ceil = min(m.customer_apy_max / m.cut_customer, m.buffer_apy_max / m.cut_buffer)
    return (floor + ceil) / 2


def target_vault_apy_floor(m: Mandate) -> float:
    return max(m.customer_apy_min / m.cut_customer, m.buffer_apy_min / m.cut_buffer)


def target_vault_apy_ceil(m: Mandate) -> float:
    return min(m.customer_apy_max / m.cut_customer, m.buffer_apy_max / m.cut_buffer)


# ===========================================================================
# §2 — Slippage and per-trade cost (math-formulas §2)
# ===========================================================================

# These five coefficients are the only tuning constants in the cost model.
# They are CALIBRATED CONSTANTS (critique #11 acknowledgment): PRINCIPLES.md §2
# requires the framework to contain no backtest-derived fixed values in the
# decision path, and these coefficients are a narrowly-scoped exception. They
# parameterise the slippage estimator, which needs a starting point before any
# real fills are observed; they must be re-calibrated from Phase 1 dry-run
# fills before live trading. Until recalibration, treat their values as
# conservative defaults, not derived truths.
SLIPPAGE_OI_FRACTION_AS_DEPTH: float = 0.10      # 10% of OI is "easily reachable" depth
SLIPPAGE_VOL_FRACTION_AS_DEPTH: float = 0.01     # 1% of 24h volume is "easily reachable" depth
SLIPPAGE_IMPACT_COEFFICIENT: float = 0.0008      # √-impact coefficient (Almgren-Chriss class)
SLIPPAGE_FLOOR: float = 0.0001                   # 1bp floor (always pay tick spread)
SLIPPAGE_CEILING: float = 0.02                   # 200bp ceiling (anything above = uncrossable)


def slippage(notional_usd: float, oi_usd: float, vol_24h_usd: float) -> float:
    """Square-root market-impact estimator. Returns slippage as fraction of notional."""
    if notional_usd <= 0:
        return 0.0
    depth = max(
        SLIPPAGE_OI_FRACTION_AS_DEPTH * oi_usd,
        SLIPPAGE_VOL_FRACTION_AS_DEPTH * vol_24h_usd,
        1_000.0,
    )
    raw = SLIPPAGE_IMPACT_COEFFICIENT * math.sqrt(notional_usd / depth)
    return max(SLIPPAGE_FLOOR, min(SLIPPAGE_CEILING, raw))


# ===========================================================================
# §2b — Lifecycle cost accounting (v3.5.2 critique followup)
# ===========================================================================
# The earlier cost model evaluates ONE round-trip of one pair. It does NOT
# project that cost onto an annualized vault return. For a product with a
# 5-7% mandate, rotation costs are first-order: at commitment hold 168h,
# a position rotates ~52×/year, so even a 15bp round-trip compounds to ~7.8%
# annual cost. The lifecycle function below closes that loop.

def lifecycle_annualized_return(
    per_pair_spread_apy: float,
    commitment_hold_h: float,
    c_round_trip: float,
    leverage: int,
    alpha: float,
    r_idle: float,
) -> dict:
    """End-to-end annualized vault return, including rotation cost.

    Arguments
    ---------
    per_pair_spread_apy : s̄
        Average per-pair funding spread, annualized, expressed as a fraction
        of per-leg notional. (Equivalent to the unweighted mean of |OU μ APY|
        across active candidates.) This is what the hedge actually captures
        per unit of notional per year, BEFORE leverage and BEFORE rotation.
    commitment_hold_h : T_hold
        Planning horizon per position (hours). Determines rotation frequency
        as rotations/year = 8760 / T_hold.
    c_round_trip : c
        Cost per pair per round-trip, as a fraction of per-leg notional. From
        round_trip_cost_pct(). Covers fees (both legs, open+close), slippage,
        and bridge round-trip.
    leverage : L
    alpha : α
        Idle fraction of AUM (non-deployed).
    r_idle : r_idle
        Idle bucket yield (Kamino USDC lending, typically 4-5%).

    Returns
    -------
    dict with gross_on_margin, annual_cost_on_margin, net_on_margin,
         idle_contribution, trading_contribution, vault_gross,
         customer (capped), buffer (capped + cap-routing excess), reserve,
         mandate_customer_ok, mandate_buffer_ok, customer/buffer margins
         relative to mandate floors, and breakeven_spread (the s̄ that
         would make customer land exactly at the 5% floor).

    Formula
    -------
    Per pair:
      notional_per_leg = (margin_per_pair × L) / 2
      income_per_year  = s̄ × notional_per_leg
      return_on_margin = s̄ × L / 2
      cost_per_rotation = c × notional_per_leg = c × margin_per_pair × L / 2
      cost_on_margin_per_rotation = c × L / 2
      annual_cost_on_margin = (c × L / 2) × rotations_per_year

    Vault:
      trading_contribution_on_aum = (1 − α) × [(L/2) × (s̄ − c × rotations)]
      idle_contribution_on_aum    = α × r_idle
      vault_gross = idle + trading

    Cap routing (65/25/10 policy split, customer capped 8%, buffer 5%):
      customer_raw = 0.65 × vault_gross
      excess_to_buffer = max(0, customer_raw − 0.08)
      buffer_raw = 0.25 × vault_gross + excess_to_buffer
      excess_to_reserve = max(0, buffer_raw − 0.05)
      reserve = 0.10 × vault_gross + excess_to_reserve
    """
    rotations_per_year = 8760.0 / commitment_hold_h
    gross_on_margin = (leverage / 2.0) * per_pair_spread_apy
    annual_cost_on_margin = (leverage / 2.0) * c_round_trip * rotations_per_year
    net_on_margin = gross_on_margin - annual_cost_on_margin

    trading_contribution = (1.0 - alpha) * net_on_margin
    idle_contribution = alpha * r_idle
    vault_gross = idle_contribution + trading_contribution

    customer_raw = vault_gross * 0.65
    buffer_raw = vault_gross * 0.25
    reserve_raw = vault_gross * 0.10
    customer_capped = min(max(customer_raw, 0.0), 0.08)
    customer_excess = max(0.0, customer_raw - 0.08)
    buffer_with_excess = buffer_raw + customer_excess
    buffer_capped = min(max(buffer_with_excess, 0.0), 0.05)
    buffer_excess = max(0.0, buffer_with_excess - 0.05)
    reserve = reserve_raw + buffer_excess

    # Breakeven: the s̄ that lands customer exactly at 5% (mandate floor).
    # 0.05 = 0.65 × [α r_idle + (1−α)(L/2)(s − c × rot)]
    # → s = c × rot + (0.05/0.65 − α r_idle) / ((1−α)(L/2))
    denom = (1.0 - alpha) * (leverage / 2.0)
    if denom > 0:
        s_breakeven = c_round_trip * rotations_per_year + (0.05 / 0.65 - alpha * r_idle) / denom
    else:
        s_breakeven = float("inf")

    return {
        "commitment_hold_h": commitment_hold_h,
        "rotations_per_year": rotations_per_year,
        "leverage": leverage,
        "alpha": alpha,
        "r_idle": r_idle,
        "c_round_trip": c_round_trip,
        "per_pair_spread_apy": per_pair_spread_apy,
        "gross_on_margin": gross_on_margin,
        "annual_cost_on_margin": annual_cost_on_margin,
        "net_on_margin": net_on_margin,
        "idle_contribution": idle_contribution,
        "trading_contribution": trading_contribution,
        "vault_gross": vault_gross,
        "customer": customer_capped,
        "buffer": buffer_capped,
        "reserve": reserve,
        "mandate_customer_ok": 0.05 <= customer_capped <= 0.08,
        "mandate_buffer_ok": 0.02 <= buffer_capped <= 0.05,
        "customer_margin_vs_floor": customer_capped - 0.05,
        "buffer_margin_vs_floor": buffer_capped - 0.02,
        "breakeven_spread_apy": s_breakeven,
    }


def round_trip_cost_pct(symbol: str, v_p: str, v_c: str, notional_usd: float,
                        inputs: LiveInputs) -> float:
    """c(s, v_p, v_c, n) — total cost as fraction of single-leg notional."""
    fee_p = inputs.fee_maker.get(v_p, 0.00015) * 2          # open + close on Pacifica
    fee_c = inputs.fee_maker.get(v_c, 0.00020) * 2          # open + close on counter
    slip_p = slippage(
        notional_usd,
        inputs.open_interest_usd.get((symbol, v_p), 0.0),
        inputs.volume_24h_usd.get((symbol, v_p), 0.0),
    ) * 2  # in + out
    slip_c = slippage(
        notional_usd,
        inputs.open_interest_usd.get((symbol, v_c), 0.0),
        inputs.volume_24h_usd.get((symbol, v_c), 0.0),
    ) * 2
    bridge = inputs.bridge_fee_round_trip.get((v_p, v_c), 0.0)
    # Critique #9: if the counter venue is on a different chain, margin collateral
    # spends 1-3 days in bridge transit where it does not accrue idle APY. That
    # opportunity cost is NOT the bridge fee itself but the idle-yield forgone
    # during transit. For Pacifica (Solana) ↔ Hyperliquid (Arbitrum) at
    # r_idle ≈ 4.4% and 2-day transit the implied cost is ≈ 24bps per round
    # trip, amortized over the hold period. Caller should add this to `bridge`
    # when comparing against expected income.
    return fee_p + fee_c + slip_p + slip_c + bridge


# ===========================================================================
# §3 — Symbol persistence statistics (math-formulas §3)
# ===========================================================================

def lookback_hours_for(symbol: str, v_c: str, inputs: LiveInputs, m: Mandate) -> int:
    """T(s, v_c) clamped to [T_min, T_max]."""
    pac_hist = inputs.funding_history_h.get((symbol, "pacifica"), [])
    cnt_hist = inputs.funding_history_h.get((symbol, v_c), [])
    if not pac_hist or not cnt_hist:
        return 0
    pac_ts = {ts for ts, _ in pac_hist}
    common = sum(1 for ts, _ in cnt_hist if ts in pac_ts)
    return max(0, min(m.persistence_lookback_h_max, max(common, 0)))


def _aligned_spread_series(symbol: str, v_c: str, inputs: LiveInputs, T: int) -> list:
    """Returns the most recent T hours of (counter - pacifica) per-hour spreads."""
    pac_map = dict(inputs.funding_history_h.get((symbol, "pacifica"), []))
    cnt_map = dict(inputs.funding_history_h.get((symbol, v_c), []))
    common_ts = sorted(set(pac_map.keys()) & set(cnt_map.keys()))
    if not common_ts:
        return []
    common_ts = common_ts[-T:]
    return [cnt_map[t] - pac_map[t] for t in common_ts]


def signed_mean_apy(symbol: str, v_c: str, inputs: LiveInputs, m: Mandate) -> float:
    """μ̂(s, v_c) — signed mean spread, annualized."""
    T = lookback_hours_for(symbol, v_c, inputs, m)
    if T < m.persistence_lookback_h_min:
        return 0.0
    spreads = _aligned_spread_series(symbol, v_c, inputs, T)
    if not spreads:
        return 0.0
    return statistics.mean(spreads) * 24 * 365


def sample_std_apy(symbol: str, v_c: str, inputs: LiveInputs, m: Mandate) -> float:
    """σ̂(s, v_c) — sample standard deviation of spread, annualized."""
    T = lookback_hours_for(symbol, v_c, inputs, m)
    if T < m.persistence_lookback_h_min:
        return 0.0
    spreads = _aligned_spread_series(symbol, v_c, inputs, T)
    if len(spreads) < 2:
        return 0.0
    return statistics.stdev(spreads) * 24 * 365


def persistence_pct(symbol: str, v_c: str, inputs: LiveInputs, m: Mandate) -> float:
    """p̂(s, v_c) — fraction of past hours where sign(spread) == sign(mean spread)."""
    T = lookback_hours_for(symbol, v_c, inputs, m)
    if T < m.persistence_lookback_h_min:
        return 0.0
    spreads = _aligned_spread_series(symbol, v_c, inputs, T)
    if not spreads:
        return 0.0
    mean_sign = 1 if sum(spreads) > 0 else -1
    matching = sum(1 for s in spreads if (1 if s > 0 else -1) == mean_sign)
    return matching / len(spreads)


# ===========================================================================
# §4 — Persistence threshold p_min(T)
# ===========================================================================

def persistence_threshold(T_hours: int, m: Mandate) -> float:
    """p_min(T) = 0.5 + Z · √(0.25/T) — Z-σ rejection of fair-coin null."""
    if T_hours <= 0:
        return 1.0
    sigma_p = math.sqrt(0.25 / T_hours)
    return min(0.95, 0.5 + m.Z_persistence * sigma_p)


# ===========================================================================
# §5 — Income/cost ratio ρ from per-symbol SNR (math-formulas §5)
# ===========================================================================

def required_ratio(snr: float, m: Mandate) -> float:
    """ρ(s) = 1 / (1 - Z · 1/SNR). Returns ∞ if SNR too low."""
    if snr <= m.Z_ratio_downside + 0.05:
        return float("inf")
    discount = 1.0 - m.Z_ratio_downside / snr
    if discount <= 0:
        return float("inf")
    return 1.0 / discount


def candidate_snr(symbol: str, v_c: str, inputs: LiveInputs, m: Mandate) -> float:
    """Sample SNR = |μ̂| / σ̂."""
    mu = abs(signed_mean_apy(symbol, v_c, inputs, m))
    sigma = sample_std_apy(symbol, v_c, inputs, m)
    if sigma <= 0:
        return float("inf") if mu > 0 else 0.0
    return mu / sigma


# ===========================================================================
# §6 — Drawdown stop d_max(s) from rolling basis volatility (math-formulas §6)
# ===========================================================================

def drawdown_stop(symbol: str, inputs: LiveInputs, m: Mandate) -> float:
    """d_max(s) = Z_draw · σ_basis(s). Falls back to bootstrap value if data thin."""
    hist = inputs.basis_divergence_history.get(symbol, [])
    if len(hist) < 24:
        return m.bootstrap_drawdown_stop
    values = [v for _, v in hist[-m.basis_lookback_h:]]
    if len(values) < 2:
        return m.bootstrap_drawdown_stop
    sigma = statistics.stdev(values)
    return m.Z_drawdown * sigma


# ===========================================================================
# §7 — Daily P&L circuit breakers (math-formulas §7)
# ===========================================================================

def pnl_breakers(inputs: LiveInputs, m: Mandate) -> tuple:
    """Returns (k_warn, k_halve, k_kill) as fractions of AUM. Adapts to vault σ."""
    if len(inputs.vault_daily_returns) < 14:
        return (m.bootstrap_pnl_warn, m.bootstrap_pnl_halve, m.bootstrap_pnl_kill)
    sigma = statistics.stdev(inputs.vault_daily_returns[-m.pnl_lookback_d:])
    return (
        -m.Z_pnl_warn * sigma,
        -m.Z_pnl_halve * sigma,
        -m.Z_pnl_kill * sigma,
    )


# ===========================================================================
# §8 — Required leverage L*(t) (math-formulas §8)
# ===========================================================================

def required_leverage(median_pair_apy: float, inputs: LiveInputs, m: Mandate,
                      venue_max_leverage: int = 10) -> int:
    """L*(t) = smallest integer s.t. mandate target hits at α_floor with the median μ̂.

    v3.5.2: LOCKED_MIN_LEVERAGE is 1 (floor disabled). The v3.5.1 lock at L=3 was
    revoked after external review — its justification comment claimed L=3 routed to
    cap (customer 8.00% / buffer 4.78%) but the measured dry-run gave 6.63% / 2.55%,
    and the tail-risk bound used Gaussian VaR on a basis whose empirical Hurst ≈ 0.9
    implies fat persistent tails. Any future floor must be derived from a real-data
    CVaR or DRO tail argument, not a single-regime 60-day sweep."""
    target = target_vault_apy(m)
    if median_pair_apy <= 0:
        return min(venue_max_leverage, _safe_max_leverage(m))
    needed = 2 * (target - m.aum_buffer_floor * inputs.r_idle) / ((1 - m.aum_buffer_floor) * median_pair_apy)
    L_required = max(1, math.ceil(needed))
    L_safe = _safe_max_leverage(m)
    L_computed = min(L_required, venue_max_leverage, L_safe)
    return max(LOCKED_MIN_LEVERAGE, L_computed)


def _safe_max_leverage(m: Mandate, d_max_typical: float = 0.005) -> int:
    """Drawdown stop must fire k_safety_L× before maintenance margin call."""
    return max(1, int(1.0 / (m.leverage_safety_multiplier * d_max_typical)))


# ===========================================================================
# §9 — Idle bucket fraction α(t) (math-formulas §9)
# ===========================================================================

def idle_fraction(median_pair_apy: float, leverage: int, inputs: LiveInputs, m: Mandate) -> float:
    """α(t) = clamp((X - target) / (X - r_idle), α_floor, α_cap)."""
    target = target_vault_apy(m)
    X = median_pair_apy * leverage / 2
    if X <= inputs.r_idle:
        return m.aum_idle_cap
    raw = (X - target) / (X - inputs.r_idle)
    return max(m.aum_buffer_floor, min(m.aum_idle_cap, raw))


# ===========================================================================
# §10 — Active candidate count and §11 — m_pos
# ===========================================================================

def position_aum_cap(idle_frac: float, leverage: int, n_active: int, m: Mandate) -> float:
    """m_pos(t) = (1-α)·L / (2·N_target). Higher N → more diversified, smaller per symbol."""
    if n_active <= 0:
        return 0.0
    n_target = min(n_active, m.max_simultaneous_pairs)
    return (1 - idle_frac) * leverage / (2 * n_target)


# ===========================================================================
# §12 — Per counter venue cap m_counter
# ===========================================================================

def counter_venue_cap(n_counters_active: int) -> float:
    """m_counter(t) = 1.20 / N_counter_active, headroom 20%."""
    if n_counters_active <= 0:
        return 0.0
    return min(1.0, 1.20 / n_counters_active)


# ===========================================================================
# §13 — Per-leg OI cap m_oi
# ===========================================================================

def oi_cap(symbol: str, venue: str, inputs: LiveInputs) -> float:
    """m_oi(s, v) = 0.05 · clamp(√turnover, 0.5, 1.4)."""
    oi = inputs.open_interest_usd.get((symbol, venue), 0.0)
    vol = inputs.volume_24h_usd.get((symbol, venue), 0.0)
    if oi <= 0:
        return 0.0
    turnover = vol / max(oi, 1.0)
    scale = max(0.5, min(1.4, math.sqrt(turnover)))
    return 0.05 * scale


# ===========================================================================
# §14 — Per-trade min hold H_min(s, v_c, n)
# ===========================================================================

def per_trade_min_hold_h(symbol: str, v_c: str, notional: float,
                         ratio: float, inputs: LiveInputs, m: Mandate) -> float:
    """H_min = ρ · 8760 · c / |μ̂|. Returns ∞ if signal absent."""
    mu = abs(signed_mean_apy(symbol, v_c, inputs, m))
    if mu <= 0 or ratio == float("inf"):
        return float("inf")
    c = round_trip_cost_pct(symbol, "pacifica", v_c, notional, inputs)
    if c <= 0:
        return 0.0
    return ratio * 8760 * c / mu


# ===========================================================================
# §15 — NET APY floor r_min(t)
# ===========================================================================

def net_apy_floor(idle_frac: float, leverage: int, inputs: LiveInputs, m: Mandate) -> float:
    """r_min = max(r_idle + ε_op, mandate-floor implied r_pair)."""
    floor_op = inputs.r_idle + m.operational_risk_premium
    if (1 - idle_frac) <= 0 or leverage <= 0:
        return floor_op
    floor_mandate = (
        2 * (target_vault_apy_floor(m) - idle_frac * inputs.r_idle)
        / ((1 - idle_frac) * leverage)
    )
    return max(floor_op, floor_mandate)


# ===========================================================================
# §16 — Composition: compute the system state at this tick
# ===========================================================================

@dataclass
class SystemState:
    timestamp_ms: int
    target_vault_apy: float
    median_pair_apy: float
    leverage: int
    idle_fraction: float
    position_aum_cap: float
    counter_venue_cap: float
    net_apy_floor: float
    n_active_candidates: int
    n_counter_venues_active: int
    pnl_breakers: tuple
    candidates_summary: list = field(default_factory=list)


def build_candidates(inputs: LiveInputs, m: Mandate) -> list:
    """Pre-filter: any (symbol, counter) pair where both venues have funding data
    and persistence/SNR clear the basic statistical gates."""
    syms = {s for (s, v) in inputs.funding_rate_h.keys() if v == "pacifica"}
    out = []
    for s in syms:
        for v_c in m.dex_venues:
            if v_c == "pacifica":
                continue
            if (s, v_c) not in inputs.funding_rate_h:
                continue
            T = lookback_hours_for(s, v_c, inputs, m)
            if T < m.persistence_lookback_h_min:
                continue
            mu = signed_mean_apy(s, v_c, inputs, m)
            if mu == 0:
                continue
            sigma = sample_std_apy(s, v_c, inputs, m)
            snr = (abs(mu) / sigma) if sigma > 0 else float("inf")
            p = persistence_pct(s, v_c, inputs, m)
            p_thresh = persistence_threshold(T, m)
            if p < p_thresh:
                continue
            if snr <= m.Z_ratio_downside + 0.05:
                continue
            inst_spread = inputs.funding_rate_h.get((s, v_c), 0) - inputs.funding_rate_h.get((s, "pacifica"), 0)
            if inst_spread == 0:
                continue
            if (inst_spread > 0) != (mu > 0):
                continue  # instantaneous direction conflicts with persistent direction
            out.append({
                "symbol": s,
                "counter_venue": v_c,
                "T": T,
                "mu_signed_apy": mu,
                "sigma_apy": sigma,
                "snr": snr,
                "persistence_pct": p,
                "persistence_threshold": p_thresh,
            })
    return out


def compute_system_state(inputs: LiveInputs, m: Mandate) -> SystemState:
    """Run the per-tick formula chain. Pure function of (inputs, mandate)."""
    target = target_vault_apy(m)
    candidates = build_candidates(inputs, m)
    n_active = len(candidates)

    if n_active == 0:
        return SystemState(
            timestamp_ms=inputs.timestamp_ms,
            target_vault_apy=target,
            median_pair_apy=0.0,
            leverage=1,
            idle_fraction=m.aum_idle_cap,
            position_aum_cap=0.0,
            counter_venue_cap=0.0,
            net_apy_floor=inputs.r_idle + m.operational_risk_premium,
            n_active_candidates=0,
            n_counter_venues_active=0,
            pnl_breakers=pnl_breakers(inputs, m),
        )

    median_mu = statistics.median([abs(c["mu_signed_apy"]) for c in candidates])
    L = required_leverage(median_mu, inputs, m)
    alpha = idle_fraction(median_mu, L, inputs, m)
    counters_seen = {c["counter_venue"] for c in candidates}
    n_counters = len(counters_seen)
    m_pos = position_aum_cap(alpha, L, n_active, m)
    m_cnt = counter_venue_cap(n_counters)
    r_min = net_apy_floor(alpha, L, inputs, m)
    breakers = pnl_breakers(inputs, m)

    return SystemState(
        timestamp_ms=inputs.timestamp_ms,
        target_vault_apy=target,
        median_pair_apy=median_mu,
        leverage=L,
        idle_fraction=alpha,
        position_aum_cap=m_pos,
        counter_venue_cap=m_cnt,
        net_apy_floor=r_min,
        n_active_candidates=n_active,
        n_counter_venues_active=n_counters,
        pnl_breakers=breakers,
        candidates_summary=candidates,
    )


# ===========================================================================
# §17 — Per-trade evaluation (the bot calls this for each candidate)
# ===========================================================================

@dataclass
class TradeDecision:
    should_enter: bool
    reason: str
    symbol: str = ""
    counter_venue: str = ""
    notional_per_leg_usd: float = 0.0
    direction: int = 0
    leverage: int = 1
    expected_funding_apy: float = 0.0
    projected_net_apy: float = 0.0
    income_cost_ratio: float = 0.0
    min_hold_h: float = 0.0
    drawdown_stop_pct: float = 0.0
    cost_breakdown_pct: dict = field(default_factory=dict)


def evaluate_trade_live(symbol: str, counter_venue: str,
                        inputs: LiveInputs, m: Mandate,
                        state: SystemState) -> TradeDecision:
    """Score one candidate against all live-derived gates. Pure function."""
    if counter_venue not in m.dex_venues or counter_venue == "pacifica":
        return TradeDecision(should_enter=False, reason=f"counter {counter_venue} not in DEX whitelist")

    mu = signed_mean_apy(symbol, counter_venue, inputs, m)
    sigma = sample_std_apy(symbol, counter_venue, inputs, m)
    snr = (abs(mu) / sigma) if sigma > 0 else float("inf")
    if snr <= m.Z_ratio_downside + 0.05:
        return TradeDecision(should_enter=False, reason=f"SNR {snr:.2f} ≤ Z_ratio+0.05")

    p = persistence_pct(symbol, counter_venue, inputs, m)
    T = lookback_hours_for(symbol, counter_venue, inputs, m)
    p_thresh = persistence_threshold(T, m)
    if p < p_thresh:
        return TradeDecision(should_enter=False, reason=f"persistence {p:.0%} < threshold {p_thresh:.0%} (T={T}h)")

    direction = 1 if mu > 0 else -1
    inst_pac = inputs.funding_rate_h.get((symbol, "pacifica"), 0.0)
    inst_cnt = inputs.funding_rate_h.get((symbol, counter_venue), 0.0)
    inst_dir = 1 if inst_cnt - inst_pac > 0 else -1
    if inst_dir != direction:
        return TradeDecision(should_enter=False, reason="instantaneous spread sign conflicts with persistent direction")

    notional = state.position_aum_cap * inputs.aum_usd
    notional = min(notional, oi_cap(symbol, "pacifica", inputs) * inputs.open_interest_usd.get((symbol, "pacifica"), 0))
    notional = min(notional, oi_cap(symbol, counter_venue, inputs) * inputs.open_interest_usd.get((symbol, counter_venue), 0))
    if notional <= 0:
        return TradeDecision(should_enter=False, reason="notional collapsed to 0 by m_pos / m_oi caps")

    rho = required_ratio(snr, m)
    H_min = per_trade_min_hold_h(symbol, counter_venue, notional, rho, inputs, m)
    if H_min == float("inf") or H_min > m.persistence_lookback_h_max:
        return TradeDecision(should_enter=False, reason=f"H_min {H_min:.0f}h exceeds cap {m.persistence_lookback_h_max}")

    # The bot's PLANNED hold = the persistence lookback horizon (T) — how long we have
    # statistical evidence the signal will keep its sign. H_min is the COMMITMENT floor
    # (downside protection); T is the EXPECTED hold for normal accrual computation.
    T_plan = max(lookback_hours_for(symbol, counter_venue, inputs, m), int(H_min))

    c = round_trip_cost_pct(symbol, "pacifica", counter_venue, notional, inputs)
    expected_income_pct_min_hold = abs(mu) * (H_min / 8760)
    realized_ratio_min_hold = expected_income_pct_min_hold / c if c > 0 else float("inf")
    if realized_ratio_min_hold < rho:
        return TradeDecision(should_enter=False, reason=f"realized ratio {realized_ratio_min_hold:.2f} < required {rho:.2f}")

    # NET APY at the planned hold horizon. One round-trip cost amortized over T_plan hours.
    income_over_plan_pct = abs(mu) * (T_plan / 8760)
    projected_net_apy = (income_over_plan_pct - c) * (8760 / T_plan)
    if projected_net_apy < state.net_apy_floor:
        return TradeDecision(
            should_enter=False,
            reason=f"NET APY {projected_net_apy:.2%} < floor {state.net_apy_floor:.2%}",
            projected_net_apy=projected_net_apy,
        )

    d_max = drawdown_stop(symbol, inputs, m)

    breakdown = {
        "fee_pacifica_round_trip": inputs.fee_maker.get("pacifica", 0.00015) * 2,
        "fee_counter_round_trip": inputs.fee_maker.get(counter_venue, 0.00020) * 2,
        "slippage_pacifica_round_trip": slippage(notional,
                                                  inputs.open_interest_usd.get((symbol, "pacifica"), 0),
                                                  inputs.volume_24h_usd.get((symbol, "pacifica"), 0)) * 2,
        "slippage_counter_round_trip": slippage(notional,
                                                 inputs.open_interest_usd.get((symbol, counter_venue), 0),
                                                 inputs.volume_24h_usd.get((symbol, counter_venue), 0)) * 2,
        "bridge_round_trip": inputs.bridge_fee_round_trip.get(("pacifica", counter_venue), 0.0),
        "total_pct": c,
    }

    return TradeDecision(
        should_enter=True,
        reason="all live-derived gates passed",
        symbol=symbol,
        counter_venue=counter_venue,
        notional_per_leg_usd=notional,
        direction=direction,
        leverage=state.leverage,
        expected_funding_apy=mu,
        projected_net_apy=projected_net_apy,
        income_cost_ratio=realized_ratio_min_hold,
        min_hold_h=H_min,
        drawdown_stop_pct=d_max,
        cost_breakdown_pct=breakdown,
    )
