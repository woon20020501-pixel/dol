# dashboard

Next.js 14 retail + operator UI for Dol — the consumer yield product on Pacifica.

## Purpose

Single web surface for three consumers:

1. **Depositors** — wallet connect, approve USDC, deposit, receive vault shares.
2. **Shareholders** — monitor balance, pending withdraws, cooldown countdown, share price history.
3. **Operators** — live NAV chart across all active symbols, LIVE/STALE/SIM status pill, per-venue health, NAV reporter countdown and signer address.

The app is read-mostly: all state-changing paths go through the vault contract via wagmi + viem; no backend service holds user funds or signs transactions on behalf of users.

## Layout

```
packages/dashboard/
├── src/
│   ├── app/                  Next.js App Router
│   │   ├── page.tsx          landing
│   │   ├── deposit/          3-step deposit flow
│   │   ├── my-dol/           holder balance + withdraw
│   │   ├── dashboard/        operator view (Aurora Console)
│   │   ├── docs/             markdown-rendered product docs
│   │   ├── legal/            terms, privacy, risk disclosure
│   │   ├── faq/              FAQ tabs
│   │   ├── unavailable/      geo-blocked fallback
│   │   └── api/
│   │       ├── nav/route.ts      reads bot-rs nav.jsonl, 2s polling target
│   │       └── signal/route.ts   reads bot-rs per-symbol signal JSON tree
│   ├── components/           UI (shadcn + custom)
│   │   ├── aurora/               AuroraConsole, MultiSymbolNavPanel, ambient spotlight
│   │   ├── vault/                DepositCard, WithdrawCard, AllocationBar, NavReporterCard
│   │   ├── positions/            PositionTable
│   │   ├── hero/                 StatusPill, live vault ticker
│   │   ├── common/               banners, error fallback
│   │   └── ui/                   shadcn primitives
│   ├── hooks/                wagmi + React Query hooks
│   ├── lib/                  formatters, constants, env, ABI bindings, logger
│   ├── content/              markdown bundled at build time
│   │   ├── docs/trust/           strategy-paper, architecture, assumptions
│   │   ├── legal/                risk, terms, privacy
│   │   └── faq.md                landing FAQ
│   └── abi/                  generated ABI fragments
├── public/
│   ├── demo/nav.jsonl        bundled snapshot — production fallback when live bot unreachable
│   └── demo/signals.json     same idea for per-symbol signals
├── middleware.ts             geo-gate + robots
├── tailwind.config.ts
└── next.config.mjs
```

Top-level `packages/dashboard/README.md` has additional detail on env vars, the yield architecture, and the NAV reporter card.

## Key interfaces

### Data hooks (`src/hooks/`)

| Hook | Purpose | Contract surface |
|---|---|---|
| `useVaultReads` | Live `totalAssets`, `totalSupply`, `sharePrice`, `paused`, `navLastReportedAt`, `navLastReportedValue` | `PacificaCarryVault` |
| `useDeposit` | 3-state (approve → deposit) with tx tracking | ERC-20 `approve` + `deposit` |
| `useWithdraw` / `useDolWithdraw` | `requestWithdraw` + cooldown timer + `claimWithdraw` | `requestWithdraw`, `claimWithdraw`, `instantWithdraw` |
| `useDolBalance` | Share balance + share price → USD value | ERC-20 `balanceOf` + `convertToAssets` |
| `useNavReporter` | Last report time, signer address, next-scheduled-report countdown | event log + view reads |
| `useTxHistory` | On-chain event enumeration scoped to the connected address | event filter |
| `useTranche` | Phase-1 read of Dol token (junior inactive) | `Dol` ERC-20 |
| `useBotStatus` / `useBotHealth` / `useBotEvents` | Optional `/health`, `/status`, `/events` polls against the bot HTTP surface | off-chain |
| `useAuroraTelemetry` | Orchestrates `/api/nav` + `/api/signal`, falls back to a deterministic client-side simulator when both are unreachable | reads bot output files via server route |

### API routes (`src/app/api/`)

| Route | Reads | Stale threshold | Fallback |
|---|---|---|---|
| `GET /api/nav` | `bot-rs/output/demo_smoke/nav.jsonl` (or `NAV_JSONL_PATH`) | 30s mtime | `public/demo/nav.jsonl` bundled snapshot, then `{ fallback_to_simulator: true }` |
| `GET /api/signal` | `bot-rs/output/demo_smoke/signals/{SYMBOL}/{yyyymmdd}/{ts}.json` (latest per symbol) | 30s mtime | `public/demo/signals.json` bundled snapshot |

