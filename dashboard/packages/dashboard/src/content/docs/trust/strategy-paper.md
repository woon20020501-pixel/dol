# Strategy paper (summary)

This page is the publishable distillation of Dol's internal Aurora-Ω engineering whitepaper. The full version lives in the strategy repository and is the source of truth that the Rust runtime is built against; this is the ~30% slice that's appropriate for external readers — engineers, auditors, journalists, large depositors, and academic reviewers — to verify that the math under the marketing pitch is real.

If you're a regular user, you don't need this page. Read [How it works](/docs/how-it-works) instead.

If you're an engineer, also read [Architecture](/docs/trust/architecture) for the implementation side: type system, parity testing, atomic close orchestration, on-chain settlement.

---

## 1. The engine in one sentence

Dol's strategy is a **funding-capture engine**: it holds a delta-neutral position across two perpetuals venues for one funding window at a time, places passive maker quotes on the primary venue, and immediately taker-hedges any maker fill on a secondary venue chosen by a UCB bandit. The funding spread is the main source of return; everything else — maker rebate, microstructure alpha — is a small additive layer on top.

This is **not** a "trade every second, pivot on every signal" bot. The funding leg is locked for one full hour. Only the execution leg moves quickly.

---

## 2. The five-label honesty rule

Every claim in the internal whitepaper carries one of five labels. We think this is the most important credibility decision in the document, so we publish the rule here.

| Label | Meaning |
|---|---|
| **[THEOREM]** | A result we can prove under stated model assumptions. |
| **[LEMMA]** | A supporting result — a bound, convergence proof, or auxiliary inequality. |
| **[TARGET]** | An operational goal we measure against. Not a proof; a benchmark. |
| **[HEURISTIC]** | A rule of thumb backed by intuition or empirical experience. |
| **[CALIBRATION]** | A number that has to be estimated from data, not derived. |

Most "DeFi yield" papers blur all five into the same prose voice. That's how a calibrated number ends up sounding like a theorem and a heuristic ends up sounding like a guarantee. The Aurora-Ω whitepaper labels every single claim. When you see a number on this page, you can tell which label it carries.

---

## 3. The economic structure

### 3.1 Two layers of position

Every position in Dol's strategy lives at one of two timescales:

**Funding leg.** Locked for one full funding window (1 hour on Pacifica). Direction and notional are fixed at the start of the window and held until the next funding tick. Purpose: receive the funding spread.

**Execution leg.** Only moves when a passive maker quote on the primary venue gets filled. When it does, the same size is immediately hedged via taker order on the chosen secondary venue. Purpose: remove price exposure on the freshly opened maker fill, plus capture a small microstructure edge from the maker rebate and the cross-venue spread.

**The key idea**: the bot doesn't reverse direction every second. Funding leg = lock. Execution leg = react.

### 3.2 Funding cycle lock

Define the funding cycle index `c(t) = ⌊t / 3600⌋`. For every cycle `c`, the bot picks one hedge direction `h_c` and one target notional `N_c` and holds them constant across the entire interval `[c·3600, (c+1)·3600)`. The cycle lock is not just an implementation rule — it's the mathematical definition of "this is a funding-capture bot." Without the lock, the position becomes a generic market-maker and the strategy's main source of return disappears.

### 3.3 Venue selection via funding bandit

For a hedge venue candidate set (Hyperliquid, Lighter, Backpack, etc.), the bot computes for each venue `k`:

```
Π_k = expected funding receive_k − expected spread/slippage_k
```

and selects the venue maximizing `Π_k`. The selector is **not** a fixed rule; it's a **UCB (upper confidence bound) bandit** that tracks each venue's realized payoff and updates the estimate online. The bandit's regret bound is documented in the internal whitepaper as a separate lemma.

---

## 4. The three main theorems

These three are the load-bearing results. Every other lemma in the whitepaper exists to support one of these.

### [THEOREM A] Capacity invariance

