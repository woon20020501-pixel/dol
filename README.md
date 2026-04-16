# Dol

**Toss erased the certificate wall. Dol erases the DeFi wall.**

DeFi yield has been stuck on the same problem for a decade: *Which chain? Which protocol? Which collateral? Which risk? Which redemption window?* If a user can't answer all five, they can't deposit. Ethena, Resolv, and Elixir pushed strategies forward — but left all five questions in place.

Dol erases them.

What the user does: **deposit USDC. receive DOL. redeem DOL.** That's it.

```
       Deposit USDC ──► mint DOL (1:1 receipt) ──► redeem DOL → USDC
```

---

## How Dol erases the five questions

| Question the user no longer asks | How Dol removes it |
|---|---|
| **"Which chain?"** | Pacifica-native. Single venue, single chain. No bridge decisions. |
| **"Which protocol?"** | One receipt token: DOL. No vault-hopping, no LP pairing, no gauge voting. |
| **"Which collateral?"** | USDC. One asset in, one asset out. |
| **"Which risk?"** | Market-neutral by construction — same asset, two venues, opposite sides. Price exposure is provably zero, not statistically small. |
| **"Which redemption window?"** | Fast redeem path under normal conditions. Buffer rules + guardian controls under stress. No 7-day unlock, no epoch wait. |

Every architectural decision in this repo exists to erase one of these five questions. Nothing else.

---

## What it takes to make something this simple

Making five questions disappear requires that the system answers them internally, permanently, and correctly. That is the real engineering challenge — not the strategy itself, but making the strategy invisible.

### Market-neutral execution (erases "which risk?")

The system holds the same perpetual contract on two DEX venues — long on one, short on the other — capturing only the funding-rate differential. Price exposure cancels by construction (β = 1.0), enforced at the Rust type level as a closed invariant. This is not a diversification argument; it is a mechanical cancellation.

### 442 tests across 4 languages (the cost of simplicity)

If the UX is this simple, the backend cannot fail silently. That is why there are 442 passing tests and 0 failing — not as a vanity metric, but as the engineering cost of removing five questions from the user's screen.

```bash
cd bot-rs     && cargo test --workspace          # 204 passed, 0 failed
cd strategy   && python -m pytest tests/          # 130 passed
cd contracts  && forge test                       # 110 passed (unit + fuzz + invariant)
```

| Component | Tests | What it covers |
|---|---:|---|
| Rust runtime | 204 | decision engine, funding-cycle lock, Pacifica adapter, NAV accounting, parity harness |
| Python math framework | 130 | OU MLE, empirical Bernstein, conformal prediction, α-cascade scoring, CVaR, chance-constrained portfolio |
| Solidity contracts | 110 | Dol.sol deposit/redeem, PacificaCarryVault fuzz + invariant |
| **Total** | **444** | **0 failing** |

### Math framework (the prerequisite for erasing "which risk?")

The user never asks about risk because the system answers it formally. Conformal prediction, Maurer-Pontil empirical Bernstein bounds, CVaR budgeting, α-cascade strict-proper forecast scoring — these exist so the user doesn't have to. The math is specified in [`strategy/docs/`](./strategy/docs/) and every claim has a unit test.

### Rust ↔ Python parity harness (erases drift between spec and runtime)

The Python reference framework defines the math. The Rust runtime executes it. A byte-level parity harness (`strategy/rust_fixtures/`, 22 fixture sections) ensures they never diverge. The spec is the code; the code is the spec.

---

## Why Dol matters for Pacifica

Dol is not a trading tool built on top of Pacifica. It is a **consumer financial product** built on top of Pacifica.

This distinction matters because it shows what the Pacifica builder ecosystem enables beyond raw trading interfaces:

- Turning perp infrastructure into a retail savings product
- Making sophisticated cross-venue strategies invisible to end users
- Demonstrating that Pacifica's APIs, performance, and builder primitives can power products that compete with fintech apps, not just with other DEXes

Five Pacifica integration points make this possible:

1. **Funding-rate feed** — hourly rates across 10 symbols
2. **Order-book depth** — feeds the capacity allocator
3. **Authenticated endpoint** — Ed25519 API-key sign-in
4. **Builder-code attribution** — every decision tagged for the revenue-share program
5. **XAU / XAG / PAXG RWA perps** — Pacifica lists gold and silver; most venues don't

