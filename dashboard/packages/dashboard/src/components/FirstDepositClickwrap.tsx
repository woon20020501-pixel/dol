"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import Link from "next/link";

/**
 * Layer B clickwrap — per-wallet ToS/Risk acceptance, captured at the
 * moment a user is about to deposit. Unlike Layer A (browser-scoped),
 * this binds acceptance to a specific wallet address so we can prove
 * which version of the legal docs that wallet agreed to.
 *
 * Storage key: `dol_tos_accept_{wallet}_v0.1`. The version suffix lets
 * us force re-acceptance whenever the legal docs are bumped.
 *
 * Usage pattern:
 *   const { requireTos, modal } = useTosAcceptance(walletAddress);
 *   const onClick = () => requireTos(() => doApprove());
 *   return (<>{modal}<button onClick={onClick}>...</button></>);
 *
 * If accepted, the action runs immediately. If not, the modal opens
 * and the action is queued — on accept the queued action fires.
 */
const STORAGE_VERSION = "v0.2";

function storageKey(wallet: string) {
  return `dol_tos_accept_${wallet.toLowerCase()}_${STORAGE_VERSION}`;
}

function isAcceptedFor(wallet: string | null | undefined): boolean {
  if (!wallet) return false;
  try {
    return localStorage.getItem(storageKey(wallet)) !== null;
  } catch {
    return false;
  }
}

function recordAcceptFor(wallet: string) {
  try {
    localStorage.setItem(
      storageKey(wallet),
      JSON.stringify({ ts: Date.now(), version: STORAGE_VERSION }),
    );
  } catch {
    // ignore
  }
}

export function useTosAcceptance(wallet: string | null | undefined) {
  const [open, setOpen] = useState(false);
  const [accepted, setAccepted] = useState(false);
  // Pending action to run after accept. Stored in a ref so that
  // re-renders between requireTos() and onAccept don't lose it.
  const pendingRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    setAccepted(isAcceptedFor(wallet));
  }, [wallet]);

  const requireTos = useCallback(
    (action: () => void) => {
      if (isAcceptedFor(wallet)) {
        action();
        return;
      }
      pendingRef.current = action;
      setOpen(true);
    },
    [wallet],
  );

  const handleAccept = useCallback(() => {
    if (!wallet) return;
    recordAcceptFor(wallet);
    setAccepted(true);
    setOpen(false);
    const pending = pendingRef.current;
    pendingRef.current = null;
    if (pending) pending();
  }, [wallet]);

  const handleCancel = useCallback(() => {
    pendingRef.current = null;
    setOpen(false);
  }, []);

  const modal = open ? (
    <ClickwrapModal onAccept={handleAccept} onCancel={handleCancel} />
  ) : null;

  return { requireTos, accepted, modal };
}

function ClickwrapModal({
  onAccept,
  onCancel,
}: {
  onAccept: () => void;
  onCancel: () => void;
}) {
  const [country, setCountry] = useState(false);
  const [legal, setLegal] = useState(false);
  const allChecked = country && legal;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="tos-clickwrap-title"
      className="fixed inset-0 z-[110] flex items-center justify-center bg-black/90 p-6 backdrop-blur"
    >
      <div
        className="w-full max-w-md rounded-2xl border border-white/10 bg-[#0a0a0a] p-8 shadow-2xl"
        style={{ boxShadow: "0 30px 80px rgba(0,0,0,0.6)" }}
      >
        <h2
          id="tos-clickwrap-title"
          className="text-2xl font-semibold text-white"
          style={{ letterSpacing: "-0.02em" }}
        >
          One last thing.
        </h2>
        <p className="mt-3 text-[14px] text-white/60">
          Before your first deposit, please confirm:
        </p>

        <div className="mt-6 space-y-4">
          <CheckboxRow
            id="cw-country"
            checked={country}
            onChange={setCountry}
            label="I am a legal resident of Vietnam, Turkey, the Philippines, Mexico, or Argentina, and I am at least 18 years old."
          />
          <CheckboxRow
            id="cw-legal"
            checked={legal}
            onChange={setLegal}
            label={
              <>
                I have read and agree to the{" "}
                <Link
                  href="/legal/terms"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="underline hover:text-white"
                >
                  Terms of Service
                </Link>
                ,{" "}
                <Link
                  href="/legal/privacy"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="underline hover:text-white"
                >
                  Privacy Policy
                </Link>
                , and{" "}
                <Link
                  href="/legal/risk"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="underline hover:text-white"
                >
                  Risk Disclosure
                </Link>
                . I understand my deposit can lose value, including to zero.
              </>
            }
          />
        </div>

        <div className="mt-8 flex flex-col gap-3">
          <button
            type="button"
            onClick={onAccept}
            disabled={!allChecked}
            className="w-full rounded-full bg-white px-6 py-3 text-[15px] font-semibold text-black transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-30"
          >
            I agree — continue
          </button>
          <button
            type="button"
            onClick={onCancel}
            className="w-full rounded-full border border-white/10 bg-transparent px-6 py-3 text-[14px] text-white/60 transition-colors hover:bg-white/5"
          >
            Cancel
          </button>
        </div>
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
  label: React.ReactNode;
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
