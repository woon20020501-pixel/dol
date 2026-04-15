"use client";

import { useState, useMemo, useDeferredValue, useRef } from "react";
import Link from "next/link";
import { motion, useInView } from "framer-motion";
import { MagneticCard } from "./MagneticCard";

/**
 * Interactive ROI preview card — Wealthfront / Toss pattern.
 *
 * A visitor who has never touched DeFi gets ONE concrete answer to
 * the "what do I get" question: they type or slide an amount and
 * immediately see what that amount would be worth over one week,
 * one month, and one year at Dol's target rate. A small sparkline
 * shows the 12-month trajectory so the compounding feels visible,
 * not mathematical.
 *
 * Design notes (anti-jank):
 *   - Every flex row uses `leading-none` on its children so mixed
 *     font sizes don't throw the optical baseline off (the same
 *     class of bug that made the CashoutSheet Max button float).
 *   - The amount input is `type="text"` with manual sanitization —
 *     type=number has known cross-browser rendering quirks at this
 *     font size.
 *   - The slider uses `accent-white` so the stock UA control is
 *     native-fast and compositor-friendly. No custom track/thumb
 *     CSS that can drift across Safari/Firefox/Chrome.
 *
 * Math: continuous compound at 7.5% APY.
 *   future_value = principal × e^(rate × years)
 * Matches how LiveCounter on the hero already works.
 */

const APY = 0.075;

const TIMEFRAMES = [
  { label: "In 1 week", days: 7 },
  { label: "In 1 month", days: 30 },
  { label: "In 1 year", days: 365 },
] as const;

