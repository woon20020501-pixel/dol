# Architecture

This page is for the engineers, auditors, and journalists who want to know what's actually under the hood. The marketing pitch is "a dollar that grows itself" — that hides a lot of work, and the work is worth showing.

If you don't care about the engineering, skip to [On-chain & verified](/docs/trust/on-chain) for the addresses you can check yourself, or [Risks](/docs/trust/risks) for the plain-English risk list.

---

## The pitch in one paragraph

Dol's growth comes from a delta-neutral funding-rate strategy that runs against multiple perpetuals venues, with a vault contract on Base that mints and redeems Dol tokens 1:1 against USDC. The strategy itself is built as a pure-math layer in Rust with newtype-driven type safety, parity-tested against an independent Python reference down to six decimal places. The runtime sits on top as a `tokio` async layer that handles venue connectivity, order routing, and atomic close orchestration. Everything that touches user funds is enforced by on-chain contracts, not by the bot.

That's the whole stack. The rest of this page expands each layer with the sources you can verify.

---

## Layer 1: The strategy is real math

Most "yield" products advertise an APY without explaining where the number comes from. Dol's number comes from a published mathematical framework. It has names you can look up:

- **OU mean-reversion model** for funding-rate spread dynamics. The expected time-averaged spread over a planning horizon `τ` is `D̄(τ; D₀) = μ̃ + (D₀ − μ̃) · φ(θ^OU · τ)`, where `φ(x) = (1 − e^(-x)) / x` is the standard absorption factor.
- **Orderbook impact correction** on top, because a position large enough to move the funding price needs to discount the spread it sees. The framework uses an Impact Notional sourced from each venue's own published rules and updates as those rules change.
- **Bernstein robust leverage bound** instead of the usual gut-feel cap. Maximum leverage is computed from the maintenance margin requirement, the move size at a given confidence level `ε`, and the time horizon, via the Bernstein concentration inequality. This bounds the probability of liquidation under a worst-case path, not just the expected one.
- **MFG free-entry equilibrium** for capacity. Every delta-neutral funding strategy has a finite economic capacity because funding rates are competitive. The framework treats the competition density `ρ^comp` as an explicit parameter and lets the optimal trade size fall out of the free-entry condition.
- **Mandate cap routing** for the protocol's customer / buffer / reserve allocation. The split is enforced by code, not by a discretionary policy.
- **Model C execution costs** for the round-trip cost a position has to clear before it's worth opening — the break-even hold `τ^BE = 8760 · c · (1 + ρ^comp) / D^eff`.

If you want to check the math, the paper-style writeups live next to the implementation in the strategy repo under `docs/math-*.md`. Each function in the Rust crate references the equation number from that paper in its doc comment.

---

## Layer 2: Pure-math Rust with newtype safety

The strategy is implemented in Rust, not Python or TypeScript, for two reasons: deterministic numerics, and a type system that catches unit-mismatch bugs at compile time.

The **newtype pattern** wraps every numeric quantity in a struct that names its semantic unit:

```rust
pub struct AnnualizedRate(pub f64);  // 1/year
pub struct HourlyRate(pub f64);      // 1/hour
pub struct Hours(pub f64);
pub struct Usd(pub f64);
pub struct AumFraction(pub f64);     // [0, 1]
pub struct Dimensionless(pub f64);

impl AnnualizedRate {
    pub fn from_hourly(h: HourlyRate) -> Self { Self(h.0 * 8760.0) }
    pub fn to_hourly(&self) -> HourlyRate { HourlyRate(self.0 / 8760.0) }
}
```

The Rust compiler refuses to pass an `HourlyRate` where an `AnnualizedRate` is expected, even though both wrap the same `f64`. Conversion is explicit. The class of "I forgot to multiply by 8760" bug that costs every quant team money is closed at compile time.

The codebase is split into two layers with a hard rule: **pure math has no I/O and no state**.

```
crates/
├── bot-types         primitive newtypes
├── bot-math          pure functions, no async, no I/O
├── bot-strategy      framework layer (rigorous, cost, portfolio)
├── bot-venues        exchange adapters
├── bot-execution     order routing, atomic close
├── bot-state         position tracking, persistence
├── bot-runtime       main loop, signal generation
└── bot-tests         parity fixtures
```

`bot-math` and `bot-strategy` import zero async crates. They cannot reach the network. They cannot read state. They take their inputs as function arguments and return their outputs by value. This is what makes the parity test (next section) possible.

---

## Layer 3: Determinism and parity testing

The strategy was prototyped in Python first and exists as a reference implementation. The Rust version is required to match the Python output to **six decimal places** on every fixture. The parity test is the build gate: a Rust commit that breaks parity fails CI.

The rules that enable bit-for-bit reproducibility:

