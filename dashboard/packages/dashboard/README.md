# Pacifica FX Carry Vault — Dashboard

Single-screen dashboard for the Pacifica FX Carry Vault. Shows real-time
vault metrics, funding spread charts, position data, and deposit/withdraw
flows.

## Quick start

```bash
# From the monorepo root
cd packages/dashboard

# Copy env and fill in values
cp .env.example .env.local

# Install dependencies (from monorepo root)
pnpm install

# Run dev server
pnpm dev
```

The dashboard runs on **http://localhost:3001** (configured in `package.json`).

## Environment variables

| Variable | Required | Default | Description |
|---|---|---|---|
| `NEXT_PUBLIC_RPC_URL` | No | `https://sepolia.base.org` | RPC endpoint for on-chain reads |
| `NEXT_PUBLIC_BOT_API_URL` | No | `http://localhost:7777` | Bot Status API base URL |
| `NEXT_PUBLIC_DEMO_MODE` | No | `false` | Show deterministic demo data instead of live data |
| `NEXT_PUBLIC_CHAIN_ID` | No | `84532` | Target chain ID (84532 = Base Sepolia) |
| `NEXT_PUBLIC_PRIVY_APP_ID` | Yes | — | Privy app ID for wallet/email/social login |

## Pointing at different data sources

**Local bot (default):**
```env
NEXT_PUBLIC_BOT_API_URL=http://localhost:7777
```

**Remote bot:**
```env
NEXT_PUBLIC_BOT_API_URL=https://your-bot-server.example.com
```

**Demo mode (no bot or contract needed):**
```env
NEXT_PUBLIC_DEMO_MODE=true
```

Demo mode renders realistic fake data matching the backtest results from
PLAN.md. No network requests are made. Deposit/withdraw buttons are
disabled with a "Demo mode" label.

## Build

```bash
pnpm build   # Production build
pnpm lint    # ESLint check
pnpm dev     # Dev server with hot reload
```

## Deploy to Vercel

1. Import the repo in Vercel
2. Set **Root Directory** to `packages/dashboard`
3. Set **Framework Preset** to Next.js
4. Add environment variables (see table above)
5. Deploy

The build command is `next build` and the output directory is `.next`.

## Yield architecture

V1.5 stacks two yield sources on top of the same TVL:

```
                         depositor
                            │
                            ▼
                    ┌───────────────┐
                    │  CarryVault   │   ERC-4626
                    │   (USDC)      │
                    └───────┬───────┘
                            │ totalAssets
            ┌───────────────┴───────────────┐
            │                               │
        70% margin                     30% treasury
            │                               │
            ▼                               ▼
   ┌─────────────────┐             ┌─────────────────┐
   │ Pacifica perp   │             │ Moonwell market │
   │  short USDJPY   │             │  USDC supply    │
   │       +         │             │                 │
   │ Lighter perp    │             │  permissionless │
   │  long  USDJPY   │             │  base yield     │
   └────────┬────────┘             └────────┬────────┘
            │                               │
       funding α                        base APY
       (~13% × 0.7)                     (~5% × 0.3)
            │                               │
            └──────────────┬────────────────┘
                           ▼
                     total APY ≈ ~10%
                  delta-neutral, hedged
```

- **Funding alpha** comes from the perp leg only. The vault is short
  USDJPY on Pacifica (the high-funding venue) and long USDJPY on
  Lighter (the hedge leg). Net price exposure is ~zero; the vault
  collects the funding rate spread.
- **Treasury yield** is the conservative base layer. 30% of TVL is
  supplied to a Moonwell-style permissionless lending market for a
  steady ~5% APY, which routes to real Moonwell on V2 mainnet.
- **Total APY** is the share-weighted sum of the two layers, surfaced
  in the hero strip and broken down in the "Yield Sources" panel.

The dashboard reads the live allocation from the vault config
(`shared/contracts.json` → `allocation.{treasuryBps,marginBps}`) and
the live treasury balance from the configured `treasuryVault` address
when present. If those fields are missing it falls back to a 70/30
synthetic split so the UI is still demo-ready pre-deploy.

### NAV reporter

Vault NAV is not implicitly trusted from on-chain reads alone — perp
positions live off-chain (Pacifica, Lighter), so somebody has to mark
the vault to market. That somebody is the **NAV reporter**: an
off-chain process owned by the operator that, every ~5 minutes,
fetches every leg's mark, computes the total NAV, and submits it
on-chain via `vault.reportNAV(value)` signed by the operator key.

The vault contract enforces a **±10% sanity guard** at write time:
any reported NAV that drifts more than 10% from the previous reported
value is rejected. This caps how badly a compromised or malfunctioning
operator can mark the vault per tick, while still letting honest
updates through.

The dashboard surfaces all of this in the **NAV Reporter** card in the
right sidebar: operator address (with copy), last report relative
time, last reported NAV in USD, last tx hash (linked to BaseScan),
and a live countdown to the next scheduled report. A status pill in
the card header (live / dry-run / error / idle) tells the user at a
glance whether their funds are being honestly priced right now, and
the page header surfaces a colored hint if the reporter is erroring
or has not yet posted an initial report.

## Architecture

- **Next.js 14** (App Router) + TypeScript strict
- **shadcn/ui** + Tailwind for components and styling
- **Wagmi v2** + Viem for on-chain reads/writes via Privy wallets
- **Recharts** for funding spread and cumulative PnL charts
- **React Query** (via Wagmi) for data fetching with polling

Data flows:
- On-chain: vault contract reads via wagmi (`totalAssets`, `sharePrice`, `balanceOf`)
- Off-chain: Bot Status API at `/health`, `/status`, `/events`
- Contract config: `shared/contracts.json` (written by the contracts layer at deploy time)
- Fallback: deterministic demo data when bot/contract unavailable
