"use client";

import { useEffect, useState } from "react";
import Link from "next/link";

/**
 * Layer A clickwrap — first-visit geo + VPN + US/sanctions attestation.
 *
 * Shown exactly once per browser profile. Acceptance is recorded in
 * localStorage under `dol_visit_gate_v1` together with a timestamp so
 * we can prove consent was captured (to a technical bar — not legal
 * evidence; that's what Layer B's per-wallet clickwrap is for).
 *
 * Why both layers: Layer A is a cheap first-line filter against
 * casual drive-by traffic from blocked regions. Layer B (on /deposit)
 * is the legally significant one — it binds the acceptance to a
 * specific wallet address at transaction time.
 *
 * The modal is non-dismissible except via accept. We don't offer a
 * "cancel" path because the whole point is that non-accepting users
 * can't use the site. They can still read /legal/* (middleware
 * passthrough) so no one is locked out of the terms themselves.
 *
 * Hidden entirely on /legal/*, /unavailable, and /api/* because:
 *   - legal pages must be readable without any gate
 *   - /unavailable is already a dead end for blocked users
 *   - api routes (future) handle their own auth
 */
const STORAGE_KEY = "dol_visit_gate_v1";

function alreadyAccepted(): boolean {
  try {
    return localStorage.getItem(STORAGE_KEY) !== null;
  } catch {
    return true; // fail closed if storage unavailable — don't block the UI
  }
}

function recordAccept() {
  try {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ ts: Date.now(), v: "v1" }),
    );
  } catch {
    // ignore
  }
}

export function VisitGateModal() {
  const [mounted, setMounted] = useState(false);
  const [open, setOpen] = useState(false);
  const [notUS, setNotUS] = useState(false);
  const [tierA, setTierA] = useState(false);
  const [noVpn, setNoVpn] = useState(false);

  useEffect(() => {
    setMounted(true);
    // Respect the always-allow paths — the gate should never appear
    // on legal/unavailable routes even on first visit.
    const p = window.location.pathname;
    if (
      p.startsWith("/legal") ||
      p.startsWith("/unavailable") ||
      p.startsWith("/api")
    ) {
      return;
    }
    if (!alreadyAccepted()) {
      setOpen(true);
    }
  }, []);

  if (!mounted || !open) return null;

  const allChecked = notUS && tierA && noVpn;

  const onAccept = () => {
    if (!allChecked) return;
    recordAccept();
    setOpen(false);
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="visit-gate-title"
      className="fixed inset-0 z-[100] flex items-center justify-center bg-black/90 p-6 backdrop-blur"
    >
      <div
        className="w-full max-w-md rounded-2xl border border-white/10 bg-[#0a0a0a] p-8 shadow-2xl"
        style={{ boxShadow: "0 30px 80px rgba(0,0,0,0.6)" }}
      >
        <h2
          id="visit-gate-title"
          className="text-2xl font-semibold text-white"
          style={{ letterSpacing: "-0.02em" }}
        >
          Before you continue
        </h2>
        <p className="mt-3 text-[14px] text-white/60">
          Dol is currently only available in Vietnam, Turkey, the Philippines,
          Mexico, and Argentina. Please confirm the following:
        </p>

        <div className="mt-6 space-y-4">
          <CheckboxRow
            id="gate-not-us"
            checked={notUS}
            onChange={setNotUS}
            label="I am not a resident or citizen of the United States, EU, UK, Korea, Japan, China, Canada, Australia, Singapore, or Hong Kong."
          />
          <CheckboxRow
            id="gate-tier-a"
            checked={tierA}
            onChange={setTierA}
            label="I am a legal resident of Vietnam, Turkey, the Philippines, Mexico, or Argentina."
          />
          <CheckboxRow
            id="gate-no-vpn"
            checked={noVpn}
            onChange={setNoVpn}
            label="I am not using a VPN, proxy, or any tool to disguise my country of residence."
          />
        </div>

        <button
          type="button"
          onClick={onAccept}
          disabled={!allChecked}
          className="mt-8 w-full rounded-full bg-white px-6 py-3 text-[15px] font-semibold text-black transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-30"
        >
          Continue
        </button>

        <p className="mt-5 text-center text-[11px] text-white/40">
          By continuing you confirm the above and agree to our{" "}
          <Link href="/legal/terms" className="underline hover:text-white/70">
            Terms
          </Link>
          ,{" "}
          <Link href="/legal/privacy" className="underline hover:text-white/70">
            Privacy Policy
          </Link>
          , and{" "}
          <Link href="/legal/risk" className="underline hover:text-white/70">
            Risk Disclosure
          </Link>
          .
        </p>
      </div>
    </div>
  );
}

function CheckboxRow({
  id,
  checked,
  onChange,
  label,
}: {
  id: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
}) {
  return (
    <label
      htmlFor={id}
      className="flex cursor-pointer items-start gap-3 text-[13px] leading-relaxed text-white/80"
    >
      <input
        id={id}
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="mt-0.5 h-4 w-4 shrink-0 cursor-pointer accent-white"
      />
      <span>{label}</span>
    </label>
  );
}
