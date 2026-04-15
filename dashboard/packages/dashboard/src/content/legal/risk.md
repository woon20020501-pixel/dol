# Risk Disclosure
**Version 0.2 / Effective 2026-04-14**

> **What changed from v0.1:** Added new Section 3.5 (Oracle and Pricing Risk) covering oracle accuracy, value drift, and de-pegging risk between Dol and the Vault's underlying NAV. All v0.1 risk categories retained.

**Issuer:** Dol Labs (Cayman Islands entity pending)
**Protocol Deployment:** Base Sepolia Testnet (Phase 1)

> **IMPORTANT:** YOU MUST READ AND ACKNOWLEDGE THIS DISCLOSURE. BY INTERACTING WITH THE DOL PROTOCOL, YOU EXPLICITLY CONFIRM THAT YOU HAVE READ, UNDERSTAND, AND ACCEPT ALL RISKS DESCRIBED HEREIN, INCLUDING THE RISK OF LOSING ALL DEPOSITED FUNDS.

---

## 1. Dol is a Crypto Asset, NOT a Bank Deposit
Dol is a digital token representing a share in an on-chain protocol. It is **not** a bank account, deposit, checking account, or savings account. Dol is **not** offered by a bank or a regulated financial institution. Your Dol tokens and the funds used to acquire them are **not protected** by any government deposit insurance scheme. This includes, but is not limited to, the U.S. FDIC, Korea's KDIC, the UK's FSCS, Canada's CDIC, or any other national deposit guarantee program. There is **no government guarantee** that you will get your money back. The value of your Dol tokens can go to zero, resulting in a total loss of your deposited capital.

## 2. Value Volatility and No Return Guarantee
Dol offers a **target return** based on the historical performance of its underlying strategy. This target (5–7.5%) is an estimate, **not a promise, guarantee, or fixed interest rate**. The actual return you experience will be variable and "pass-through," meaning it directly reflects the net performance of the protocol's strategy, whether positive or negative.
- The return can be **lower than the target**, **zero**, or **negative**.
- A negative return means the value of your Dol tokens will decrease relative to your deposit.
- Past performance, including any 90-day backtest data, is **not a reliable indicator** of future results. Financial markets are unpredictable, and historical gains do not guarantee future profits.

## 3. On-Chain Protocol Risk
Dol operates through non-custodial smart contracts (an ERC-4626 vault) deployed on a blockchain. This technology carries inherent risks:
- **Bugs and Exploits:** The protocol's code may contain bugs, vulnerabilities, or logical errors that could be exploited by malicious actors, leading to a partial or total loss of user funds.
- **Audits Are Not Guarantees:** While the protocol may undergo security audits, audits **do not eliminate risk**. They are a snapshot review that cannot guarantee the absence of all vulnerabilities.
- **Irreversible Loss:** Transactions on-chain are typically irreversible. If funds are lost due to a protocol bug or exploit, there is **no recourse** to recover them.
- **Upgradability:** The protocol contracts may be upgradeable by the team. While intended for improvements, any upgrade carries its own risk of introducing new bugs or unintended consequences.

## 3.5. Oracle, Pricing, and Value Drift Risk
The value of your Dol token at any given moment is computed from on-chain reads against the Vault's underlying assets, and the Vault's strategy depends on price oracles for the perpetual futures positions and spot reference assets it trades. This pricing layer is a separate source of risk from the protocol code itself.

- **Oracle accuracy.** The Vault relies on price oracles to mark its positions, compute its net asset value (NAV), and decide when to enter or exit trades. Oracle providers can publish stale data, manipulated data, or simply incorrect data due to bugs or upstream venue problems. When this happens, the Vault may price your deposit incorrectly, return the wrong amount on withdrawal, or temporarily refuse to transact at all.
- **Value drift / de-pegging.** Although Dol's pricePerShare is computed deterministically from on-chain reads of the Vault's holdings, the relationship between the marketed "target return" (5–7.5% annualized) and the value you actually realize on cash-out can diverge in either direction. During periods of strategy underperformance, oracle anomalies, NAV reporting interruptions, or operator inactivity, the realized value of your Dol may be materially below the headline target. Dol Labs makes no guarantee that any user transacting at any moment receives a price that reflects the marketed target.
- **NAV reporting dependence.** The Vault's reported NAV is updated by an off-chain operator. If the operator is offline, slow, or compromised, the on-chain NAV used to compute pricePerShare can become stale. Withdrawals during stale-NAV periods may execute at incorrect prices.
- **No recourse for oracle losses.** If you lose value due to an oracle malfunction, manipulation, or stale data, Dol Labs cannot reverse the loss. Oracles are external dependencies and outside Dol Labs's direct control.

