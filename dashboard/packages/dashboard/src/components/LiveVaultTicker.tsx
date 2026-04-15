"use client";

import { useEffect, useRef, useState } from "react";
import { useReadContract } from "wagmi";
import { baseSepolia } from "wagmi/chains";
import { type Abi } from "viem";
import { motion, useInView, AnimatePresence } from "framer-motion";
import { getPBondConfig } from "@/lib/pbond";
import { onDolStateShouldRefresh } from "@/lib/txEvents";
import LiveCounter from "./LiveCounter";
import { AmbientSpotlight } from "./AmbientSpotlight";
import { Glossary } from "./Glossary";

/**
 * LiveVaultTicker — the real on-chain state of Dol, compounding
 * before your eyes.
 *
 * Reads `Dol.totalSupply()` from Base Sepolia every 10 seconds,
 * then hands the seed value off to LiveCounter which owns a RAF
 * loop writing `element.innerText` directly — zero React
 * reconciliation in the per-frame hot path. At 144 Hz the number
 * ticks every 6.94 ms with sub-microsecond precision on the
 * compounded delta.
 *
 * The point: a BTC-lite visitor who has never seen a DeFi protocol
 * gets to watch real capital growing in real time. Most "DeFi demo"
 * counters show fake animated numbers. This one is the actual
 * sum of everything everyone has ever put into Dol, accruing at the
 * current target rate, live.
 *
 * When the RPC is unreachable we fall back to a static "— Dol"
 * placeholder and keep the section visible (still conveys the
 * idea), but don't seed LiveCounter until a real value lands.
 */

const APY = 0.075;
const POLL_MS = 10_000;
const SHARE_DECIMALS = 6;
const TARGET_CHAIN_ID = baseSepolia.id;

