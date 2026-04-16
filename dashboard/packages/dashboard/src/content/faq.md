# FAQ
**Version 0.1 / Effective 2026-04-14**

> **Format note for dashboard integration:** This FAQ is structured into 7 categories with ~28 questions, designed for a tabbed UI (horizontal category tabs at the top, questions inline under each tab). Answer length target: 50-100 words each. The structure mirrors `liminal.money/faq` adapted for Dol's product surface.

---

## Category 1 — General

### What is Dol?

Dol is a token whose value slowly grows while you hold it. You buy one with USDC, you hold it, and when you want your money back, you cash out and get the current value. The growth comes from a strategy that runs underneath the protocol on Pacifica DEX. Three steps: Buy, Hold, Cash out.

### How does Dol grow in value?

The Dol pool runs a strategy that captures small market fees from perpetual futures contracts on real-world assets (oil, silver, gold, natural gas, copper). When the strategy makes money, the on-chain price of each Dol token slowly ticks up. Your Dol balance number stays the same — the *value* per Dol grows.

### What does "interest-bearing token" mean?

It means the token's value increases on its own over time, without you having to claim or stake anything. Different from a normal crypto token that just sits at whatever the market says it's worth — Dol's price grows based on what the underlying protocol earns. You do not need to take any action to receive the growth.

### Is Dol a stablecoin?

No. A stablecoin tries to track $1 with no movement. Dol tries to grow above $1 over time. The growth target is 5–7.5 percent per year, but it is not guaranteed and the value can also go down if the strategy loses money. Read the [Risk Disclosure](/legal/risk) before deciding.

---

## Category 2 — Buying and Holding

### How do I buy a Dol?

Sign in with Google or a wallet, confirm your country in the first-visit modal, get test USDC from a Base Sepolia faucet, and tap "Buy a Dol" on the homepage. The full step-by-step is at [How to buy a Dol](/docs/getting-started/how-to-buy).

### Why does my Dol balance never change?

Your Dol balance is a fixed number of tokens. What grows is the **value of each token**, measured in USDC. So 100 Dol stays 100 Dol forever, but its USDC value rises slowly. To see the current value, open the [My Dol](/my-dol) page.

### How fast does the value grow?

At a target of 7 percent per year, your value grows by roughly 0.02 percent per day. On a 100 Dol position, that is roughly 0.02 USDC per day or 7 USDC per year. Slow on a phone screen, real over a year. The actual growth is variable.

### Can the value go down?

Yes. If the strategy loses money in a given period, the per-Dol value drops. Dol holders absorb the strategy result directly — there is no separate buffer that absorbs losses for you. Read [Risks](/docs/trust/risks) for the categories and [Framework assumptions](/docs/trust/framework-assumptions) for the published Pacifica numbers behind each.

### Can I add more Dol later?

Yes. Each new purchase mints fresh Dol at the current price and combines with your existing position. You always end up holding pro-rata to your total deposited amount. There is no minimum or maximum — buy as little or as much as you want.

---

## Category 3 — Cashing Out

### What are the two cash-out options?

**Instant** — get your USDC in the same transaction. Pays a 0.05 percent fee. **Scheduled** — submit a request now, claim your USDC after a 30-minute cooldown. No fee. Pick whichever fits the moment. Both are non-custodial and burn your Dol on the way out.

### Why is there a fee on Instant?

Instant uses a buffer of available USDC that the protocol keeps ready for fast cash-outs. The 0.05 percent fee covers the cost of maintaining that buffer and goes to the Dol team for operations. Scheduled does not need the buffer (it pulls from the strategy side after the cooldown), so it is free.

### What is the cooldown for Scheduled?

Thirty minutes on the live deployment. The cooldown gives the protocol time to rotate funds out of the active strategy positions before paying you. After 30 minutes, come back to the [My Dol](/my-dol) page and tap Claim — your USDC arrives in the same transaction.

### Why did my Instant cash-out fail?

If your wallet shows a "transaction would fail" warning when you tap Instant, the most likely reason is the buffer is temporarily short of available USDC. Switch to Scheduled — it always works (subject to the 30-minute wait). The buffer refills automatically as new deposits come in.

### Can I cash out part of my Dol?

Yes. Enter any amount up to your full balance. The exchange is proportional — half your Dol gets you half the current USDC value, minus the fee if you used Instant. You can keep the rest, sell more later, or sell everything at any time.

---

## Category 4 — Safety and Trust

### Who holds my money?

You do. The Dol protocol is non-custodial — your wallet holds your Dol tokens and we never have access to your keys. The team cannot move your funds without you signing a transaction. The most we can do is block your access to the Dol website. Your tokens remain in your wallet either way.

### Can the team rug pull?

No, in the technical sense. The on-chain protocol cannot freeze, seize, or transfer user funds. Even if the website is taken offline, you can call the cash-out function directly from any wallet client and the protocol will pay you. The team can fail in other ways, but a classic rug pull is not in the threat model.

### Is the contract audited?

