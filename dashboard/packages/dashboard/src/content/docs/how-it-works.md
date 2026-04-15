# How it works

The full mechanics in plain English. If you have read [What is Dol](/docs/getting-started/what-is-dol), this page goes one level deeper — what actually happens to your USDC after you buy a Dol, and how the value grows.

There are five pieces. Read them in order.

## 1. Pooled capital

When you exchange USDC for Dol, your USDC does not sit in a personal account. It joins a shared pool of USDC contributed by everyone else who bought Dol. The pool is held by an on-chain protocol on Base Sepolia, not by Dol Labs and not by any custodian. We do not hold your keys. We cannot move your money on our own — every change to the pool is a public transaction that anyone can verify.

In exchange for the USDC you contribute, you receive Dol tokens at a one-to-one ratio. Your Dol tokens are your claim on a slice of the pool. Bigger pool, more value flowing through the strategy. Smaller pool, less.

## 2. The strategy

The pool does not just sit there. **Up to half of it** is deployed into a market strategy that captures small mispricings on the Pacifica decentralized exchange. The other half is held as a liquid USDC buffer so we can pay people back instantly when they want to cash out (more on this in section 4).

The strategy captures funding-rate spreads on perpetual futures contracts and hedges the directional exposure so the spread is what we keep. This is **not directional speculation**. We are not betting on whether the underlying market goes up or down. We are capturing the small fees that one side of a crowded market pays the other side, and we hedge the price movement so that fee is what survives.

This is a real strategy with real risk. It can lose money, especially during market regime changes or when a venue has technical problems. The [Risk Disclosure](/legal/risk) explains the full set of strategy risks, and [Framework assumptions](/docs/trust/framework-assumptions) lists the three high-level assumptions baked into how the strategy is designed.

## 3. How returns flow to you

Here is where most people get confused. **The number of Dol tokens in your wallet stays the same** while you hold. You will always see the same balance. What changes is the **value of each Dol** measured in USDC.

When the strategy makes money, the pool grows. Each Dol you hold now represents a slightly larger slice of a slightly larger pool. The price-per-Dol — measured in USDC — quietly ticks up.

A simple example. You buy 100 Dol for 100 USDC, so the starting price is 1.00 USDC per Dol. A month later, the strategy has made 0.5 percent. The pool is now worth 100.5 USDC for every 100 Dol of yours. The price-per-Dol is now 1.005. Your 100 Dol is worth 100.5 USDC. You did not need to claim it, sign anything, or pay any fee. The growth happened on its own.

When you cash out, you get the current value: 100 Dol returns 100.5 USDC.

## 4. Two ways to cash out

When you want your money back, you have two options. Both are non-custodial — your Dol is burned and USDC is sent directly to your wallet. The difference is speed versus fee.

**Instant.** Your Dol is exchanged for USDC in the same transaction. You pay a 0.05 percent fee on the amount you cash out. No waiting. The fee goes to the Dol team to fund operations.

**Scheduled.** Same exchange, but you wait through a 30-minute cooldown before claiming your USDC. No fee. You submit a request now, and after 30 minutes you come back and claim. The cooldown gives the protocol time to rotate funds out of the active strategy if needed.

You can pick either one every time you cash out. Use Instant when you need speed, Scheduled when you want to save the fee.

## 5. The liquid buffer

**Roughly half of the pool is kept as a liquid USDC buffer at all times.** This buffer is what makes Instant cash-out possible — when you tap Instant, the USDC comes from the buffer, not from the strategy positions. We do not need to unwind any trades to pay you.

If a lot of people cash out instantly at once and the buffer drains, Instant withdrawals will fail with an "Insufficient liquidity" error. When that happens, you can switch to Scheduled, which pulls from the strategy side after the 30-minute cooldown. The buffer refills automatically as deposits come in and as the operator bot rebalances.

This split — half buffer, half active strategy — is the protocol's target ratio. It is enforced as a soft target maintained by the operator rather than as a hard on-chain rule, because hard on-chain rules cost gas on every deposit and cash-out. The reason the soft target is acceptable is that the worst case if the operator fails or misbehaves is that Instant cash-outs become temporarily unavailable and everyone has to use Scheduled. **Funds are not at risk from this — only the convenience of the Instant path.**

For the deeper context — the assumptions baked into the buffer split, the stress scenarios that could move it, and the Phase 1 TVL ceiling Dol chose for itself — read [Framework assumptions](/docs/trust/framework-assumptions).

---

That is the entire system. Five pieces: pooled capital, the strategy, value flowing through the price-per-Dol, two cash-out paths, and the liquid buffer.

If you want to go even deeper:

- **[On-chain & verified →](/docs/trust/on-chain)** — exact contract addresses, test results, how to verify yourself
- **[Architecture →](/docs/trust/architecture)** — the engineering overview: math framework, type-safe Rust, parity testing
- **[Framework assumptions →](/docs/trust/framework-assumptions)** — the three high-level assumptions baked into the strategy
- **[Risks →](/docs/trust/risks)** — the short risk list
- **[Risk Disclosure →](/legal/risk)** — the full legal version