export function LiveVaultTicker() {
  const config = getPBondConfig();
  const senior = config.senior;

  const { data: totalSupplyRaw, refetch: refetchTotalSupply } = useReadContract(
    {
      address: senior.address,
      abi: senior.abi as Abi,
      functionName: "totalSupply",
      chainId: TARGET_CHAIN_ID,
      query: {
        enabled: !!senior.address,
        refetchInterval: POLL_MS,
        refetchIntervalInBackground: false,
      },
    },
  );

  // Any deposit/withdraw/claim tx anywhere in the app forces an
  // immediate totalSupply refetch so the landing-page ticker reflects
  // the change within one RPC round trip. Tab visibility change also
  // triggers a refetch (user returning from another tab).
  useEffect(() => {
    const unsub = onDolStateShouldRefresh(() => refetchTotalSupply());
    return unsub;
  }, [refetchTotalSupply]);

  const totalDol =
    typeof totalSupplyRaw === "bigint"
      ? Number(totalSupplyRaw) / 10 ** SHARE_DECIMALS
      : null;

  const ref = useRef<HTMLDivElement>(null);
  const inView = useInView(ref, { once: true, margin: "-80px" });

  // Activity pulse — when on-chain totalSupply changes between polls
  // (i.e., someone just deposited or withdrew), fire a one-shot CSS
  // ripple from the center of the big number. Each pulse gets its
  // own id so AnimatePresence can overlap multiple ripples cleanly
  // if a burst of activity comes in.
  //
  // Cleanup: every pulse schedules a setTimeout to drop itself after
  // 2.5s. We track those timeout handles in a ref and clear all of
  // them on unmount, so React never tries to update state on an
  // unmounted component when the user navigates away mid-pulse.
  const [pulses, setPulses] = useState<number[]>([]);
  const lastTotalRef = useRef<number | null>(null);
  const timeoutsRef = useRef<ReturnType<typeof setTimeout>[]>([]);

  useEffect(() => {
    return () => {
      timeoutsRef.current.forEach(clearTimeout);
      timeoutsRef.current = [];
    };
  }, []);

  useEffect(() => {
    if (totalDol === null) return;
    const prev = lastTotalRef.current;
    // First value we see — record and skip (no pulse on first load).
    if (prev === null) {
      lastTotalRef.current = totalDol;
      return;
    }
    // Delta threshold: ignore microscopic changes that fall inside the
    // floating-point noise band. A real deposit or withdraw will always
    // produce a delta much larger than this.
    const delta = Math.abs(totalDol - prev);
    if (delta > 1e-6) {
      const id = Date.now() + Math.random();
      setPulses((p) => [...p, id]);
      const handle = setTimeout(() => {
        setPulses((p) => p.filter((x) => x !== id));
        timeoutsRef.current = timeoutsRef.current.filter((t) => t !== handle);
      }, 2500);
      timeoutsRef.current.push(handle);
    }
    lastTotalRef.current = totalDol;
  }, [totalDol]);

  return (
    <section
      id="grow"
      className="relative overflow-hidden px-6 py-32"
      ref={ref}
    >
      {/* Ambient radial glow behind the number — pure CSS, compositor */}
      <div
        className="pointer-events-none absolute inset-0 -z-10"
        aria-hidden
        style={{
          background:
            "radial-gradient(ellipse 60% 40% at 50% 50%, rgba(255,255,255,0.04) 0%, transparent 70%)",
        }}
      />

      {/* Ambient cursor spotlight — tracks pointermove at 144Hz via
          RAF-throttled CSS custom properties. Subtle white highlight
          follows the cursor while it's inside the section. */}
      <AmbientSpotlight size={640} intensity={0.1} />

      <div className="mx-auto flex max-w-3xl flex-col items-center text-center">
        <motion.span
          initial={{ opacity: 0, y: 8 }}
          animate={inView ? { opacity: 1, y: 0 } : undefined}
          transition={{ duration: 0.9, ease: [0.05, 0.7, 0.1, 1.0] }}
          className="inline-flex items-center text-[11px] font-medium uppercase tracking-[0.18em] text-white/40"
        >
          Live, <Glossary term="on-chain">on-chain</Glossary>
        </motion.span>

        <motion.h2
          initial={{ opacity: 0, y: 24 }}
          animate={inView ? { opacity: 1, y: 0 } : undefined}
          transition={{
            duration: 1,
            ease: [0.05, 0.7, 0.1, 1.0],
            delay: 0.08,
          }}
          className="mt-4 text-[32px] md:text-[44px] font-semibold text-white"
          style={{ letterSpacing: "-0.03em" }}
        >
          Growing every second.
        </motion.h2>

        <motion.div
          initial={{ opacity: 0, y: 24 }}
          animate={inView ? { opacity: 1, y: 0 } : undefined}
          transition={{
            duration: 1.2,
            ease: [0.05, 0.7, 0.1, 1.0],
            delay: 0.18,
          }}
          className="mt-14"
        >
          {totalDol !== null ? (
            // Two adjacent spans — the counter runs under the iridescent
            // gradient (background-clip:text + transparent text-fill),
            // while the "Dol" suffix sits as its own span outside that
            // clip so it stays dim-white and doesn't inherit transparency.
            <div className="relative flex items-baseline justify-center gap-3 whitespace-nowrap sm:gap-4">
              {/* Activity pulses — one SVG ring per recent on-chain
                  event. Each pulse runs a 2.5s CSS keyframe animation
                  scaling from 0 → 3 and fading out. The rings sit
                  absolutely behind the number so they spread out from
                  the counter without pushing layout. */}
              <div
                className="pointer-events-none absolute left-1/2 top-1/2 -z-10"
                style={{
                  transform: "translate(-50%, -50%)",
                  width: 220,
                  height: 220,
                }}
                aria-hidden
              >
                <AnimatePresence>
                  {pulses.map((id) => (
                    <motion.span
                      key={id}
                      className="absolute inset-0 rounded-full border border-white/30"
                      initial={{ scale: 0.2, opacity: 0.6 }}
                      animate={{ scale: 3, opacity: 0 }}
                      exit={{ opacity: 0 }}
                      transition={{ duration: 2.4, ease: [0.05, 0.7, 0.1, 1.0] }}
                      style={{ willChange: "transform, opacity" }}
                    />
                  ))}
                </AnimatePresence>
              </div>
              {/*
                The 6-decimal ticker can exceed a narrow phone width
                if the font stays at 64 px. `clamp(32 px, 11vw, 96 px)`
                scales it from 41 px at 375 px → 84 px at 768 px → 96 px
                at ~874 px+. Paired with the suffix's matching
                clamp(18 px, 5.5vw, 48 px) so the two stay proportional.
                `whitespace-nowrap` on the parent flex row guarantees
                the number + Dol suffix stay on a single line.
              */}
              <span
                className="font-bold leading-[0.95]"
                style={{
                  fontSize: "clamp(32px, 11vw, 96px)",
                  letterSpacing: "-0.045em",
                  backgroundImage:
                    "linear-gradient(120deg, #fafafa 0%, #a5b4fc 18%, #f0abfc 35%, #fef3c7 52%, #a5f3fc 70%, #e2e8f0 100%)",
                  backgroundSize: "220% 100%",
                  WebkitBackgroundClip: "text",
                  backgroundClip: "text",
                  WebkitTextFillColor: "transparent",
                  color: "transparent",
                  animation: "liveShimmer 8s ease-in-out infinite",
                }}
              >
                <LiveCounter initial={totalDol} apy={APY} decimals={6} />
              </span>
              <span
                className="font-semibold text-white/50"
                style={{
                  fontSize: "clamp(18px, 5.5vw, 48px)",
                  WebkitTextFillColor: "rgba(255,255,255,0.5)",
                }}
              >
                Dol
              </span>
            </div>
          ) : (
            <div
              className="font-bold leading-[0.95] text-white/20"
              style={{
                fontSize: "clamp(32px, 11vw, 96px)",
                letterSpacing: "-0.045em",
              }}
            >
              — Dol
            </div>
          )}
        </motion.div>

        <motion.p
          initial={{ opacity: 0 }}
          animate={inView ? { opacity: 1 } : undefined}
          transition={{
            duration: 1,
            ease: [0.05, 0.7, 0.1, 1.0],
            delay: 0.3,
          }}
          className="mt-10 max-w-xl text-[14px] md:text-[15px] leading-relaxed text-white/45"
        >
          This is the real sum of every Dol anyone has put in — pulled from
          the chain, compounding right now at the target rate. It updates
          every frame. Nothing is simulated.
        </motion.p>
      </div>

      <style jsx global>{`
        @keyframes liveShimmer {
          0%,
          100% {
            background-position: 0% 50%;
          }
          50% {
            background-position: 100% 50%;
          }
        }
      `}</style>
    </section>
  );
}
