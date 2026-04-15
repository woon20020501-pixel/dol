# Risks

This is the short version of what can go wrong. The full legal version with every risk category and the exact language we want you to acknowledge is in the [Risk Disclosure](/legal/risk). Read both before you commit any meaningful amount of money.

## The five things that can hurt you

### 1. Dol is not a bank deposit

Dol is a crypto asset. It is not insured by FDIC, KDIC, FSCS, or any government insurance program. There is no government guarantee that you will get your money back. The value can go to zero. You can lose everything you put in.

If you need a guaranteed savings product, do not buy Dol. Use a regulated bank account.

### 2. The strategy can lose money

The Dol pool runs a real trading strategy on real markets. Most of the time it captures small fees from market participants, and that is what makes the value of your Dol grow. But the strategy can and will lose money in some conditions:

- **Market regime change** — the funding fee patterns the strategy depends on can disappear or invert
- **Venue problems** — if the Pacifica DEX has an outage or a bug, the strategy may be unable to exit positions
- **Basis decoupling** — if the perpetual futures price drifts away from the spot price unexpectedly

When the strategy loses money, the value of your Dol goes down — Dol holders absorb the strategy result directly. There is no separate buffer that absorbs losses for you.

### 3. Cash-out is not always instant

The Instant cash-out path is served from a 50 percent liquid buffer. If too many people cash out instantly at the same time, the buffer drains and Instant fails. When that happens you can still use Scheduled (30-minute cooldown, no fee), but you cannot get your money out in the same second.

Under extreme market stress, even Scheduled cash-out could be delayed beyond 30 minutes if the protocol cannot rotate funds out of the strategy in time.

### 4. Smart contract risk

The Dol on-chain protocol is tested (117 tests, 100 percent vault coverage, Slither clean) but **not formally audited yet**. Tests reduce risk but do not eliminate it. A bug or exploit in any of the contracts — Dol token, Pacifica vault, treasury vault — could result in a partial or total loss of pool funds. If that happens, the loss is irreversible. There is no insurance and no recovery mechanism.

### 5. Regulatory risk

Crypto law varies by country and changes fast. Dol is available only in five countries because we cannot afford the legal work to be available everywhere. If a regulator in one of those five countries — or in the Cayman Islands where the Dol entity is being established — orders us to stop, we stop. That could affect cash-outs, the value of your Dol, or your ability to access the service at all.

## What is *not* a major risk in Phase 1

We want to be honest about both directions, so here are things people often worry about that are *not* a meaningful Phase 1 risk:

- **Bank-run failure.** The backtest shows the protocol survives 50 percent daily withdrawal demand for the full year in 99.9 percent of paths. Bank runs are stress-test material, not a primary failure mode.
- **A team rug pull.** The contracts are non-custodial. We do not hold your keys. We cannot freeze your Dol or send your USDC anywhere. The worst we can do operationally is shut down the frontend website — your assets remain accessible by direct contract call from any wallet you control.
- **Fee surprises.** The Instant fee is 0.05 percent and the Scheduled fee is 0 percent, both fixed in the contract code. No hidden fees.

## Read this next

For the strategy-level assumptions that shape these risk categories — the cash-out path design, the delta-neutral stress bounds, and the Phase 1 capacity ceiling — read [Framework assumptions](/docs/trust/framework-assumptions). That page pairs with this one: this page lists risk categories in plain English, the assumptions page goes one level deeper into how each one is bounded.

For the full risk list with every category and the legal language you will be asked to acknowledge before depositing, read the [Risk Disclosure](/legal/risk). It covers eleven risk categories in detail and is the document we ask you to formally accept on the cash-out clickwrap.

If anything on this page is unclear, ask a question on our support channels (see [Support](/docs/more/support)). We would rather answer your question now than have you commit to something you do not fully understand.