**Plain English**: in the regime where market microstructure is the binding constraint (rather than the bot's capital), adding more AUM does not let you proportionally add more positions. The optimal number of active hedge layers is set by what the orderbook can absorb, not by how much capital you have to deploy.

**Why it matters**: this is what bounds Dol's capacity ceiling. We can't grow TVL forever and expect linear yield. There is a real, computed ceiling, and growing past it dilutes the per-Dol return. The Phase 1 cap published in [Framework assumptions](/docs/trust/framework-assumptions) is Dol's own conservative early-stage choice, sized inside the headroom the theorem gives us.

### [THEOREM B] LL-irrelevance

**Plain English**: in the same capacity-binding regime, the **liquidation lag** — the time between a forced close decision and the close actually clearing — is not a first-order driver of optimal policy. Depth, latency, toxicity, and funding spread dominate it.

**Why it matters**: most DeFi vault designs obsess over liquidation timing. The Aurora-Ω framework proves that under the regime we operate in, that's the wrong thing to optimize for. The bot puts its decision budget into the four variables that actually move the result.

### [THEOREM C] Free-entry ceiling

**Plain English**: in a free-entry equilibrium — meaning new operators can join when profits exceed costs — long-run excess value at any individual venue equals the marginal operating cost. Stated formally: `V* = C_op`. No venue can structurally pay more than its operating cost in the long run, because new entrants will compete the excess away.

**Why it matters**: this is the upper bound on what any single venue can pay. It's also the theoretical justification for why Dol routes across multiple venues instead of betting on one. Diversification across venues is not a marketing slogan here — it's a direct consequence of the free-entry ceiling.

> These three are the **only** results in the internal whitepaper that carry the [THEOREM] label and are claimed as load-bearing. Everything else is either a [LEMMA] supporting one of them, or a [TARGET] / [HEURISTIC] / [CALIBRATION] in the operational layer.

---

## 5. APY decomposition

The headline yield number breaks down into three positive sources and three cost terms:

```
APY ≈
   funding spread yield      [main]
 + maker rebate              [secondary]
 + microstructure alpha      [small positive]
 −
   slippage                  [cost]
 + latency / timing          [cost]
 + fallback tail             [cost]
```

The funding spread is the engine. Maker rebate and microstructure alpha are small positives that exist because the execution leg uses passive maker quotes instead of pure taker. The three cost terms are ineliminable: every order pays slippage, every cross-venue hedge has timing risk, and any fallback path that isn't the cheapest venue costs more.

**The honest version of the pitch**: the difference between a good funding-capture engine and a bad one is whether the cost terms eat the positive ones. The whole reason for the depth-aware allocation, the toxicity filter, the fair-value oracle, and the fallback router is to keep each cost term bounded under conditions we can predict.

---

## 6. Risk budget (with caveat)

The internal whitepaper sets a **provisional** risk budget:

```
CVaR_99 ≤ provisional risk budget
```

Concretely the seed value is set in the strategy repo's calibration file. **This is labelled [CALIBRATION], not [THEOREM]**. It's a budget the operator measures against. The actual constraint at runtime is that the rolling empirical CVaR must stay below the budget — if the strategy starts paying out tail losses larger than the budget allows, the FSM (fail-safe controller) reduces position sizes and notifies the operator before the next decision tick.

The risk stack itself has four layers, each handling a different statistical property of the loss distribution:

1. **Entropic certainty equivalent** — converts the moment-generating function of the loss into a Kelly-compatible utility number.
2. **ECV** — combines CVaR at 99% with a standard-deviation regularizer for dimensionally-consistent tail measurement.
3. **Execution χ²** — flags execution distributions that drift from their expected shape.
4. **RL critic uncertainty** — when the policy network's value function disagrees with itself across an ensemble, the robust mode kicks in and the bot trades smaller.

Each of these layers fires independently. The fail-safe controller activates **when at least two of the four are red**, not on any single signal. That two-of-four rule is itself a [HEURISTIC] — empirical, not derived.

---

## 7. What was removed (the honest section)

Earlier drafts of the framework contained ideas that were later proved wrong, dimensionally inconsistent, or overclaimed. The published whitepaper documents what was demoted or deleted — this is the section that, if you're reviewing the project, you should read most carefully because it shows what the team was willing to be wrong about.

| Earlier claim | Final disposition |
|---|---|
| Expected-Shortfall–based IES → Kelly link | **Deleted.** The dimensional argument didn't hold. |
| ECV defined as `CVaR + κ · Var` | **Replaced** with `CVaR + κ · Std`. Var is the wrong dimension for direct addition to a CVaR. |
| A specific latency-dimension formula | **Deleted.** The original derivation conflated execution impact and timing risk. |
| Execution impact and timing risk treated as one term | **Split.** Now `q²/D` for execution impact and `qσ√τ` for timing risk are two independent terms. |
| "IOC ≥ 65%" presented as a theorem | **Demoted to [TARGET].** It's an operational benchmark, not a proven result. |
| "Staleness theorem" | **Demoted to a remark.** The earlier version proved a triviality; the remark version is honest about what's actually known. |
| MFG existence + uniqueness as main theorem | **Demoted to a literature-backed weak-equilibrium note.** The strong claim was overreach. |

If you're an academic reviewer or auditor, this table is the part that signals seriousness. The whitepaper is willing to say "we were wrong about this" in writing.

---

## 8. Python vs Rust split

The framework is implemented with a hard separation between offline research and live runtime:

**Python** is the artifact generator. It runs:
- Gradient-boosted-tree training for the toxicity filter
- Fractal slope OLS calibration
- Alpha-cascade baseline generation
- Offline backtesting and validation

Python never touches a live order. Its outputs are JSON / parquet files that the Rust runtime loads at startup as immutable calibration artifacts.

**Rust** is the runtime engine. It runs:
- Live websocket handling on each venue
- Funding-cycle lock state machine
- Quote placement and cancellation
- IOC hedge and fallback routing
- Risk stack enforcement
- Telemetry emission

The Python implementation is also the **parity oracle** for the Rust implementation: every pure math primitive in the Rust crate has to match the Python reference to six decimal places on every fixture, enforced as a build gate. See [Architecture](/docs/trust/architecture) for the full parity-test discipline.

---

## 9. The honest classification

Here is the complete labelling for the framework's load-bearing claims. This is the table an auditor would want to see and a marketing site would never publish.

### [THEOREM]
- Capacity invariance
- LL-irrelevance
- Free-entry ceiling
- Fractal slope OLS consistency
- Adverse-loss high-probability bound
- Entropic CE small-η expansion
- Contraction-mapping self-correction

### [CALIBRATION]
- The depth threshold `D_min` (seed value, refined per venue)
- The toxicity score β-vector (initial empirical seed, retrained)
- Stale time constant `τ_stale`
- Fallback mixture parameters
- The `(γ, λ)` constants in the offset-control rule
- `κ` in ECV
- All RL hyperparameters
- IOC success target

### [HEURISTIC] / operational target
- IOC fill rate ≥ 65%
- The 4-step retry schedule
- Two-minute flatten timer
- The minimum hedge size rule
- Red-flag thresholds in the FSM

A reader's mental shortcut: if you see an exact number quoted in this paper, ask which label it carries. If it carries [THEOREM], it's derivable. If it carries [CALIBRATION], it has to be measured. If it carries [HEURISTIC], we picked it because it works in the conditions we've seen so far.

---

## 10. The one-line definition

The Aurora-Ω framework is, formally:

> A funding-centric hybrid quant engine that holds a 1-hour funding-cycle lock, places passive maker quotes on Pacifica, immediately taker-hedges any fill on the best available secondary venue selected by a UCB bandit, and integrates depth-aware allocation, fractal liquidity correction, a toxicity filter, partial-fill Bayesian update, a unified fair-value oracle, latency-aware execution penalty, a four-layer risk stack, a fail-safe controller, and an RL execution policy to bound cross-venue execution risk.

That's the whole thing in one sentence. Every clause in that sentence corresponds to a labelled section in the internal whitepaper.

---

## What's in the full whitepaper that isn't here

This page is roughly 30% of the internal document. Things deliberately not summarized:

- The exact calibration values (these are competitive)
- The specific β-vectors for the toxicity scorer
- The 20-module Rust runtime layout (already covered in [Architecture](/docs/trust/architecture))
- The persistence + observability + failure catalog plumbing
- The full alpha-cascade forecast scoring guard
- The test strategy and offline artifact builders
- The internal lineage (Aurora-1 → 2 → 3 → Ω)

Anything in this page is true. Anything missing from this page is either operationally sensitive, internally specific, or already documented on the [Architecture](/docs/trust/architecture) page.

---

## See also

- **[How it works](/docs/how-it-works)** — the same product explained in plain English without the math
- **[Architecture](/docs/trust/architecture)** — the engineering side: type system, parity testing, runtime invariants
- **[Framework assumptions](/docs/trust/framework-assumptions)** — the three high-level assumptions baked into Dol's strategy, with stress-scenario bounds
- **[On-chain & verified](/docs/trust/on-chain)** — contract addresses you can verify yourself
- **[Risks](/docs/trust/risks)** — plain-English risk categories

---

*This page is a published distillation of the internal Aurora-Ω engineering whitepaper. The full version is the source of truth and may be ahead of this summary at any given moment. Last updated 2026-04-15.*
