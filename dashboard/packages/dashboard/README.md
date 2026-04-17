# packages/dashboard

Next.js 14 App Router application. This is the only package in the `dashboard/` pnpm workspace.

## Running

```bash
# From the monorepo root
pnpm install

# Required env
cp packages/dashboard/.env.example packages/dashboard/.env.local
# fill NEXT_PUBLIC_PRIVY_APP_ID (the only required var)

# Dev server
pnpm --filter dashboard dev
# http://localhost:3000
```

## Environment variables

| Variable | Required | Default | Description |
|---|---|---|---|
| `NEXT_PUBLIC_PRIVY_APP_ID` | Yes | — | Privy app ID for wallet + email + social login |
| `NEXT_PUBLIC_RPC_URL` | No | `https://sepolia.base.org` | RPC endpoint for on-chain reads |
| `NEXT_PUBLIC_CHAIN_ID` | No | `84532` | Target chain (Base Sepolia) |
| `NEXT_PUBLIC_DEMO_MODE` | No | `false` | Force the deterministic client simulator; disable tx buttons |
| `NEXT_PUBLIC_BOT_API_URL` | No | `http://localhost:7777` | Optional bot HTTP surface (`/health`, `/status`, `/events`) |
| `NAV_JSONL_PATH` | No | monorepo-relative | Path for `/api/nav` reader (overrides the walk-up default) |
| `SIGNALS_ROOT` | No | monorepo-relative | Same idea for `/api/signal` |

## Data pipeline (`/api/nav`, `/api/signal`)

Both server routes read files from the sibling `bot-rs` package output directory and expose a JSON HTTP surface the client polls. They have identical structure:

1. **Tier 1 — live bot output**: walk up three directories from `process.cwd()` (`dol-public/dashboard/packages/dashboard` → `dol-public/`), then read `bot-rs/output/demo_smoke/...`. Returns `source: "live"` and flips `is_stale: true` when the file `mtime` is more than 30 s old.
2. **Tier 2 — bundled snapshot**: if the live tier fails, read `public/demo/nav.jsonl` (or `public/demo/signals.json`). Returns `source: "snapshot"` and never flags stale (the bundle is intentionally frozen, not a dead bot).
3. **Tier 3 — simulator**: if both file reads fail, return `{ ok: false, fallback_to_simulator: true }` and the client-side `useAuroraTelemetry` hook switches to a deterministic-ambient SIM mode.

### `/api/nav` query parameters

- `?since_ms=<ts>` — return only rows with `ts_ms > since`
- `?tail=<n>` — return only the last `n` rows

### `/api/signal`

- `?symbol=<SYM>` — return latest signal JSON for a single symbol
- No query — return latest per-symbol snapshot across `BTC, ETH, SOL, BNB, ARB, AVAX, SUI, XAU, XAG, PAXG`

## Routes

| Path | Purpose |
|---|---|
| `/` | Landing — product hero, live vault ticker, deposit CTA |
| `/deposit` | Connect → approve USDC → deposit → receive Dol |
| `/my-dol` | Holder balance, pending withdraws, cooldown countdown |
| `/dashboard` | Aurora Console — multi-symbol NAV chart, venue health, NAV reporter status |
| `/docs/*` | Trust-layer docs (strategy paper, architecture, framework assumptions) |
| `/faq` | Tabbed FAQ (7 categories, ~28 questions) |
| `/legal/{terms,privacy,risk}` | Legal pages rendered from `src/content/legal/*.md` |
| `/unavailable` | Geo-gate fallback (email capture) |

`middleware.ts` gates the entire site: requests from blocked regions (KR, US) are redirected to `/unavailable` before the Next.js handler runs.

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
            │                               │
            └──────────────┬────────────────┘
                           ▼
                     total APY
                  delta-neutral, hedged
```

- **Funding alpha** from the perp leg only. Same symbol, opposite sides on two venues; funding-rate spread is the only revenue source. Net price exposure ≈ 0.
- **Treasury yield** is the conservative base layer. 30% of TVL is supplied to a Moonwell-style permissionless lending market.
- **Total APY** is the share-weighted sum. The landing hero and the "Yield Sources" panel break it down.

Live allocation is read from the vault config (`shared/contracts.json` → `allocation.{treasuryBps,marginBps}`) and the live treasury balance from the configured `treasuryVault` address. If those fields are missing the UI falls back to a 70/30 synthetic split.

## NAV reporter card

Vault NAV is not implicit from on-chain reads — perp positions live off-chain. The NAV reporter is an operator-run process that every ~5 minutes fetches every leg's mark, computes total NAV, and submits it on-chain via `vault.reportNAV(value, signature)`. The vault contract enforces a **±10% sanity guard** at write time.

The dashboard surfaces this as the NAV Reporter card in the right sidebar: operator address, last report relative time, last NAV in USD, last tx hash (linked to BaseScan), live countdown to the next scheduled report, and a status pill (live / dry-run / error / idle) in the card header. A colored hint appears in the page header if the reporter is erroring or has not yet posted.

## Architecture

- **Next.js 14** App Router + TypeScript strict
- **shadcn/ui** + Tailwind CSS with utility classes
- **wagmi v2** + viem for typed on-chain reads and writes
- **Privy** for wallet auth (email / Google / external wallet)
- **Recharts** for time-series charts (NAV, funding spread, cumulative PnL)
- **React Query** (via wagmi) for data fetching with polling

## Testing

Vitest tests exist in-tree:

```
src/lib/{format,env,errors,txState,constants}.test.ts
src/hooks/{useDeposit,useVaultReads}.test.ts
```

The test runner is not wired into `package.json` in this repo. Run manually:

```bash
npx vitest run --root packages/dashboard
```

## Build + deploy

```bash
pnpm --filter dashboard build     # Production build
pnpm --filter dashboard lint      # ESLint
```

Deploy to Vercel: set **Root Directory** to `packages/dashboard`, **Framework** to Next.js, and add the env vars listed above. The build emits `.next/`.
