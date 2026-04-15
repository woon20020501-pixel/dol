"""
validate_formulas.py — synthetic-input validation of the live-adaptive
cost_model.py framework.

Runs two scenarios:
  (A) thin universe — small-OI symbols (real April 14 snapshot values)
  (B) rich universe — 12 deep-OI symbols, median 22% pair APY

Validates that:
  - All formulas evaluate without error
  - Under (A), the framework correctly stays mostly idle (sub-mandate is honest)
  - Under (B), the framework derives an L, α, m_pos, r_min that produces a
    vault projection inside the customer/buffer mandate band

Run anytime to re-verify after editing strategy/cost_model.py.
"""
import os
import random
import statistics
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from strategy.cost_model import (
    Mandate, LiveInputs, compute_system_state, evaluate_trade_live,
    target_vault_apy, persistence_threshold, required_ratio, slippage,
)


def synth_history(mean_per_h: float, std_per_h: float, T: int = 720, seed_base: int = 1776140000000):
    return [(seed_base - (T - i) * 3600000, random.gauss(mean_per_h, std_per_h)) for i in range(T)]


def run_scenario(label: str, m: Mandate, inputs: LiveInputs):
    print(f"\n{'='*100}")
    print(f"SCENARIO: {label}")
    print('=' * 100)
    state = compute_system_state(inputs, m)
    print(f"  target            : {state.target_vault_apy*100:.2f}%")
    print(f"  median_pair_apy   : {state.median_pair_apy*100:.2f}%")
    print(f"  leverage L        : {state.leverage}")
    print(f"  idle alpha        : {state.idle_fraction*100:.1f}%")
    print(f"  m_pos (per leg)   : {state.position_aum_cap*100:.2f}%")
    print(f"  m_counter         : {state.counter_venue_cap*100:.1f}%")
    print(f"  r_min             : {state.net_apy_floor*100:.2f}%")
    print(f"  N_active          : {state.n_active_candidates}")
    print(f"  N_counter_active  : {state.n_counter_venues_active}")
    warn, halve, kill = state.pnl_breakers
    print(f"  PnL breakers      : warn={warn*100:.2f}%  halve={halve*100:.2f}%  kill={kill*100:.2f}%")

    trades = []
    for c in state.candidates_summary:
        d = evaluate_trade_live(c['symbol'], c['counter_venue'], inputs, m, state)
        if d.should_enter:
            trades.append(d)
            print(f"  ENTER  {d.symbol:<10}/{d.counter_venue:<12} "
                  f"$={d.notional_per_leg_usd:>10,.0f}  μ={d.expected_funding_apy*100:>+6.2f}%  "
                  f"NET={d.projected_net_apy*100:>6.2f}%  ratio={d.income_cost_ratio:.2f}  "
                  f"H_min={d.min_hold_h:>4.0f}h  d_max={d.drawdown_stop_pct*100:.2f}%")
        else:
            print(f"  REJECT {c['symbol']:<10}/{c['counter_venue']:<12} {d.reason}")

    if trades:
        pair_capital = sum(2 * d.notional_per_leg_usd / d.leverage for d in trades)
        weighted_net = sum(d.projected_net_apy * (2*d.notional_per_leg_usd/d.leverage) for d in trades) / pair_capital
        deployed = pair_capital / inputs.aum_usd
        idle = 1 - deployed
        # v3.5.1 fix: trading apy on AUM = (margin/AUM) × (L/2) × apy_on_notional
        # Previously this was missing the (L/2) factor — silent at L=2 (factor=1) but
        # under-counts income at L=3+ (factor=1.5+). Use weighted-average leverage.
        avg_leverage = sum(d.leverage * (2*d.notional_per_leg_usd/d.leverage) for d in trades) / pair_capital
        vault_proj = idle * inputs.r_idle + deployed * (avg_leverage / 2) * weighted_net
        cust = vault_proj * m.cut_customer
        buf = vault_proj * m.cut_buffer
        res = vault_proj * m.cut_reserve
        cust_ok = m.customer_apy_min <= cust <= m.customer_apy_max
        buf_ok = m.buffer_apy_min <= buf <= m.buffer_apy_max
        print()
        print(f"  pair_capital      : ${pair_capital:,.0f}  ({deployed*100:.1f}% AUM deployed)")
        print(f"  weighted trade APY: {weighted_net*100:.2f}%")
        print(f"  PROJECTED VAULT   : {vault_proj*100:.2f}% gross")
        print(f"    customer (×{m.cut_customer:.2f}): {cust*100:.2f}%   {'OK' if cust_ok else 'MISS (mandate '+f'{m.customer_apy_min*100:.0f}-{m.customer_apy_max*100:.0f}%'+')'}")
        print(f"    buffer (×{m.cut_buffer:.2f})  : {buf*100:.2f}%   {'OK' if buf_ok else 'MISS (mandate '+f'{m.buffer_apy_min*100:.0f}-{m.buffer_apy_max*100:.0f}%'+')'}")
        print(f"    reserve (×{m.cut_reserve:.2f}) : {res*100:.2f}%")
        return cust_ok and buf_ok
    else:
        print("\n  no trades entered — vault stays in idle bucket only")
        idle_only = inputs.r_idle
        print(f"  vault gross from idle alone: {idle_only*100:.2f}%")
        return False