Internal verification only for Phase 1. The contracts pass 117 unit and fuzz tests, hit 100 percent line coverage on the main protocol, and clear Slither static analysis with zero high or medium findings. A formal third-party audit is planned before any mainnet launch. See [On-chain & verified](/docs/trust/on-chain) for details.

### Has Dol been hacked?

Not as of Phase 1 launch. If we ever experience an incident, we will publish it on the documentation site within 24 hours and notify all known wallet addresses through the on-chain notification mechanism. We will not hide a hack — that is not the kind of team we want to be.

### What happens if Pacifica DEX has a problem?

Pacifica is the venue where the Dol strategy runs. If Pacifica goes down, gets hacked, or changes their rules in a way that breaks the strategy, the Dol pool can lose value. The [Risk Disclosure](/legal/risk) section 4 covers this in detail. We chose Pacifica because of their track record, but no venue is risk-free.

---

## Category 5 — Where Dol Works

### Which countries can use Dol?

Vietnam, Turkey, Philippines, Mexico, and Argentina. That is the entire list for Phase 1. Dol's homepage detects your country and blocks access from anywhere else. See [Where Dol is available](/docs/getting-started/supported-countries) for the reasoning behind these five.

### Why only those five?

Because crypto law is different in every country and we are a small team without the budget to be compliant everywhere on day one. We picked five markets with high crypto adoption and clear legal posture, and we are starting there. As we grow, we will add more — each new country needs its own legal review.

### Can I use a VPN to bypass the geographic restriction?

You should not. The first-visit modal asks you to attest you are not using a VPN, and bypassing it puts you in violation of our [Terms of Service](/legal/terms). Section 11 of the Terms reserves the right to block any wallet identified as originating from a restricted jurisdiction. Please do not try.

### What if my country is added later?

Visit the Unavailable page from a blocked region and you can leave your email on the waitlist. We will notify you when (or if) Dol becomes available where you live. The waitlist is one-way notification only — we will not market other products to you and we will not share your email with anyone.

---

## Category 6 — Fees and Taxes

### What does Dol cost?

**Buying:** no fee. Exchange one USDC for one Dol at the moment of purchase. **Holding:** no fee while you hold. **Cashing out:** 0.05 percent on Instant, 0 percent on Scheduled. **Network fees:** standard Base Sepolia transaction fees (fractions of a cent on testnet) on every interaction.

### Are there hidden fees?

No. The 0.05 percent Instant fee is the only protocol-level fee. It is hardcoded in the on-chain protocol and visible to anyone who reads the contract source. The team does not take a management fee, performance fee, or success fee on the Dol product. Read [On-chain & verified](/docs/trust/on-chain) to verify yourself.

### Will Dol report my taxes?

No. Dol does not produce 1099 forms, K-1s, or any tax documentation. You are solely responsible for tracking your buys, sells, and value changes and reporting them to your local tax authority. Most countries treat crypto gains and losses as taxable events. Talk to a local accountant if you are unsure.

### Can I deposit fiat instead of crypto?

Not in Phase 1. Phase 1 requires test USDC on Base Sepolia. A fiat onramp via Stripe Crypto Onramp (Apple Pay or bank card → USDC → Dol in one tap) is planned for Phase 3. We will announce it on the homepage and the documentation site when it is live.

---

## Category 7 — Support

### Where can I get help?

For general questions, [the FAQ](/faq) covers most things and the [docs](/docs) cover the rest. For things not in either, email **support@dol.app** — we read everything. We do not run a Discord, Telegram, or live chat in Phase 1 because we are a small team and would rather respond slowly and well than fake 24/7 coverage.

### I think I found a bug

Send the details to **support@dol.app**: what you were trying to do, what you expected, what actually happened, a screenshot if possible, and your wallet address (the public one only — never share your seed phrase). We respond to bug reports within two business days.

### I think I found a security issue

Please report it **privately**, not publicly. Email **security@dol.app** with as much technical detail as you can. We do not have a paid bug bounty program in Phase 1, but we will publicly credit responsible disclosures and we will do our best to compensate severe findings. We aim to respond within 24 hours.

---

## Category metadata (for dashboard tab generation)

```ts
const FAQ_CATEGORIES = [
  { id: "general", label: "General", count: 4 },
  { id: "buying", label: "Buying & Holding", count: 5 },
  { id: "cashout", label: "Cashing Out", count: 5 },
  { id: "safety", label: "Safety & Trust", count: 5 },
  { id: "where", label: "Where Dol Works", count: 4 },
  { id: "fees", label: "Fees & Taxes", count: 4 },
  { id: "support", label: "Support", count: 3 },
];
// Total: 30 questions, ~2400 words
```

The "All" tab (Liminal pattern) shows every question in order, ungrouped.

---

*This FAQ was prepared for Phase 1 Dol launch. It pairs with the [Risk Disclosure](/legal/risk) and [Terms of Service](/legal/terms) — when in doubt, the legal documents are authoritative.*
