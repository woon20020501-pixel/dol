# Dol Dashboard

The retail UI and operator dashboard for **Dol** — a delta-neutral cross-venue funding-rate harvester on Pacifica.

## Layout

```
packages/
  dashboard/           Next.js 14 app
    src/app/           Routes (landing, deposit, my-dol, dashboard, docs, legal, faq)
    src/components/    Reusable UI (DepositCard, WithdrawCard, NavChart, etc.)
    src/hooks/         wagmi hooks (useDeposit, useWithdraw, useVaultReads, useAuroraTelemetry)
    src/lib/           Formatters, config, ABI bindings
    src/content/       Markdown content (docs, FAQ, legal)
```

## Routes

| Path | Purpose |
|---|---|
| `/` | Landing — product hero, live vault ticker, deposit CTA |
| `/deposit` | Three-step deposit flow — connect → approve USDC → deposit → receive DOL |
| `/my-dol` | User balance — principal, earned yield, withdraw button |
| `/dashboard` | Operator view — live NAV chart across 10 symbols, bot health, LIVE/STALE/SIM badge |
| `/docs/*` | Product docs, strategy architecture, FAQ |
| `/legal/{terms,privacy,risk}` | Legal pages |

## Key integrations

- **[Privy](https://privy.io)** — wallet authentication (email, Google, or external wallet)
- **[wagmi](https://wagmi.sh)** — contract reads and transactions
- **Base Sepolia** — target chain for testnet deployment
- **`/api/nav`** — server-side reader for the bot's `nav.jsonl` stream (polled every 2s, 30s staleness threshold, automatic fallback to a deterministic client simulator if the file is absent)

## Running locally

```bash
pnpm install
cp packages/dashboard/.env.example packages/dashboard/.env.local  # fill in values
pnpm --filter dashboard dev
```

Then open http://localhost:3000.

Required env vars (see `.env.example`):

- `NEXT_PUBLIC_PRIVY_APP_ID` — your Privy app ID
- `NEXT_PUBLIC_RPC_URL` — Base Sepolia RPC
- `NEXT_PUBLIC_BOT_API_URL` — URL of the bot HTTP surface (defaults to `http://localhost:7777`)
- `NEXT_PUBLIC_DEMO_MODE` — `true` to disable live transactions and show the deterministic simulator
- `NAV_JSONL_PATH` — absolute path to the bot's `nav.jsonl` (optional; a sensible relative default is used when the dashboard runs inside the monorepo)