function formatUSD(v: number): string {
  return v.toLocaleString("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
}

// Upper bound for the try-before-you-buy input. Must match the slider
// `max` below so typing a big number doesn't desync the slider thumb
// from the displayed value.
const MAX_AMOUNT = 100_000;

function sanitizeAmount(raw: string): number {
  // Digits only, no decimals — treating ROI input as whole USDC.
  const digits = raw.replace(/[^\d]/g, "");
  if (!digits) return 0;
  const parsed = Number(digits);
  if (!Number.isFinite(parsed) || parsed < 0) return 0;
  return Math.min(parsed, MAX_AMOUNT);
}

export function ROICalculator() {
  const [amount, setAmount] = useState<number>(1000);
  // Slider + input stay on the urgent render path so the thumb and
  // caret feel 1:1 with the pointer. Everything derived — three
  // projection cards, sparkline, CTA label — runs on the deferred
  // value so React 18 can skip or batch those renders when frames
  // get tight. At 144 Hz the sparkline still updates visually
  // indistinguishably from the slider, but the scheduler gets room
  // to prioritize input responsiveness during heavy drag.
  const deferredAmount = useDeferredValue(amount);
  const ref = useRef<HTMLDivElement>(null);
  const inView = useInView(ref, { once: true, margin: "-120px" });

  // Discrete per-timeframe projection
  const projections = useMemo(
    () =>
      TIMEFRAMES.map((tf) => {
        const years = tf.days / 365;
        const value = deferredAmount * Math.exp(APY * years);
        const earned = value - deferredAmount;
        return { ...tf, value, earned };
      }),
    [deferredAmount],
  );

  // 13-point sparkline (one per month + the endpoint). Uses a
  // fixed 320×80 viewBox so we never need to recompute on resize.
  const sparklinePath = useMemo(() => {
    const W = 320;
    const H = 80;
    const POINTS = 13;
    const principal = Math.max(deferredAmount, 1);
    const minVal = principal;
    const maxVal = principal * Math.exp(APY); // 12-month value
    const range = maxVal - minVal || 1;
    const segments: string[] = [];
    for (let i = 0; i < POINTS; i++) {
      const t = i / (POINTS - 1);
      const value = principal * Math.exp(APY * t);
      const x = t * W;
      const y = H - ((value - minVal) / range) * H;
      segments.push(`${i === 0 ? "M" : "L"} ${x.toFixed(1)} ${y.toFixed(1)}`);
    }
    const linePath = segments.join(" ");
    const areaPath = `${linePath} L ${W} ${H} L 0 ${H} Z`;
    return { linePath, areaPath };
  }, [deferredAmount]);

  return (
    <section className="px-6 py-32">
      <div className="mx-auto max-w-3xl" ref={ref}>
        <motion.div
          initial={{ opacity: 0, y: 24 }}
          animate={inView ? { opacity: 1, y: 0 } : undefined}
          transition={{ duration: 1, ease: [0.05, 0.7, 0.1, 1.0] }}
          className="text-center"
        >
          <h2
            className="text-[40px] md:text-[56px] font-bold text-white leading-[1.02]"
            style={{ letterSpacing: "-0.04em" }}
          >
            See it grow.
          </h2>
          <p className="mt-4 text-[15px] md:text-[17px] text-white/50">
            Pick an amount. Watch what happens.
          </p>
        </motion.div>

        <motion.div
          initial={{ opacity: 0, y: 32 }}
          animate={inView ? { opacity: 1, y: 0 } : undefined}
          transition={{ duration: 1, ease: [0.05, 0.7, 0.1, 1.0], delay: 0.15 }}
          className="mt-14"
        >
         <MagneticCard
           className="rounded-3xl border border-white/10 bg-white/[0.02] p-6 md:p-10 backdrop-blur-sm"
           style={{ boxShadow: "0 40px 120px rgba(0,0,0,0.4)" }}
         >
          {/* Amount input row */}
          <label
            htmlFor="roi-amount"
            className="text-[11px] font-medium uppercase tracking-[0.14em] text-white/40"
          >
            How much would you put in?
          </label>
          <div
            className="mt-3 flex items-center gap-3 rounded-2xl border border-white/10 bg-white/[0.03] px-5 py-4 focus-within:border-white/25 transition-colors"
          >
            <span className="text-[26px] font-semibold leading-none text-white/40">
              $
            </span>
            <input
              id="roi-amount"
              type="text"
              inputMode="numeric"
              autoComplete="off"
              value={amount ? amount.toLocaleString("en-US") : ""}
              placeholder="1,000"
              onChange={(e) => setAmount(sanitizeAmount(e.target.value))}
              className="flex-1 min-w-0 bg-transparent text-[26px] font-semibold leading-none text-white placeholder-white/25 focus:outline-none"
            />
            <div className="flex shrink-0 items-center gap-2">
              <span className="text-[15px] leading-none text-white/45">
                USDC
              </span>
            </div>
          </div>

          {/* Slider.
              `h-2` styles the track on Chromium; Firefox and Safari
              respect `accent-color` for the thumb and use their own
              track height. That's acceptable — slider thickness drift
              across browsers is invisible to the user, the thumb
              color stays consistent. Value is clamped by MAX_AMOUNT. */}
          <div className="mt-6">
            <input
              type="range"
              min={100}
              max={MAX_AMOUNT}
              step={100}
              value={Math.min(amount, MAX_AMOUNT) || 100}
              onChange={(e) => setAmount(Number(e.target.value))}
              aria-label="Deposit amount slider"
              className="w-full accent-white h-2 cursor-pointer"
            />
            <div className="mt-2 flex justify-between text-[10px] font-medium uppercase tracking-[0.14em] text-white/30">
              <span>$100</span>
              <span>${(MAX_AMOUNT / 1000).toFixed(0)}k</span>
            </div>
          </div>

          {/* Projections grid.
              Plain <div> (no motion.div + layout) — card size is
              fixed by padding and the only thing that changes is the
              number text, so a layout animation would be wasted work
              measured on every state tick during a slider drag. At
              144 Hz that matters. `tabular-nums` keeps the number
              widths consistent so digit changes don't trigger a
              visual shimmer either. */}
          <div className="mt-10 grid grid-cols-1 gap-3 sm:grid-cols-3">
            {projections.map((p) => (
              <div
                key={p.label}
                className="rounded-2xl border border-white/5 bg-white/[0.02] p-5"
              >
                <div className="text-[10px] font-medium uppercase tracking-[0.14em] text-white/40">
                  {p.label}
                </div>
                <div
                  className="mt-3 text-[24px] font-semibold text-white leading-none tabular-nums"
                  style={{ letterSpacing: "-0.02em" }}
                >
                  {formatUSD(p.value)}
                </div>
                <div className="mt-2 text-[12px] leading-none text-emerald-400/85 tabular-nums">
                  +{formatUSD(p.earned)}
                </div>
              </div>
            ))}
          </div>

          {/* Sparkline */}
          <div
            className="mt-10 relative"
            aria-hidden
            style={{ height: 80 }}
          >
            <svg
              viewBox="0 0 320 80"
              className="w-full h-full"
              preserveAspectRatio="none"
            >
              <defs>
                <linearGradient
                  id="roi-fill"
                  x1="0%"
                  y1="0%"
                  x2="0%"
                  y2="100%"
                >
                  <stop offset="0%" stopColor="#ffffff" stopOpacity="0.15" />
                  <stop offset="100%" stopColor="#ffffff" stopOpacity="0" />
                </linearGradient>
              </defs>
              <path d={sparklinePath.areaPath} fill="url(#roi-fill)" />
              <path
                d={sparklinePath.linePath}
                fill="none"
                stroke="#ffffff"
                strokeWidth="1.5"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
            <div className="pointer-events-none mt-2 flex justify-between text-[10px] font-medium uppercase tracking-[0.14em] text-white/30">
              <span>Today</span>
              <span>1 year</span>
            </div>
          </div>

          {/* CTA — when amount is 0 (empty input) we fall back to a
              generic label so the button never reads "Start with $0.00",
              which would be nonsense. */}
          <Link
            href="/deposit"
            className="mt-10 flex items-center justify-center rounded-full bg-white px-8 py-4 text-[15px] font-semibold text-black transition-opacity hover:opacity-90"
            style={{ boxShadow: "0 14px 40px rgba(255,255,255,0.15)" }}
          >
            {deferredAmount > 0
              ? `Start with ${formatUSD(deferredAmount)}`
              : "Start with your own amount"}
          </Link>

          <p className="mt-5 text-center text-[11px] text-white/30 leading-relaxed">
            Target rate up to 7.5% a year. Not a promise — it moves with the
            market and may be lower. Capital is at risk.
          </p>
         </MagneticCard>
        </motion.div>
      </div>
    </section>
  );
}
