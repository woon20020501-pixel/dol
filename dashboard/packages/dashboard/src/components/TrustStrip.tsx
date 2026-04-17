"use client";

import { useRef } from "react";
import { motion, useInView } from "framer-motion";

/**
 * Trust Strip — the five "why you can believe us" signals that live
 * right above the final CTA. Inspired by Revolut / Wealthfront /
 * Linear homepage footer strips, where a tight row of hard claims
 * backed by verifiable facts buys more trust than any marketing
 * paragraph.
 *
 * Every label here is:
 *   1. True (not aspirational)
 *   2. Verifiable on-chain or on GitHub by the reader
 *   3. Written in plain English a BTC-lite user understands without
 *      reaching for Google
 *
 * Banned vocabulary ("smart contract", "APY", "yield", "earn",
 * "interest") is avoided per the  landing cleanup audit.
 *
 * Layout: responsive grid — two columns on phones, five across on
 * desktop. Icons use Tabler/Lucide strokes at 20 px, wrapped in a
 * 48 px halo circle for a consistent weight across different glyph
 * complexities. Spacing uses `leading-none` + explicit `text-*`
 * sizes to keep cross-row alignment stable.
 */

type Signal = {
  icon: React.ReactNode;
  label: string;
  sub: string;
};

const SIGNALS: Signal[] = [
  {
    // 1:1 backing — visible shield
    icon: (
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
        className="h-5 w-5"
        aria-hidden
      >
        <path d="M12 2 4 5v7c0 5 3.5 8 8 10 4.5-2 8-5 8-10V5l-8-3z" />
        <path d="M9 12l2 2 4-4" />
      </svg>
    ),
    label: "1:1 USDC backed",
    sub: "Every Dol is backed by one USDC in the vault.",
  },
  {
    // Code audited — magnifier over code
    icon: (
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
        className="h-5 w-5"
        aria-hidden
      >
        <path d="m7 8-4 4 4 4" />
        <path d="m17 8 4 4-4 4" />
        <path d="m14 4-4 16" />
      </svg>
    ),
    label: "Code audited",
    sub: "Verified on Basescan. Reviewed by external auditors.",
  },
  {
    // Non-custodial — key
    icon: (
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
        className="h-5 w-5"
        aria-hidden
      >
        <circle cx="8" cy="15" r="4" />
        <path d="m10.85 12.15 7.4-7.4" />
        <path d="m18 5 2 2" />
        <path d="m15 8 2 2" />
      </svg>
    ),
    label: "You hold your keys",
    sub: "Non-custodial. We never touch your wallet.",
  },
  {
    // Withdraw anytime — arrows up-down
    icon: (
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
        className="h-5 w-5"
        aria-hidden
      >
        <path d="M7 4v16" />
        <path d="m3 8 4-4 4 4" />
        <path d="M17 20V4" />
        <path d="m13 16 4 4 4-4" />
      </svg>
    ),
    label: "Withdraw anytime",
    sub: "No lockup. Cash out instantly or in 30 minutes.",
  },
  {
    // On-chain — chain link
    icon: (
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
        className="h-5 w-5"
        aria-hidden
      >
        <path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" />
        <path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" />
      </svg>
    ),
    label: "On-chain transparent",
    sub: "Every Dol and every transaction is publicly verifiable.",
  },
];

export function TrustStrip() {
  const ref = useRef<HTMLDivElement>(null);
  const inView = useInView(ref, { once: true, margin: "-120px" });

  return (
    <section id="trust" className="px-6 py-32">
      <div className="mx-auto max-w-5xl" ref={ref}>
        <motion.h2
          initial={{ opacity: 0, y: 24 }}
          animate={inView ? { opacity: 1, y: 0 } : undefined}
          transition={{ duration: 1, ease: [0.05, 0.7, 0.1, 1.0] }}
          className="text-center text-[28px] md:text-[36px] font-semibold text-white"
          style={{ letterSpacing: "-0.03em" }}
        >
          Built to be worth your trust.
        </motion.h2>
        <motion.p
          initial={{ opacity: 0, y: 16 }}
          animate={inView ? { opacity: 1, y: 0 } : undefined}
          transition={{ duration: 1, ease: [0.05, 0.7, 0.1, 1.0], delay: 0.1 }}
          className="mt-3 text-center text-[14px] md:text-[15px] text-white/45"
        >
          Every claim below is verifiable. Nothing is marketing.
        </motion.p>

        {/*
          flex-wrap + justify-center keeps the last row centered on
          small viewports where the item count doesn't divide evenly
          into the column count. On desktop all five sit in a single
          horizontal row; on tablet they wrap to 3+2 centered; on
          phone they wrap to 2+2+1 with the last card dead-center.
          Every card has a fixed min-width so the layout doesn't jitter
          as the viewport resizes.
        */}
        <div className="mt-14 flex flex-wrap items-start justify-center gap-x-6 gap-y-10 md:gap-x-4">
          {SIGNALS.map((s, i) => (
            <motion.div
              key={s.label}
              initial={{ opacity: 0, y: 16 }}
              animate={inView ? { opacity: 1, y: 0 } : undefined}
              transition={{
                duration: 0.9,
                ease: [0.05, 0.7, 0.1, 1.0],
                delay: 0.18 + i * 0.06,
              }}
              className="flex w-[160px] flex-col items-center text-center md:w-[180px]"
            >
              <div className="flex h-12 w-12 items-center justify-center rounded-full border border-white/10 bg-white/[0.03] text-white/80">
                {s.icon}
              </div>
              <div
                className="mt-4 text-[13px] font-semibold leading-tight text-white"
                style={{ letterSpacing: "-0.01em" }}
              >
                {s.label}
              </div>
              <div className="mt-1.5 text-[11px] leading-relaxed text-white/40">
                {s.sub}
              </div>
            </motion.div>
          ))}
        </div>
      </div>
    </section>
  );
}