Query parameters: `?since_ms=<number>` (delta tail) and `?tail=<n>` on `/api/nav` for incremental streaming.

## Dependencies

| Package | Purpose |
|---|---|
| `next@14.2.35` | App Router, RSC, API routes |
| `@privy-io/react-auth` + `@privy-io/wagmi` | Wallet auth (email, Google, external wallet) |
| `wagmi` | Typed contract reads/writes with React Query cache |
| `viem` | RPC transport, ABI encoding |
| `@tanstack/react-query` | Data fetching, polling, retries |
| `recharts` | NAV chart, funding spread chart |
| `framer-motion` | Motion primitives for ambient spotlight and the Aurora Console |
| `shadcn` + `tailwindcss` + `tailwind-merge` | Design system + utility CSS |
| `sonner` | Toast notifications on tx state transitions |

## Testing

The package contains vitest files for the critical logic:

| File | Coverage |
|---|---|
| `src/lib/format.test.ts` | `formatUsd`, `formatUsdCompact`, `formatPct`, `formatBps`, `formatSharePrice`, `pnlColor` |
| `src/lib/env.test.ts` | env-var parsing, missing-var fallbacks, validation |
| `src/lib/errors.test.ts` | error classification (user-rejected, insufficient-gas, chain-mismatch, contract-revert) |
| `src/lib/txState.test.ts` | idle → signing → pending → confirmed → error state machine |
| `src/lib/constants.test.ts` | constant validity (address checksum, bps in `[0, 10000]`) |
| `src/hooks/useDeposit.test.ts` | approve→deposit flow with mocked wagmi |
| `src/hooks/useVaultReads.test.ts` | `totalAssets`/`sharePrice` read composition |

Test runner is not wired into `package.json` in this monorepo (no `test` script); run ad hoc via `npx vitest run` once vitest is installed at the workspace level.

## Integration points

- **Contracts** — reads `shared/contracts.json` at build time (emitted by the contracts `Deploy.s.sol` ffi helper) to get `PacificaCarryVault`, `Dol`, and `USDC` addresses plus the target chain ID.
- **Bot** — reads `nav.jsonl` and the per-symbol signal JSON tree via the server routes. No direct bot HTTP dependency in the common path; the optional `useBotStatus`/`useBotEvents` hooks consume `/health`, `/status`, `/events` if `NEXT_PUBLIC_BOT_API_URL` is set.
- **Strategy research** — none at runtime. The spec lives in `strategy/docs/` for humans.

## Configuration

| env | Required | Default | Use |
|---|---|---|---|
| `NEXT_PUBLIC_PRIVY_APP_ID` | Yes | — | Privy auth |
| `NEXT_PUBLIC_RPC_URL` | No | `https://sepolia.base.org` | RPC transport |
| `NEXT_PUBLIC_CHAIN_ID` | No | `84532` | Base Sepolia |
| `NEXT_PUBLIC_DEMO_MODE` | No | `false` | Force the deterministic simulator, disable tx buttons |
| `NEXT_PUBLIC_BOT_API_URL` | No | `http://localhost:7777` | Optional bot HTTP surface |
| `NAV_JSONL_PATH` | No | `../../../bot-rs/output/demo_smoke/nav.jsonl` | Absolute or monorepo-relative path for `/api/nav` |
| `SIGNALS_ROOT` | No | `../../../bot-rs/output/demo_smoke/signals` | Same idea for `/api/signal` |

## Running locally

```bash
pnpm install
cp packages/dashboard/.env.example packages/dashboard/.env.local  # fill in NEXT_PUBLIC_PRIVY_APP_ID
pnpm --filter dashboard dev     # http://localhost:3000
```

## Ship status

| Surface | Status |
|---|---|
| Deposit flow | Live on Base Sepolia |
| Request/claim withdraw | Live; cooldown clamp matches contract |
| Instant withdraw | Live — 50% idle-liquidity policy enforced by the contract |
| Aurora Console | Consumes live bot output when reachable; falls back to bundled snapshot then deterministic simulator |
| NAV Reporter card | Live — reads operator address + `NavReported` event log |
| Geo-gate middleware | Active — blocks KR/US (see `middleware.ts`) |
| Farcaster mini-app | Dependency present; surface not yet wired |
| E2E Playwright tests | Not wired in this monorepo (scripts absent) |
