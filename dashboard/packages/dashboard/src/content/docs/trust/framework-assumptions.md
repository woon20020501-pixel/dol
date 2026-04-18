<!--
  IMPORTANT — pre-mainnet messaging note (2026-04-15)

  This page was rewritten to remove venue-specific weakness framing
  during the pre-launch / hackathon period. The earlier draft
  publicly called out Pacifica's Closed Beta withdrawal cap, the
  TVL→drain time table (up to 50 days for $5M), the specific
  Impact Notional values ($20k BTC / $6k other), and the ADL
  cascade scenario as Dol's primary failure modes. Those facts
  are technically true but framed as Pacifica being the
  bottleneck — bad for partnership messaging and for hackathon
  perception.

  The substance is preserved: the $100k Phase 1 cap, the
  stress scenarios, the trust-building thesis. Only the
  Pacifica-specific framing was softened to "Dol's own
  conservative early-stage choices."

  AT MAINNET LAUNCH: expand this page back to the fuller version
  with concrete venue-specific numbers. By then partnership
  conversations are settled and the explicit version is more
  appropriate. The earlier draft is preserved in git history
  (commit `9cf75aa` `[C/assumptions]`).
-->

# Framework assumptions

Most DeFi "yield" products publish a headline number like "12%" and nothing else. That's dangerous. The number depends on a set of assumptions — about market conditions, about the strategy's capacity, about what can break — and if any one of them shifts, the number doesn't hold.

This page lists the three biggest assumptions baked into Dol's strategy. You should know these before you put meaningful money in.

This is not a legal disclaimer. It's an honest account of what has to be true for Dol to work as advertised.

---

## Assumption 1: Cash out is not always instant

Dol offers two cash-out paths. **Instant** is served from a local USDC buffer inside the vault — fast, but limited to roughly the buffer size at any given moment. **Scheduled** is a 30-minute queued path designed for any amount up to your full balance.

**Why two paths exist.** Every delta-neutral strategy has to leave room between "money on standby" and "money working in the strategy." Money on standby costs nothing to return — that's where Instant pulls from. Money working in the strategy has to be unwound before it can come back, and unwinding takes time.

**What this means in practice.** For everyday small-to-medium cash outs, Instant works fine. For larger withdrawals or when many people cash out at once, the Instant buffer can drain — at which point the frontend will gently route you to Scheduled, which always works regardless of buffer state. Scheduled is the load-bearing path; Instant is the convenience path.

The split between these two is not just a UX choice. It's the practical answer to a real constraint that every funding-strategy vault has to solve.

---

## Assumption 2: Delta-neutral doesn't mean loss-free

Dol's strategy runs a delta-neutral position: long and short paired across two perpetuals venues so the directional exposure cancels out. In theory, your return comes from the funding-rate spread and not from market moves. In practice, several things can erode that.

**Where the loss can come from**, in plain English:

- **Mark-price drift.** The two venues use slightly different reference prices. In a stressed hour, a 1–2% drift between them can produce immediate unrealized loss on the delta-neutral book even though neither leg is technically wrong.
- **Funding rate flip.** The spread we capture can invert. When it does, the leg that was paying us starts charging us until the operator rebalances.
- **Forced-exit cascade.** In an extreme market event, an exchange's risk engine may close positions at unfavorable prices to protect the broader market. Our hedge legs would be exposed if this happens during high volatility.
- **Oracle gap.** Price feeds sample at fixed intervals. In the gap between samples, price can move beyond the funding clamp, and the clamp does not protect against forced moves on the underlying.

**Realistic stress-scenario loss bound:**

| Scenario | 24h drawdown estimate |
|---|---|
| Normal market, small funding swing | under 0.1% |
| Elevated volatility, ~1% mark-price drift | 1–3% |
| Stressed market, partial position adjustment | 5–10% |
| Extreme tail event | 10%+, realized immediately |

These ranges are estimates based on general perpetuals-market mechanics. **The actual stress loss in a real black-swan event has not been measured** because Dol is in Phase 1 and the strategy hasn't seen one yet. We will not hide that from you when it happens.

---

## Assumption 3: There is a real capacity ceiling

Every delta-neutral funding strategy has a finite capacity. Two things bound Dol's:

**The mathematical ceiling.** Funding rates compete down as more capital chases the same spread. A position large enough to materially move the funding price discounts the spread it sees. This is a property of the strategy itself, not of any particular venue, and it's why Dol's optimal position size has a closed-form ceiling rather than scaling linearly with TVL. The math is documented in [Strategy paper](/docs/trust/strategy-paper) under the Capacity invariance theorem.

**Dol's Phase 1 self-imposed cap.** On top of the mathematical ceiling, Phase 1 carries a deliberately conservative **$100,000 hard TVL cap** — chosen by the Dol team, not derived from any external limit. The reason is that Phase 1 is the period where the strategy's real-world behavior is being measured for the first time. Capping TVL while we collect that data means a stress event hits with a small enough exposure to be a learning moment, not a wipeout.

**What this means for you.** When Dol's TVL approaches $100,000, the deposit page surfaces a clear "capacity reached" state and refuses new deposits until the level drops or the cap is raised. This is enforced both at the documentation level and at the actual `/deposit` UI — try to deposit beyond the remaining headroom and the action is gated. The cap will be raised in subsequent phases as the strategy proves out.

---

## Why we publish this

Two reasons.

**First — fairness to you.** You are deciding whether to put money into a product. You deserve to know the actual constraints that shape the headline number, not just the headline number. If your plan is "deposit $50k and pull it out next week," the cash-out paths matter. If your plan is "set and forget for a year," the capacity ceiling matters more. Different people need different facts.

**Second — durable liquidity.** The people who read this page and still deposit are the people who understood what they were buying. They do not panic in a stress event because the stress event was in the documentation before they funded. Yield farmers who saw only a headline rate are the worst kind of depositor — they add to TVL in good times and drain it in one block the moment something goes wrong. We would rather grow slower and keep the users who are here on purpose.

---

## Where to verify this yourself

- **Dol contract addresses on Basescan** — see [On-chain & verified](/docs/trust/on-chain). The TVL cap is enforced live in the deposit UI; you can also query `totalSupply()` directly.
- **The math behind the capacity ceiling** — see [Strategy paper](/docs/trust/strategy-paper) §3 (the three main theorems).
- **General DeFi risk categories** — see [Risks](/docs/trust/risks).

---

*This page covers Dol's framework-level assumptions only. For the broader plain-English risk list see [Risks](/docs/trust/risks). For the legal version of every risk category, see the [Risk Disclosure](/legal/risk).*