def scenario_a_thin_universe(m: Mandate) -> LiveInputs:
    """Real April 14 2026 snapshot values — thin OI on most actionable symbols."""
    random.seed(42)
    return LiveInputs(
        timestamp_ms=1776146400000, aum_usd=1_000_000.0, r_idle=0.044,
        funding_rate_h={
            ("ARB", "pacifica"): 0.000015, ("ARB", "backpack"): -0.000033,
            ("BTC", "pacifica"): -0.0000117, ("BTC", "backpack"): -0.000009, ("BTC", "hyperliquid"): -0.000010,
            ("FARTCOIN", "pacifica"): 0.000015, ("FARTCOIN", "backpack"): -0.000040,
            ("WLD", "pacifica"): 0.000015, ("WLD", "hyperliquid"): 0.000078,
        },
        open_interest_usd={
            ("ARB", "pacifica"): 145_000, ("ARB", "backpack"): 800_000,
            ("BTC", "pacifica"): 33_000_000, ("BTC", "backpack"): 5_000_000, ("BTC", "hyperliquid"): 50_000_000,
            ("FARTCOIN", "pacifica"): 350_000, ("FARTCOIN", "backpack"): 800_000,
            ("WLD", "pacifica"): 100_000, ("WLD", "hyperliquid"): 1_500_000,
        },
        volume_24h_usd={
            ("ARB", "pacifica"): 5_000, ("ARB", "backpack"): 1_500_000,
            ("BTC", "pacifica"): 518_000_000, ("BTC", "backpack"): 80_000_000, ("BTC", "hyperliquid"): 700_000_000,
            ("FARTCOIN", "pacifica"): 155_000, ("FARTCOIN", "backpack"): 600_000,
            ("WLD", "pacifica"): 1_032_000, ("WLD", "hyperliquid"): 5_500_000,
        },
        fee_maker={"pacifica": 0.00015, "backpack": 0.00020, "hyperliquid": 0.00025, "lighter": 0.00020},
        fee_taker={"pacifica": 0.00040, "backpack": 0.00050, "hyperliquid": 0.00050, "lighter": 0.00050},
        bridge_fee_round_trip={
            ("pacifica", "backpack"): 0.0,
            ("pacifica", "hyperliquid"): 0.0010,
            ("pacifica", "lighter"): 0.0015,
        },
        funding_history_h={
            ("ARB", "pacifica"): synth_history(0.000010, 0.0000020),
            ("ARB", "backpack"): synth_history(-0.000022, 0.0000020),
            ("BTC", "pacifica"): synth_history(-0.0000005, 0.0000050),
            ("BTC", "backpack"): synth_history(0.0000005, 0.0000050),
            ("BTC", "hyperliquid"): synth_history(0.0000003, 0.0000050),
            ("FARTCOIN", "pacifica"): synth_history(0.000013, 0.0000025),
            ("FARTCOIN", "backpack"): synth_history(-0.000017, 0.0000025),
            ("WLD", "pacifica"): synth_history(0.000010, 0.0000030),
            ("WLD", "hyperliquid"): synth_history(0.000050, 0.0000030),
        },
        basis_divergence_history={
            "ARB": [(0, 0.0010 + random.gauss(0, 0.0005)) for _ in range(168)],
            "BTC": [(0, 0.0005 + random.gauss(0, 0.0002)) for _ in range(168)],
            "FARTCOIN": [(0, 0.0012 + random.gauss(0, 0.0006)) for _ in range(168)],
            "WLD": [(0, 0.0008 + random.gauss(0, 0.0004)) for _ in range(168)],
        },
        vault_daily_returns=[],
    )