## 4. Strategy Risk (Underlying Trading Activity)
**This is a critical risk.** Dol's value is derived from an underlying vault that engages in active trading. The vault's strategy is not passive or risk-free.
- **What the Vault Does:** The vault trades perpetual futures contracts on Real-World Asset (RWA) commodities (like crude oil, silver, gold, natural gas, copper) and foreign exchange (FX) pairs on the Pacifica decentralized exchange (DEX).
- **The Source of Return:** The strategy aims to capture "funding rate" mispricings between these perpetual contracts and their underlying spot markets. This is **not** simple directional speculation on price movements.
- **Sources of Loss:** The strategy can and will lose money under certain market conditions, including but not limited to:
  - Sudden changes in market structure or regime.
  - Technical failures or outages on the Pacifica DEX or its oracle providers.
  - Unpredictable decoupling between the perpetual futures price and the actual RWA spot price (basis risk).
  - The 90-day historical dataset used for strategy calibration is **shorter than a full financial market cycle** and may not account for all possible adverse scenarios.

## 5. Regulatory Status — Commodity Pool
Because the Vault trades derivative contracts on commodities, under the laws of certain jurisdictions (including the United States Commodity Exchange Act) this may classify the Vault as a "commodity pool" and Dol Labs as a "Commodity Pool Operator" (CPO). **Dol Labs is NOT registered with the U.S. Commodity Futures Trading Commission (CFTC) as a CPO, nor with any equivalent regulator in any jurisdiction.** No regulatory agency has reviewed or approved the Vault's strategy, assets, or risk disclosures. You are not afforded the protections of the Commodity Exchange Act or any equivalent framework. The Service's geographic restrictions (see Section 8) are specifically designed to avoid targeting markets where such registration would be required.

## 6. Junior Tranche Dependence (Critical Structural Disclosure)
Dol is architecturally designed as the **senior position** in a two-tier (senior/junior) vault structure. This structure is intended to provide a buffer where the junior tranche absorbs initial losses, protecting the senior tranche (Dol).

**PHASE 1 REALITY:** For Phase 1 on Base Sepolia testnet, the **junior tranche is DEACTIVATED at the on-chain smart contract level**.

**Consequence:** In Phase 1, Dol holders receive a **direct, unfiltered pass-through** of the vault strategy's net return.
- If the strategy return is **positive**, Dol holders capture all of it.
- If the strategy return is **negative**, Dol holders **absorb 100% of the losses**. There is **no junior tranche buffer** to protect you.

**Future Changes:** The junior tranche may be reactivated in a future phase. Any such material change to the risk structure will be communicated to users, but does not retroactively protect Phase 1 deposits.

## 7. Liquidity and Withdrawal Risk
Dol offers two withdrawal methods, both subject to limitations:
- **Instant Withdrawal:** This is served from a dedicated liquid buffer (targeting 50% of deposits). If withdrawal demand exceeds the available buffer, Instant withdrawals will fail with an "InsufficientLiquidity" error. You will then need to use the Scheduled path.
- **Scheduled Withdrawal:** This has a 30-minute cooldown period and processes via a request/claim queue. While designed to be reliable, under extreme network congestion or protocol stress, even Scheduled withdrawals could be delayed.
- **No Guarantee of Exit:** You cannot assume you will be able to withdraw your funds immediately or at all at any specific time. In a severe market event or "bank run" scenario, withdrawal functionality may be severely impaired.

