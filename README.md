# Dol

A consumer-first DeFi product that turns Pacifica's perp infrastructure into a simple, 3-tap yield experience.

Built on Pacifica's performance, APIs, and builder infrastructure — packaged into a retail-ready product for everyday users.

Dol is a market-neutral yield product for everyday crypto users. Deposit USDC, receive DOL 1:1, and redeem through a fast retail-style flow — without needing to understand funding, hedging, or cross-venue execution. Under the hood, Dol uses a Rust decision engine and Pacifica-native infrastructure to capture funding-rate differentials while minimizing directional exposure through matched same-asset hedges.

```
                       Simple UI
                           │
                           ▼
       Deposit USDC ──► mint DOL (1:1 receipt) ──► redeem DOL → USDC
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

## Why users adopt Dol

- No need to understand funding markets, hedge routing, or execution logic
- Retail-first UX inspired by the simplicity of products like Robinhood and Toss
- Clean deposit and redeem flow instead of fragmented DeFi workflows
- Built on Pacifica's performance, APIs, and builder infrastructure

---

## Why Dol matters for Pacifica

Dol shows how Pacifica can power not only trading interfaces, but real consumer financial products. It converts Pacifica's perp infrastructure into a retail-facing application that expands ecosystem utility, showcases builder leverage, and makes sophisticated strategies accessible to a broader class of users.

---

## Simple by design

Traditional DeFi yield products ask users to understand venues, funding, leverage, collateral, and risk mechanics before they can do anything useful. Dol removes that complexity.

With Dol, the user experience is simple:

- Deposit USDC
- Receive DOL 1:1 as a receipt token
- Track value through a clean dashboard
- Redeem through a fast, intuitive flow

The complexity stays in the system, not on the user.

---

## Fast redemption, with guardrails

Dol is built to feel instant for normal user flows.

Under normal conditions, users can move through a fast redemption path designed for a smooth retail experience. Under stressed conditions, the system is protected by buffer rules, guardian controls, and fallback redemption logic designed to preserve system integrity and user safety.

We optimize for simplicity at the surface and discipline underneath.

DOL is a receipt token representing a claim on a USDC-funded strategy vault. It is not an algorithmic stablecoin or an endogenous monetary asset.

---

## One product, four layers

| Layer | What | Where |
|---|---|---|
| **Retail UI** | Next.js 14 + Privy wallet login. Three-tap deposit flow. Instant-withdraw UX. Operator dashboard with 10-symbol live NAV chart. | [`dashboard/`](./dashboard/) |
| **Smart contracts** | `Dol.sol` ERC-20 receipt token with `deposit`, `redeem`, `instantRedeem`, cooldown queue, guardian role. Deployed on Base Sepolia. | [`contracts/`](./contracts/) |
| **Runtime bot** | Rust workspace (8 crates). Decision engine, funding-cycle lock, Pacifica authenticated adapter with builder-code attribution, NAV tracker, multi-venue routing. | [`bot-rs/`](./bot-rs/) |
| **Math framework** | Aurora-Ω reference implementation in Python. Conformal prediction, Maurer-Pontil empirical Bernstein bounds, CVaR budget, α-cascade strict-proper forecast scoring, chance-constrained portfolio, Hurst-regime routing, funding-cycle lock formal spec. | [`strategy/`](./strategy/) |

---

## Proof of execution

Dol is not a concept deck. It is a working full-stack product.

- Live deposit and redeem flow demonstrated successfully
- Smart contracts for receipt-token and redemption mechanics
- Rust runtime for routing, risk enforcement, and NAV logic
- Math framework for forecasting, bounds, portfolio constraints, and cycle discipline
- Full-stack validation with extensive automated tests
- Rust ↔ Python parity checks for core strategy logic

We focused on building a product that works end-to-end, not just presenting an idea.

### Test coverage: **687 passing, 0 failing** across the full stack

```bash
# Rust runtime tests                (361 passing, 17 live-gated)
cd bot-rs && cargo test --workspace

# Python framework tests           (130 passing)
cd strategy && python -m pytest tests/ -v

