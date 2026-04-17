/**
 * Inline glossary — plain-English definitions for every jargon term
 * a BTC-lite user might bump into on the Dol landing or product
 * surfaces. Each entry has three fields:
 *
 *   label  — the proper noun as it appears in body copy
 *   short  — one tight sentence, shown as the tooltip headline
 *   long   — one or two more sentences of context, shown beneath
 *            the headline in a smaller weight
 *
 * Writing rules:
 *   - Talk to the user in second person ("you"), never third person
 *   - Never use another jargon term inside a definition without also
 *     defining it somewhere in this file
 *   - Every long description fits in ≤ 260 characters so the tooltip
 *     card stays small enough to fly on top of the content layer
 *   - Banned-word list from  still applies: no "APY", no
 *     "yield", no "smart contract", no "earn"
 */

export type GlossaryTerm =
  | "usdc"
  | "wallet"
  | "gas"
  | "dol"
  | "vault"
  | "non-custodial"
  | "on-chain"
  | "testnet"
  | "approve"
  | "scheduled-cashout"
  | "instant-cashout"
  | "pacifica";

interface GlossaryEntry {
  label: string;
  short: string;
  long: string;
}

export const GLOSSARY: Record<GlossaryTerm, GlossaryEntry> = {
  usdc: {
    label: "USDC",
    short: "A digital dollar.",
    long: "1 USDC is always worth 1 US dollar. Issued by Circle, a regulated US company. Unlike Bitcoin, it doesn't move up and down — it's stable.",
  },
  wallet: {
    label: "Wallet",
    short: "A tool that holds your crypto.",
    long: "Think of it like a personal vault. Only you have the key. Dol never touches it — you hold your own money the whole time.",
  },
  gas: {
    label: "Network fee",
    short: "A small fee for each on-chain action.",
    long: "Blockchains charge a tiny fee to process your request. On Base Sepolia testnet it's a fraction of a cent. On the real network it's still pennies for most actions.",
  },
  dol: {
    label: "Dol",
    short: "A dollar that grows itself.",
    long: "Each Dol is backed 1:1 by USDC in the vault. Hold it and its value slowly grows — no action needed, no lockups.",
  },
  vault: {
    label: "The vault",
    short: "Where your money lives and grows.",
    long: "An on-chain contract that holds all the USDC behind Dol. You can verify its balance yourself on Basescan any time.",
  },
  "non-custodial": {
    label: "Non-custodial",
    short: "You hold your own keys.",
    long: "We can't move, freeze, or take your money. The only person who can is you, with your wallet. If Dol disappears tomorrow, your money stays with you.",
  },
  "on-chain": {
    label: "On-chain",
    short: "Written on a public ledger.",
    long: "Every Dol and every transaction is recorded on the blockchain. Anyone can verify it. Nothing hidden, nothing guessed.",
  },
  testnet: {
    label: "Testnet",
    short: "A practice version of the blockchain.",
    long: "Base Sepolia is the testing ground. The USDC here isn't real money — it's for learning the product without risk. Dol will move to the real network in a later phase.",
  },
  approve: {
    label: "Approve",
    short: "Letting Dol use your USDC.",
    long: "A one-time permission. You approve the exact amount you want to deposit — nothing more. You can revoke it any time from your wallet.",
  },
  "scheduled-cashout": {
    label: "Scheduled cash out",
    short: "Wait 30 minutes, no fee.",
    long: "Request it, wait 30 minutes, then claim your USDC. Zero fee. Best for most amounts.",
  },
  "instant-cashout": {
    label: "Instant cash out",
    short: "Get it now, small fee.",
    long: "Cash out immediately with a 0.05% fee. Pulled from the vault's liquid buffer. Best when you want your money in the same transaction.",
  },
  pacifica: {
    label: "Pacifica",
    short: "The venue where growth comes from.",
    long: "Dol routes capital to Pacifica, an on-chain exchange, and the funding flows back to you. Dol itself never holds a position — Pacifica does.",
  },
};
