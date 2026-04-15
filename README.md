# Dol

**The easiest way to earn stable yield on your USDC.**

Dol is a delta-neutral cross-venue funding-rate harvester on [Pacifica](https://app.pacifica.fi). Regular crypto holders deposit USDC, receive DOL tokens as a one-to-one receipt, and can burn them for instant USDC back at any time — no lock-ups, no cooldowns, no dashboards to watch. Under the hood a Rust decision engine runs the Aurora-Ω math framework to capture funding-rate spreads across DEX venues while remaining strictly β = 1.0 to the underlying asset.

```
                       Simple UI
                           │
                           ▼
       Deposit USDC ──► mint DOL (1 : 1) ──► instant burn → USDC
                           │
                           ▼
                Rust decision engine (Aurora-Ω)
                           │
            ┌──────────────┼──────────────┐
            ▼              ▼              ▼
        Pacifica       Hyperliquid     Lighter / Backpack
       (maker leg)     (hedge leg)       (fallback)
```

---

## One product, four layers

| Layer | What | Where |
|---|---|---|
| **Retail UI** | Next.js 14 + Privy wallet login. Three-tap deposit flow. Instant-withdraw UX. Operator dashboard with 10-symbol live NAV chart. | [`dashboard/`](./dashboard/) |
| **Smart contracts** | `Dol.sol` ERC-20 receipt token with `deposit`, `redeem`, `instantRedeem`, cooldown queue, guardian role. Deployed on Base Sepolia. | [`contracts/`](./contracts/) |
| **Runtime bot** | Rust workspace (8 crates). Decision engine, funding-cycle lock, Pacifica authenticated adapter with builder-code attribution, NAV tracker, multi-venue routing. | [`bot-rs/`](./bot-rs/) |
| **Math framework** | Aurora-Ω reference implementation in Python. Conformal prediction, Maurer-Pontil empirical Bernstein bounds, CVaR budget, α-cascade strict-proper forecast scoring, chance-constrained portfolio, Hurst-regime routing, funding-cycle lock formal spec. | [`strategy/`](./strategy/) |

---

## Test coverage: **442 passing, 0 failing** across the full stack

We don't hand-wave the risk model. Every mathematical claim has a unit test behind it.

```bash
# Rust runtime tests                (202 passing, 17 live-gated)
cd bot-rs && cargo test --workspace

# Python framework tests           (130 passing)
cd strategy && python -m pytest tests/ -v

# Solidity contract tests          (110 passing: unit + fuzz + invariant)
cd contracts/packages/contracts && forge test
```

| Component | Passing | Coverage |
|---|---:|---|
| Rust runtime (`bot-rs`) | 202 | decision engine, cycle lock, Pacifica adapter, NAV accounting, parity harness |
| Python framework (`strategy`) | 130 | OU MLE, empirical Bernstein, conformal prediction, α-cascade scoring, CVaR, funding cycle lock, chance-constrained portfolio |
| Solidity contracts (`contracts`) | 110 | `Dol.sol` unit + `PacificaCarryVault` fuzz/invariant tests |
| **Total** | **442** | **0 failing** |

Plus 17 additional Rust live-credential integration tests (gated on `PACIFICA_API_KEY`) and a Rust ↔ Python parity harness (`strategy/rust_fixtures/`) that cross-verifies 22 math modules against byte-level expected outputs.

---

## The iron law: β = 1.0 by construction

Dol holds the **same perpetual contract on two different DEX venues** — long on one, short on the other — and captures only the funding-rate spread between them. Because both legs reference the same underlying asset, price exposure cancels **mechanically**, not statistically. We enforce this in four "iron-law walls" inside the Rust runtime:

1. **I-LOCK** — `funding_cycle_lock`: once a direction is committed on a pair, no mid-cycle flipping.
2. **I-VENUE** — venue whitelist enforced at the Rust type level (closed enum). Adding a CEX requires editing a type signature, which trips code review.
3. **I-SAME** — symbol-equality check: long-leg and short-leg symbols must match on-chain identifiers.
4. **I-KILL** — FSM emergency flatten on adapter health, authorization failure, or oracle divergence.

The framework is specified across [`strategy/docs/math-aurora-omega-appendix.md`](./strategy/docs/math-aurora-omega-appendix.md) (strict-propriety proof, concentration bounds, CVaR derivation), [`strategy/docs/math-frontier.md`](./strategy/docs/math-frontier.md) (conformal prediction, DRO, Hurst DFA), and [`strategy/docs/math-rigorous.md`](./strategy/docs/math-rigorous.md) (regime routing, chance-constrained portfolio).

---

## Pacifica integration — five touchpoints

Pacifica is the center of this system, not an afterthought.

1. **Public funding-rate feed** — hourly rates for all 10 symbols, parsed into annualized APY.
2. **Order-book depth** — feeds the fractal liquidity allocator (`strategy/depth_threshold.py`).
3. **Authenticated account endpoint** — real API-key sign-in with Ed25519 (`bot-rs/crates/bot-adapters/src/pacifica_auth.rs`).
4. **Builder-code attribution** — every decision tagged in the signal JSON audit trail; routed to our registered builder account for the revenue-share program.
5. **XAU / XAG / PAXG real-world-asset perps** — Pacifica's RWA-perps as first-class yield sources. Most perp venues don't list gold or silver at all.

---

## Quickstart

### Run the full test suite

```bash
git clone https://github.com/woon20020501-pixel/dol.git
cd dol

# Python framework
cd strategy && pip install -r requirements.txt && python -m pytest tests/
cd ..

# Rust runtime
cd bot-rs && cargo test --workspace
cd ..

# Solidity contracts (requires foundry: https://getfoundry.sh)
cd contracts/packages/contracts && forge test
```

### Run the dashboard locally

```bash
cd dashboard
pnpm install
cp packages/dashboard/.env.example packages/dashboard/.env.local  # fill in values
pnpm --filter dashboard dev
```

Then open http://localhost:3000.

### Run the Rust bot in dry-run mode

```bash
cd bot-rs
cargo run --release --bin bot-cli -- demo \
  --continuous \
  --tick-interval-secs 2 \
  --starting-nav 10000 \
  --signal-dir output/demo/signals \
  --nav-log output/demo/nav.jsonl
```

The dashboard's `/api/nav` route will pick up `nav.jsonl` automatically.

---

## 10-symbol universe

| Crypto perps | Real-world-asset perps |
|---|---|
| BTC, ETH, SOL, BNB, ARB, AVAX, SUI | XAU (gold), XAG (silver), PAXG |

Ten independent hedge pairs trading simultaneously. Each pair is long-on-one-venue, short-on-another, same underlying asset. The RWA pairs carry a structural basis-risk disclosure in the UI and in the signal JSON (gold/silver perps use independent oracles on each venue — we disclose it openly rather than hide it).

---

## Why Dol, not another perp vault

- **Retail-first, not operator-first.** Three taps to deposit. Instant burn for withdraw. No funding cycles to understand, no venues to pick, no rebalances to do.
- **Delta-neutral by construction, not by vibes.** Same-asset two-venue hedge. β = 1.0 before any trade fires. Iron-law walls enforced in Rust types.
- **Proof, not demos.** 442 passing tests, formal math spec in the repo, byte-level parity harness between the Rust runtime and the Python reference framework, every invariant tested.
- **DEX-only, non-custodial.** No CEX dependency. No KYC. Users hold their keys at all times.
- **Honest disclosure.** The RWA pairs carry an oracle-divergence warning; the demo mode is labelled; the Base Sepolia deployment is a testnet; the test-count claim in the demo video is verifiable with one command.

---

## Roadmap

Dol grows in lockstep with [Pacifica's long-term vision](https://docs.pacifica.fi).

- **Week 2 — Validation.** Rust ↔ Python parity harness; staircase live rollout on Pacifica ($100 → $1,000 → $10,000); authenticated adapter + builder-code revenue share live.
- **Month 1 — Retail-ready.** Mobile-first deposit flow alongside Pacifica iOS/Android launch; multi-collateral deposits via Pacifica Unified Trading Accounts (USDC, BTC, ETH, SOL); full 4-layer risk stack live on mainnet.
- **Month 3 — Phase 3 ready.** 46-pair universe including Pacifica spot + RWA + exotic derivatives; Pacifica Options tail-risk overlays (covered calls, touch structures); native yield via Pacifica Lending; any deposit size from $10 to $10,000+; decision kernel portable to Pacifica L1 WASM runtime.

---

## Deployment

**Contracts (Base Sepolia)**
- `Dol.sol`: [`0x9E6Cc40CC68Ef1bf46Fcab5574E10771B7566Db4`](https://sepolia.basescan.org/address/0x9E6Cc40CC68Ef1bf46Fcab5574E10771B7566Db4)

**Live dashboard**: https://dol-finance.vercel.app

**Pacifica builder code**: registered *(update with actual code)*

---

## Team

Built by a team of two for the Pacifica Hackathon:

| Role | Scope |
|---|---|
| **Quantitative researcher** | Strategy design, Aurora-Ω math framework, formal proofs (strict-propriety, empirical Bernstein, CVaR derivation), risk model, funding-cycle lock spec |
| **Engineer** | Rust runtime (8-crate workspace), Solidity contracts, Next.js dashboard, Pacifica authenticated adapter, Rust ↔ Python parity harness |

Two people, four languages (Rust / Python / Solidity / TypeScript), 442 passing tests, one deployed contract, one end-to-end retail UX.

---

## License

MIT — see [`LICENSE`](./LICENSE).

---

*Dol is the Korean word for "stone". A stone yields nothing on its own — put it in the right machine, and it becomes stable yield, accessible to anyone.*