# Solidity contract tests          (196 passing: unit + fuzz + invariant)
cd contracts/packages/contracts && forge test
```

| Component | Passing | Coverage |
|---|---:|---|
| Rust runtime (`bot-rs`) | 361 | decision engine, cycle lock, Pacifica adapter, NAV accounting, parity harness |
| Python framework (`strategy`) | 130 | OU MLE, empirical Bernstein, conformal prediction, α-cascade scoring, CVaR, funding cycle lock, chance-constrained portfolio |
| Solidity contracts (`contracts`) | 196 | `Dol.sol` unit + `PacificaCarryVault` fuzz/invariant + 11 test suites |
| **Total** | **687** | **0 failing** |

Plus 17 additional Rust live-credential integration tests (gated on `PACIFICA_API_KEY`) and a Rust ↔ Python parity harness (`strategy/rust_fixtures/`) that cross-verifies 22 math modules against byte-level expected outputs.

---

## Risk model and execution discipline

Dol is designed to make a complex strategy feel simple to the user, without hand-waving the underlying risk model.

The system targets funding-rate differentials as the primary return driver while minimizing directional exposure through matched same-asset cross-venue hedges. Execution is constrained by explicit guardrails in the runtime, including venue restrictions, symbol matching, cycle discipline, and emergency flattening logic.

Our goal is not to promise "risk-free yield," but to combine transparent system design, disciplined execution, and retail-grade usability.

The framework is specified across [`strategy/docs/math-aurora-omega-appendix.md`](./strategy/docs/math-aurora-omega-appendix.md) (strict-propriety proof, concentration bounds, CVaR derivation), [`strategy/docs/math-frontier.md`](./strategy/docs/math-frontier.md) (conformal prediction, DRO, Hurst DFA), and [`strategy/docs/math-rigorous.md`](./strategy/docs/math-rigorous.md) (regime routing, chance-constrained portfolio).

---

## Pacifica integration — five touchpoints

Pacifica is the center of this system, not an afterthought.

1. **Public funding-rate feed** — hourly rates for all 10 symbols, parsed into annualized APY.
2. **Order-book depth** — feeds the fractal liquidity allocator (`strategy/strategy/depth_threshold.py`).
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
cd dashboard/packages/dashboard
cp .env.example .env.local   # fill in Privy app ID, RPC URL, etc.
cd ../..
pnpm install
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

## Why Dol

Dol abstracts away funding, hedging, and venue complexity into a clean, retail-friendly product experience built on Pacifica.

---

## What's next

Dol grows in lockstep with [Pacifica's long-term vision](https://docs.pacifica.fi).

- **Staircase live rollout.** Rust ↔ Python parity harness; staircase deployment on Pacifica ($100 → $1,000 → $10,000); authenticated adapter + builder-code revenue share live.
- **Retail-ready.** Mobile-first deposit flow alongside Pacifica iOS/Android launch; multi-collateral deposits via Pacifica Unified Trading Accounts (USDC, BTC, ETH, SOL); full 4-layer risk stack live on mainnet.
- **Pacifica Phase 3 ready.** 46-pair universe including Pacifica spot + RWA + exotic derivatives; Pacifica Options tail-risk overlays; native yield via Pacifica Lending; decision kernel portable to Pacifica L1 WASM runtime.
- **ERC-4626 compatibility** for standardized vault integrations and clearer share-based accounting.
- **Async redemption architecture** for stressed conditions, with ERC-7540-style request flows under evaluation.

---

## Deployment

**Contracts (Base Sepolia)**
- `Dol.sol`: [`0x9E6Cc40CC68Ef1bf46Fcab5574E10771B7566Db4`](https://sepolia.basescan.org/address/0x9E6Cc40CC68Ef1bf46Fcab5574E10771B7566Db4)

**Live dashboard**: https://dol-finance.vercel.app

**Pacifica builder code**: registered

---

## Team

Built by a team of two for the Pacifica Hackathon:

| Role | Scope |
|---|---|
| **Quantitative researcher** | Strategy design, Aurora-Ω math framework, formal proofs (strict-propriety, empirical Bernstein, CVaR derivation), risk model, funding-cycle lock spec |
| **Engineer** | Rust runtime (8-crate workspace), Solidity contracts, Next.js dashboard, Pacifica authenticated adapter, Rust ↔ Python parity harness |

Two people, four languages (Rust / Python / Solidity / TypeScript), 687 passing tests, one deployed contract, one end-to-end retail UX.

---

## License

MIT — see [`LICENSE`](./LICENSE).

---

*Dol is the Korean word for "stone." In the right system, something inert becomes useful — simple on the surface, deeply engineered underneath.*