- `-ffast-math` is forbidden (it breaks IEEE 754 associativity)
- `f64::mul_add` (FMA) is forbidden — Python does not use FMA, and a single fused multiply-add silently introduces a rounding difference
- The order of floating-point operations is preserved verbatim from the Python reference. `(a + b) + c` and `a + (b + c)` are different float values; we don't get to pick.
- `rayon` parallelism is forbidden in the pure layer because work-stealing scheduling is non-deterministic
- All RNG paths take an explicit seed; `rand::thread_rng()` is forbidden

The fixture format is a JSON file that records the strategy's inputs at a real point in time and the Python reference's outputs. The Rust test loads the inputs, runs the same function, and asserts the outputs match within `1e-6`. There are fixtures for every primitive (`φ`, OU spread, break-even hold, optimal margin fraction, Bernstein leverage, MFG equilibrium, cap routing) and one end-to-end fixture that runs an entire decision tick.

---

## Layer 4: Async runtime and atomic close

The runtime layer sits on top of the pure layer and handles the messy real world: venue connectivity over WebSocket, signed orders, fills, partial fills, retries, and the **atomic close orchestration** that turns "close this delta-neutral pair" from a sequence of two single-leg requests into something the borrow checker can reason about.

A delta-neutral position is two legs on two venues. Closing it requires two near-simultaneous orders, and a partial fill or an outright failure on one leg leaves the position one-legged — a directional bet the strategy never authorized. The runtime uses Rust's borrow checker plus an explicit state machine to make sure that's impossible: the position transitions through `Open → Closing → Closed` states with type-level guarantees that an in-flight close cannot be observed as still-open by another task.

This is the kind of bug class that has cost real DeFi protocols real money. Avoiding it is not optional.

---

## Layer 5: Invariants enforced at runtime

Even with the type system doing the heavy lifting, several conditions are checked on every decision tick via `debug_assert!` in development and explicit `Result::Err` in production:

| Invariant | What it guarantees |
|---|---|
| **Mandate allocation conservation** | The customer, buffer, and reserve splits sum exactly to the vault gross. No silent rounding loss. |
| **Two-legged positions** | Every active position has matching pivot and counter notional. A one-legged position is an error, not a state. |
| **Budget constraint** | `α + Σ wᵢ ≤ 1`. Active margin plus idle buffer cannot exceed AUM. |
| **Leverage bound** | Per-position leverage cannot exceed the Bernstein-derived robust bound for that position's parameters. |
| **Venue concentration** | No single venue holds more than the configured maximum fraction of AUM. |
| **PnL kill switch** | A daily loss beyond the configured Z-multiple of historical std triggers a full position-close and PM alert. |

If any of these fire, the runtime stops accepting new positions and notifies the operator. They are not soft warnings.

---

## Layer 6: On-chain settlement

None of the above runs on-chain. The Rust bot is an off-chain decision-maker; it generates signed transactions and submits them to the Dol vault contract on Base. The contract is the source of truth for who owns what. If the bot disappears tomorrow, the contract still allows every Dol holder to redeem their position by calling `redeem` directly from their wallet.

The contract surface is intentionally small:

- `deposit(usdc) → mint Dol`
- `redeem(shares) → schedule withdraw`
- `claimRedeem(requestId) → payout`
- `instantRedeem(shares) → buffer-served payout (with a small fee)`
- `pricePerShare() → on-chain fair value`

For the actual deployed addresses, source code, and verified Basescan links, see [On-chain & verified](/docs/trust/on-chain).

---

## What this buys the user

A user who deposits into Dol is trusting four things, in this order:

1. **The on-chain contract** — verifiable on Basescan, source published, audit pending
2. **The strategy math** — published, peer-reviewable, parity-tested between two independent implementations
3. **The runtime correctness** — type-driven, borrow-checked, invariant-asserted
4. **The operator's day-to-day judgment** — the smallest of the four, intentionally

Most "yield" products require the user to trust the operator first. Dol inverts that: the math is published, the implementation is published, the on-chain contract is the final authority, and the operator's discretion is bounded by code.

The cost of doing it this way is that the team is small, the testnet phase is long, and the published TVL ceiling is conservative. We think that's the right trade-off for a product asking grandparents to put real money in.

---

## Source links

- **Strategy paper and math derivations** — see the `docs/` folder of the strategy repository
- **Rust implementation** — see the `bot-rs/` workspace in the strategy repository
- **Python reference implementation** — see the `strategy/` folder of the strategy repository (the parity oracle)
- **On-chain contracts** — see [On-chain & verified](/docs/trust/on-chain)
- **What can go wrong** — see [Risks](/docs/trust/risks) for the categories and [Framework assumptions](/docs/trust/framework-assumptions) for the strategy-level assumptions and stress bounds

---

*This page is a high-level overview written for external readers. The implementation design documents in the strategy repo are the source of truth and may be more current. Last updated 2026-04-15.*