def scenario_b_rich_universe(m: Mandate) -> LiveInputs:
    """12 hypothetical symbols with deep OI ($2-10M each), median ~22% APY signal."""
    random.seed(7)
    symbols_config = [
        ('SYM01', 0.000010, -0.000022, 0.0000015),
        ('SYM02', 0.000005, -0.000020, 0.0000018),
        ('SYM03', 0.000012, -0.000025, 0.0000020),
        ('SYM04', 0.000008, -0.000018, 0.0000017),
        ('SYM05', -0.000005, 0.000018, 0.0000016),
        ('SYM06', 0.000015, -0.000010, 0.0000022),
        ('SYM07', 0.000010, -0.000023, 0.0000019),
        ('SYM08', 0.000012, -0.000012, 0.0000018),
        ('SYM09', -0.000003, 0.000022, 0.0000020),
        ('SYM10', 0.000007, -0.000019, 0.0000017),
        ('SYM11', 0.000010, -0.000015, 0.0000019),
        ('SYM12', 0.000013, -0.000017, 0.0000020),
    ]
    funding_rate_h, funding_history_h, oi, vol, basis_div = {}, {}, {}, {}, {}
    counters = ['backpack', 'hyperliquid']
    for sym, mu_p, mu_c, std in symbols_config:
        cnt = random.choice(counters)
        funding_rate_h[(sym, 'pacifica')] = mu_p
        funding_rate_h[(sym, cnt)] = mu_c
        funding_history_h[(sym, 'pacifica')] = synth_history(mu_p, std)
        funding_history_h[(sym, cnt)] = synth_history(mu_c, std)
        oi[(sym, 'pacifica')] = random.uniform(2_000_000, 6_000_000)
        oi[(sym, cnt)] = random.uniform(3_000_000, 10_000_000)
        vol[(sym, 'pacifica')] = oi[(sym, 'pacifica')] * random.uniform(0.5, 1.5)
        vol[(sym, cnt)] = oi[(sym, cnt)] * random.uniform(0.5, 1.5)
        basis_div[sym] = [(0, 0.0010 + random.gauss(0, 0.0005)) for _ in range(168)]
    return LiveInputs(
        timestamp_ms=1776146400000, aum_usd=1_000_000.0, r_idle=0.044,
        funding_rate_h=funding_rate_h, open_interest_usd=oi, volume_24h_usd=vol,
        fee_maker={"pacifica": 0.00015, "backpack": 0.00020, "hyperliquid": 0.00025, "lighter": 0.00020},
        fee_taker={"pacifica": 0.00040, "backpack": 0.00050, "hyperliquid": 0.00050, "lighter": 0.00050},
        bridge_fee_round_trip={
            ("pacifica", "backpack"): 0.0,
            ("pacifica", "hyperliquid"): 0.0010,
            ("pacifica", "lighter"): 0.0015,
        },
        funding_history_h=funding_history_h,
        basis_divergence_history=basis_div,
        vault_daily_returns=[],
    )


def main():
    m = Mandate()
    print("=== Mandate-derived constants (no live inputs) ===")
    print(f"  target_vault_apy : {target_vault_apy(m)*100:.2f}%")
    print(f"  Persistence threshold by lookback:")
    for T in (168, 336, 504, 720):
        print(f"    T={T:>3}h: p_min = {persistence_threshold(T, m)*100:.2f}%")
    print(f"  Required ratio by SNR:")
    for snr in (10, 5, 3.33, 2.5, 2):
        rho = required_ratio(snr, m)
        print(f"    SNR={snr:>5.2f}: rho = {rho:.3f}" if rho < 100 else f"    SNR={snr:>5.2f}: REJECT")

    a = scenario_a_thin_universe(m)
    b = scenario_b_rich_universe(m)

    a_ok = run_scenario("THIN UNIVERSE (real April 14 thin-OI symbols)", m, a)
    b_ok = run_scenario("RICH UNIVERSE (12 deep-OI synthetic symbols)", m, b)

    print()
    print('=' * 100)
    print('VALIDATION SUMMARY')
    print('=' * 100)
    print(f"  scenario A (thin universe)  → mandate hit: {'YES' if a_ok else 'NO (expected — universe too thin to deploy)'}")
    print(f"  scenario B (rich universe)  → mandate hit: {'YES' if b_ok else 'NO (formula bug — investigate)'}")
    print()
    if not b_ok:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
