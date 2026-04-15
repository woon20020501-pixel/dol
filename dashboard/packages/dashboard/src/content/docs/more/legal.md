# Legal

Dol's legal documents live on dedicated routes outside the documentation site. They are versioned and their wording is intentionally precise, because they form the basis of the user agreement.

## The three documents

### [Terms of Service →](/legal/terms)
Version 0.1 · Effective 2026-04-14

The user agreement. Defines who Dol is, who can use it, what we can and cannot do, what you represent when you click through the clickwrap, and what happens if you violate the terms. Includes the **Restitution and Enforcement Clause** (Section 11) which explains how Dol Labs handles wallets identified as originating from restricted jurisdictions, and the **Force Majeure** clause (Section 13) which limits Dol's liability for events outside its control.

The most important sections to read carefully if you are about to deposit:

- **Section 8** — User Representations. The five things you formally promise when you click through the clickwrap.
- **Section 11** — Restitution and Enforcement. What we reserve the right to do if you bypass our restrictions.
- **Section 12** — Commodity Pool disclosure. Why we are not registered with the U.S. CFTC and what that means.

### [Privacy Policy →](/legal/privacy)
Version 0.1 · Effective 2026-04-14

What we collect (very little), what we do not collect (KYC documents, third-party tracking), how we handle blockchain immutability (we do not control it), and how we treat data from users in restricted jurisdictions.

The short version: Dol Phase 1 is a deliberately low-collection product. We do not run Google Analytics, Meta Pixel, or any tracking pixels. We collect your wallet address (which is already public on-chain), your IP for geographic enforcement, and a few localStorage keys for clickwrap acceptance. That is it.

### [Risk Disclosure →](/legal/risk)
Version 0.1 · Effective 2026-04-14

The full risk list. Eleven risk categories covering everything from total loss to regulatory action to wallet compromise to the strategy losing money. This is the most important document in the legal stack — it is the one that proves you were informed before you committed any money.

Read every section. Then read it again. The whole point of this document is that you cannot later claim "I did not know."

The shorter, less formal version of the same content is at [Risks](/docs/trust/risks). Both say the same things — the legal version uses precise language, the docs version uses plain English.

## How clickwrap acceptance works

Before you can deposit any funds into Dol, you have to actively accept the Terms of Service, Privacy Policy, and Risk Disclosure on a clickwrap modal. The modal appears the first time you try to deposit from any wallet. Once accepted, the acceptance is recorded in your browser's local storage tied to your wallet address and the document version.

If we materially change any of the three documents, we will bump the version number, and you will be asked to re-accept on your next deposit attempt. The version pin in the localStorage key forces re-acceptance whenever the documents move.

The clickwrap is browser-scoped (it appears once per device per visit) and wallet-scoped (it appears once per wallet per document version). Both layers are enforced before any contract call goes out.

## Governing law and disputes

Dol is governed by the laws of the **Cayman Islands** (the entity is in the process of being established there as of Phase 1 launch). Disputes are resolved by **binding arbitration in the Cayman Islands** under the Cayman Islands Arbitration Law, conducted in English by a single arbitrator. You waive the right to participate in a class action lawsuit or class-wide arbitration, and you waive the right to a jury trial.

If you have a dispute that cannot be resolved through normal support channels, contact **legal@dol.app**.

## Real lawyer disclaimer

The legal documents on this site were drafted with significant assistance from automated legal research tools, then reviewed and approved by the Dol team. They are not the product of a licensed attorney's full review yet. A real-lawyer review is planned before any mainnet launch.

If you are about to commit a meaningful amount of money to Dol, you should consult your own qualified legal advisor in your jurisdiction — not rely on these documents alone.
