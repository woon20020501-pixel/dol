"use client";

import { motion } from "framer-motion";

interface DolHeroImageProps {
  size?: number;
}

/**
 * DolHeroImage — simple, flat, animation-style.
 *
 * No photorealistic 3D attempt. Just a clean luminous disc:
 *   - radial gradient silver→dark in the center
 *   - soft white bloom halo behind (dissolves into bg-black)
 *   - slow shimmer ring rotating around the disc
 *   - breathing float animation
 *   - ground glow breathes inverse
 */
export default function DolHeroImage({ size = 360 }: DolHeroImageProps) {
  return (
    <div
      className="relative flex items-center justify-center"
      style={{ width: size, height: size * 1.1 }}
    >
      {/* Outer bloom — dissolves into black bg */}
      <motion.div
        className="absolute pointer-events-none"
        style={{
          width: size * 1.6,
          height: size * 1.6,
          background:
            "radial-gradient(circle at center, rgba(255,255,255,0.16) 0%, rgba(148,163,184,0.06) 30%, transparent 65%)",
          filter: "blur(50px)",
        }}
        animate={{
          scale: [1, 0.92, 1],
          opacity: [1, 0.75, 1],
        }}
        transition={{
          duration: 4,
          repeat: Infinity,
          ease: "easeInOut",
        }}
        aria-hidden
      />

      {/* Shimmer ring — slow rotation around the disc */}
      <motion.div
        className="absolute pointer-events-none"
        style={{ width: size * 1.05, height: size * 1.05 }}
        animate={{ rotate: 360 }}
        transition={{ duration: 24, repeat: Infinity, ease: "linear" }}
      >
        <svg viewBox="0 0 200 200" className="w-full h-full">
          <defs>
            <linearGradient id="shimmer" x1="0%" y1="0%" x2="100%" y2="100%">
              <stop offset="0%" stopColor="#ffffff" stopOpacity="0" />
              <stop offset="50%" stopColor="#ffffff" stopOpacity="0.5" />
              <stop offset="100%" stopColor="#ffffff" stopOpacity="0" />
            </linearGradient>
          </defs>
          <circle
            cx="100"
            cy="100"
            r="96"
            fill="none"
            stroke="url(#shimmer)"
            strokeWidth="0.6"
          />
        </svg>
      </motion.div>

      {/* The Dol — floats up and down */}
      <motion.div
        className="relative"
        animate={{ y: [0, -12, 0] }}
        transition={{
          duration: 4,
          repeat: Infinity,
          ease: "easeInOut",
        }}
      >
        <svg
          width={size}
          height={size * 0.85}
          viewBox="0 0 240 204"
          role="img"
          aria-label="The Dol"
          style={{ display: "block" }}
        >
          {/* Cute flat pebble icon — no realistic lighting, just soft 2-tone */}

          {/* Bottom half — slightly darker for subtle depth */}
          <ellipse
            cx="120"
            cy="108"
            rx="92"
            ry="80"
            fill="#94a3b8"
          />
          {/* Top half — lighter, covers upper 60% */}
          <ellipse
            cx="120"
            cy="92"
            rx="92"
            ry="72"
            fill="#e2e8f0"
          />
          {/* Soft cheek — tiny top-left light patch, no gradient */}
          <ellipse
            cx="88"
            cy="74"
            rx="22"
            ry="14"
            fill="#f1f5f9"
            opacity="0.7"
          />
          {/* Cute little sparkle dot */}
          <circle
            cx="82"
            cy="68"
            r="4"
            fill="#ffffff"
          />
          <circle
            cx="82"
            cy="68"
            r="1.5"
            fill="#e2e8f0"
          />
        </svg>
      </motion.div>

      {/* Ground shadow — breathes inverse to float */}
      <motion.div
        className="absolute left-1/2 pointer-events-none"
        style={{
          bottom: -size * 0.02,
          width: size * 0.6,
          height: size * 0.06,
          marginLeft: -(size * 0.3),
          background:
            "radial-gradient(ellipse at center, rgba(255,255,255,0.18) 0%, transparent 70%)",
          filter: "blur(20px)",
        }}
        animate={{
          scale: [1, 0.75, 1],
          opacity: [0.9, 0.4, 0.9],
        }}
        transition={{
          duration: 4,
          repeat: Infinity,
          ease: "easeInOut",
        }}
        aria-hidden
      />
    </div>
  );
}