## 8. Regulatory Risk
The legal status of crypto assets and yield-generating protocols is uncertain, varies widely, and is rapidly evolving.
- **Jurisdictional Restrictions:** In Phase 1, Dol's frontend is intentionally geo-blocked and available **only** to users physically located in Vietnam, Turkey, the Philippines, Mexico, and Argentina. Access from all other countries is blocked.
- **VPN Use is a Legal Risk:** If you use a VPN, fake GPS, or other method to circumvent these geo-blocks from a restricted jurisdiction (e.g., the U.S., EU, UK, Korea, Canada, Australia, Singapore, Hong Kong, Japan, China, or any other country not on the whitelist), you do so **at your own legal risk**. You are solely responsible for determining the legality of your access and use.
- **Regulatory Action:** Dol or its operators may be subject to investigation, enforcement action, or legal orders from regulators in any jurisdiction. This could result in the protocol being shut down, blocked, or forced to freeze operations in certain regions without notice.
- **Future Changes:** New laws or regulations could render the continued operation of Dol infeasible, potentially affecting service availability, withdrawal functionality, or the value of the Dol token itself.

## 9. Force Majeure and Protocol Events
Dol Labs is not responsible for any failure, suspension, or loss arising from events beyond its reasonable control, including but not limited to: failure or forking of the Base Sepolia blockchain; failure, security breach, or insolvency of the Pacifica DEX or any integrated third-party protocol; failure of Privy, Vercel, Cloudflare, or any other infrastructure provider; government or regulatory action that restricts the Service; or any act of God, natural disaster, war, pandemic, or other force majeure event. In any such event, Dol Labs may suspend, modify, or terminate the Service at its sole discretion without liability.

## 10. Operator and Governance Risk
- **Small Team:** The Dol protocol is initially developed and operated by a small, pre-entity team. You are relying on their continued good faith, operational competence, and financial resources.
- **Centralized Decisions:** In Phase 1, key decisions regarding strategy parameters, deployment, upgrades, and treasury management are made by this team. There is no decentralized governance token or on-chain voting at launch.
- **No Formal Entity at Launch:** At the start of Phase 1, the formal legal entity (Dol Labs, Cayman Islands) is pending establishment. Operational risks are heightened during this period.
- **No Fiduciary Duty:** Dol Labs does not owe you any fiduciary duties. Your relationship is strictly contractual and non-custodial. No financial, investment, tax, or legal advice is provided by Dol Labs through the Service.

## 11. Tax Risk
You are solely and entirely responsible for your own tax compliance.
- **Taxable Events:** Acquiring, holding, earning returns from, selling, or swapping Dol tokens may create taxable events (capital gains, income, etc.) in your jurisdiction.
- **No Reporting:** Dol does not and will not provide any tax reporting, documentation (such as IRS Form 1099 in the U.S.), or advice.
- **Your Responsibility:** You must track all your transactions, calculate your gains or losses, and report and pay any taxes owed to the relevant authorities. Failure to do so may result in penalties, interest, or legal action against you.

## 12. Technology and Self-Custody Risk
You interact with Dol using your own non-custodial cryptocurrency wallet (e.g., via Privy, MetaMask, or similar). This places critical security responsibilities on you.
- **Irreversible Loss:** If you lose your private keys, seed phrase, or wallet access, **your funds are permanently and irreversibly lost**. Dol cannot recover them.
- **No Reversals:** If you send funds to the wrong address or are victimized by a phishing scam, the transactions **cannot be reversed or cancelled** by Dol or anyone else.
- **Testnet vs. Mainnet:** Phase 1 operates on the **Base Sepolia testnet**. The assets used have **no real-world monetary value**. Real financial risk will only begin upon a future mainnet launch (date TBD). Do not conflate testnet success with mainnet safety.

## 13. Total Loss Acknowledgment
**THIS IS THE MOST IMPORTANT RISK. YOU MUST ACKNOWLEDGE IT.**
- You acknowledge and accept that it is **possible to lose 100% of the funds you deposit** into the Dol protocol.
- You should **only deposit funds that you can afford to lose entirely**.
- Dol is **not suitable** for you if:
  - You need guaranteed returns or protection of your principal.
  - You are seeking a substitute for a regulated savings account, money market fund, or certificate of deposit.
  - You do not have a high risk tolerance or the financial capacity to absorb a total loss.
  - You are relying on these funds for short-term needs, emergencies, or retirement.

**By proceeding, you certify that you are not such a person and that you fully accept the risk of total financial loss.**

---

*This Risk Disclosure document was prepared with the assistance of an AI legal research tool. It is not legal advice. Dol Labs and its operators strongly recommend that you consult with your own qualified legal and financial advisors before engaging with any cryptocurrency protocol, especially one involving financial risk.*
