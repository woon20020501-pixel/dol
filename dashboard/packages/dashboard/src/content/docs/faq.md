# FAQ

The questions people actually ask. If yours is not here, ping us on [Support](/docs/more/support).

## The basics

### What is Dol in one sentence?

A crypto token whose value slowly grows while you hold it, because the Dol protocol runs a market strategy underneath that captures small fees, and the gains flow back to everyone holding the token.

### Is Dol a bank?

No. Dol is a crypto asset. It is not FDIC, KDIC, or government insured. The value can go down. Do not put money in Dol that you cannot afford to lose.

### Is the 7.5 percent guaranteed?

No. 5 to 7.5 percent is a target return based on what the strategy has done historically. The actual return is variable — it can be higher, lower, zero, or negative. The number is a target, not a promise.

### Where is Dol available?

Vietnam, Turkey, Philippines, Mexico, and Argentina. That is the entire list for Phase 1. See [Where Dol is available](/docs/getting-started/supported-countries) for why we picked these five.

### Is Dol live with real money?

Not yet. Phase 1 is on Base Sepolia testnet using test USDC. The test USDC has no real-world value. A real-money mainnet launch is planned for a future phase, and we will tell you well before it happens.

## How buying and holding work

### How do I buy a Dol?

Sign in with Google or a wallet, confirm your country, get some test USDC from a faucet, and click Buy a Dol on the homepage. The full step-by-step is at [How to buy a Dol](/docs/getting-started/how-to-buy).

### What is the exchange rate?

One USDC for one Dol at the moment of purchase. After you buy, the USDC value of each Dol slowly grows over time as the protocol generates returns. So your 100 Dol stays 100 Dol forever, but its USDC value rises.

### Will I see my Dol balance go up?

No. Your Dol balance number is fixed once you buy. What changes is the *value* of each Dol, measured in USDC. To see the live value, visit the [My Dol](/my-dol) page after you have bought some.

### How fast does the value grow?

At a target of 7 percent per year, your value grows by roughly 0.02 percent per day, or about 0.0007 percent per hour. On a 100 Dol position, that is roughly 0.02 USDC per day. Slow on a phone screen, real over a year.

### Can the value go down?

Yes. If the Dol strategy loses money in a given period, the value per Dol goes down. Dol holders absorb the strategy result directly — there is no separate buffer that absorbs losses for you. See [Risks](/docs/trust/risks) for the categories and [Framework assumptions](/docs/trust/framework-assumptions) for the published Pacifica numbers behind each risk.

## How cashing out works

### How do I cash out?

Go to the [My Dol](/my-dol) page, tap the Cash out button, and pick one of two options:

- **Instant** — get your USDC in the same transaction. 0.05 percent fee.
- **Scheduled** — wait 30 minutes, claim your USDC. No fee.

You can use either one every time. Pick whichever fits the moment.

### Why is there a fee on Instant?

The Instant path uses a liquid buffer that the protocol keeps ready for fast cash-outs. The 0.05 percent fee covers the cost of maintaining that buffer and goes to the Dol team for operations. The Scheduled path does not need the buffer, so it is free.

### What is the cooldown for?

When you submit a Scheduled cash-out, the protocol needs a few minutes to rotate funds out of the active strategy positions. The 30-minute cooldown gives it that window. After 30 minutes you come back and claim your USDC.

### Can Instant fail?

Yes. If a lot of people cash out instantly at the same time, the buffer can drain and Instant will fail with an "Insufficient liquidity" error. When that happens, switch to Scheduled — that path always works (subject to the 30-minute wait and extreme stress conditions noted in the [Risk Disclosure](/legal/risk)).

### Can I cash out part of my position?

Yes. Enter any amount up to your full balance. The exchange is proportional.

## Safety and trust

### Who holds my money?

You do. The Dol protocol is non-custodial. Your wallet holds your Dol tokens. We never have your keys and we cannot move your funds without you signing a transaction. The most we can do is block your access to the website frontend.

### Can the team rug pull?

No, in the technical sense. The contracts are non-custodial and the team cannot freeze, seize, or move user funds. The team can shut down the frontend website, but your Dol tokens remain in your wallet and you can call the cash-out function directly from any wallet client even if the website is gone.

The team can fail in other ways — bad strategy decisions, slow incident response, regulatory shutdown — but rug pull in the classic sense is not in the threat model.

### Is the contract audited?

Internal verification only for Phase 1. The contracts pass 117 tests, hit 100 percent line coverage on the vault, and clear Slither static analysis with zero high or medium findings. A formal third-party audit is planned before any mainnet launch. See [On-chain & verified](/docs/trust/on-chain) for the details.

### Has Dol been hacked?

Not as of Phase 1 launch. We will publish any incident on the documentation page and notify all known wallets directly. We will not hide a hack.

### What happens if Pacifica DEX has a problem?

Pacifica is the venue where the Dol strategy runs. If Pacifica goes down, gets hacked, or changes their rules in a way that breaks the strategy, the Dol pool can lose value. The [Risk Disclosure](/legal/risk) Section 4 covers this in detail.

## Money and taxes

### What does Dol cost?

- **Buying:** no fee. Exchange one USDC for one Dol.
- **Holding:** no fee. The protocol takes nothing while you hold.
- **Cashing out:** 0.05 percent on Instant, 0 percent on Scheduled.
- **Network gas:** standard Base Sepolia network fees on every transaction. Fractions of a cent on testnet.

### Will Dol report my taxes?

No. Dol does not produce 1099s, K-1s, or any tax documentation. You are responsible for tracking your own buys, sells, and value changes and reporting them in your country.

### Can I deposit fiat instead of crypto?

Not in Phase 1. Phase 3 (planned for late April) will add a fiat onramp via Stripe Crypto Onramp so you can buy Dol with Apple Pay or a bank card. For now, you need test USDC on Base Sepolia.

## The five-country thing

### Why only Vietnam, Turkey, Philippines, Mexico, and Argentina?

Crypto law is different in every country. We are a small team and we cannot afford legal compliance in every market. So we picked five with high crypto adoption and clear legal posture, and we are starting there. As we grow, we will add more — see [Where Dol is available](/docs/getting-started/supported-countries) for the full reasoning.

### Can I use a VPN?

You should not. The first-visit modal asks you to attest that you are not using a VPN, and bypassing the geographic restrictions puts you in violation of the [Terms of Service](/legal/terms). We do not currently have automatic VPN detection, but Section 11 of the Terms reserves the right to block any wallet identified as originating from a restricted jurisdiction.

### What if my country is added later?

Visit the Unavailable page and join the waitlist. We will notify you. We will not market other products to you and we will not share your email.

## Still have questions?

Ping us on [Support](/docs/more/support). The fastest way is the channel listed there.