---

## One product, four layers

| Layer | Purpose | Where |
|---|---|---|
| **Retail UI** | Three-tap deposit. Fast redeem. Operator dashboard with live 10-symbol NAV chart. Next.js 14 + Privy. | [`dashboard/`](./dashboard/) |
| **Smart contracts** | DOL receipt token. `deposit`, `redeem`, `instantRedeem`, cooldown queue, guardian role. Deployed on Base Sepolia. | [`contracts/`](./contracts/) |
| **Runtime** | Rust workspace (8 crates). Decision engine, funding-cycle lock, Pacifica authenticated adapter, NAV tracker. | [`bot-rs/`](./bot-rs/) |
| **Math framework** | Aurora-Ω. Conformal prediction, Bernstein bounds, CVaR, strict-proper scoring, chance-constrained portfolio, Hurst-regime routing. | [`strategy/`](./strategy/) |

DOL is a receipt token representing a claim on a USDC-funded strategy vault. It is not an algorithmic stablecoin or an endogenous monetary asset.

---

## 10-symbol universe

| Crypto perps | Real-world-asset perps |
|---|---|
| BTC, ETH, SOL, BNB, ARB, AVAX, SUI | XAU (gold), XAG (silver), PAXG |

Ten independent hedge pairs. Each pair: long on one venue, short on another, same underlying. The RWA pairs carry an oracle-divergence disclosure in the UI — gold and silver perps use independent oracles on each venue, and we surface that honestly rather than hide it.

---

## Quickstart

```bash
git clone https://github.com/woon20020501-pixel/dol.git && cd dol

# Tests
cd strategy   && pip install -r requirements.txt && python -m pytest tests/ && cd ..
cd bot-rs     && cargo test --workspace && cd ..
cd contracts/packages/contracts && forge test && cd ../../..

# Dashboard
cd dashboard/packages/dashboard && cp .env.example .env.local && cd ../..
pnpm install && pnpm --filter dashboard dev    # → http://localhost:3000

# Bot (dry-run)
cd bot-rs && cargo run --release --bin bot-cli -- demo \
  --continuous --tick-interval-secs 2 --starting-nav 10000 \
  --signal-dir output/demo/signals --nav-log output/demo/nav.jsonl
```

---

## What's next

Dol grows in lockstep with [Pacifica's long-term vision](https://docs.pacifica.fi).

- **Staircase live rollout.** $100 → $1,000 → $10,000 real capital on Pacifica mainnet; builder-code revenue share live.
- **Mobile-first.** Native deposit flow alongside Pacifica iOS/Android; multi-collateral via Unified Trading Accounts.
- **Pacifica Phase 3.** 46-pair universe including spot + RWA + exotic derivatives; Options as tail-risk overlays; native yield via Pacifica Lending; decision kernel portable to Pacifica L1 WASM runtime.
- **ERC-4626 compatibility** for standardized vault integrations.
- **Async redemption architecture** for stressed conditions (ERC-7540-style request flows under evaluation).

---

## Deployment

| | |
|---|---|
| **Dol.sol (Base Sepolia)** | [`0x9E6Cc40CC68Ef1bf46Fcab5574E10771B7566Db4`](https://sepolia.basescan.org/address/0x9E6Cc40CC68Ef1bf46Fcab5574E10771B7566Db4) |
| **Live dashboard** | [dol-finance.vercel.app](https://dol-finance.vercel.app) |
| **Pacifica builder code** | registered |

---

## Team

Built by two people for the Pacifica Hackathon.

| Role | Scope |
|---|---|
| **Quantitative researcher** | Strategy design, Aurora-Ω math framework, formal proofs, risk model, funding-cycle lock spec |
| **Engineer** | Rust runtime (8 crates), Solidity contracts, Next.js dashboard, Pacifica adapter, parity harness |

Two people. Four languages. 444 tests. One deployed contract. One thesis: **erase the five questions.**

---

## License

MIT — see [`LICENSE`](./LICENSE).

---

*Dol is the Korean word for "stone." In the right system, something inert becomes useful — simple on the surface, deeply engineered underneath.*
