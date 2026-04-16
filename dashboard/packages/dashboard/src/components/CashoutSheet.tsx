"use client";

import { useState, useEffect } from "react";
import { motion, AnimatePresence, type Transition } from "framer-motion";
import { X, Zap, Clock, Check, AlertTriangle } from "lucide-react";

const APPLE_SPRING: Transition = {
  type: "spring",
  stiffness: 400,
  damping: 30,
  mass: 1.2,
};

type CashoutChoice = "instant" | "scheduled";

interface CashoutSheetProps {
  open: boolean;
  /** Human-readable max balance in Dol units (6 decimal friendly). */
  balance: number;
  onClose: () => void;
  /**
   * Fires when the user picks "Scheduled". Parent calls the real
   * pBondSenior.redeem() with `amount` worth of shares and manages
   * its own toasts. The sheet just hands off the choice and amount
   * and closes.
   */
  onScheduled?: (amount: number) => void;
  /**
   * Fires when the user picks "Instant" (Plan A). Parent calls
   * pBondSenior.instantRedeem() for `amount` worth of shares and
   * handles the InsufficientLiquidity fallback → Scheduled.
   */
  onInstant?: (amount: number) => void;
  /** Disable Instant — e.g., if feeRecipient is unset (defensive). */
  instantDisabled?: boolean;
  /**
   * Instant would revert because the vault's USDC buffer is smaller
   * than the REQUESTED amount. Rendered as a *transient* "temporarily
   * unavailable" state rather than a hard disable, because the buffer
   * can refill at any moment and the next user tap might work. See
   * See the cash-out design doc for context.
   */
  instantBufferShort?: boolean;
}

/**
 * CashoutSheet — glass sheet with Instant + Scheduled choices.
 * Scheduled is wired to the real redeem flow via `onScheduled` callback.
 * Instant is rendered disabled until Plan A ships.
 */
export default function CashoutSheet({
  open,
  balance,
  onClose,
  onScheduled,
  onInstant,
  instantDisabled = false,
  instantBufferShort = false,
}: CashoutSheetProps) {
  const [step, setStep] = useState<"choose" | "done">("choose");
  const [choice, setChoice] = useState<CashoutChoice | null>(null);
  // Amount input — stored as string so we can show what the user typed
  // (including mid-type partials like "12." or ""). Parsed to number at
  // submit time with a finite check; we never pass NaN or Infinity into
  // the parent callbacks.
  const [amountInput, setAmountInput] = useState<string>("");

  // Reset state when closed — includes amount so the sheet starts
  // empty on the next open, not with a stale value.
  useEffect(() => {
    if (!open) {
      const t = setTimeout(() => {
        setStep("choose");
        setChoice(null);
        setAmountInput("");
      }, 400);
      return () => clearTimeout(t);
    }
  }, [open]);

  // Escape key closes the sheet — power-user expectation for any
  // modal-like surface. Only binds while open.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  // Body scroll lock while open. Without this, mobile users can
  // scroll the page behind the sheet which feels like a broken
  // modal. Restore previous overflow on close.
  useEffect(() => {
    if (!open) return;
    const prev = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = prev;
    };
  }, [open]);

  // Parsed amount — clamped to [0, balance]. If the string is empty or
  // not a finite number, amountNum is 0 and the buttons stay disabled.
  const amountNum = (() => {
    const parsed = Number(amountInput);
    if (!Number.isFinite(parsed) || parsed < 0) return 0;
    return Math.min(parsed, balance);
  })();
  const amountValid = amountNum > 0;
  const overBalance = Number(amountInput) > balance;

  const onMax = () => {
    // Round DOWN at wei precision (6 decimals for USDC/Dol) so the
    // parent's toShares() conversion lands at exactly the user's
    // available balance — never 1 wei over, which would revert with
    // "exceeds shares". Using Math.floor(x * 1e6) / 1e6 is
    // floating-safe for balances below ~9e9 Dol (we're nowhere near
    // that cap). Trailing zeros trimmed for a clean display.
    if (balance <= 0) return;
    const floored = Math.floor(balance * 1_000_000) / 1_000_000;
    const cleaned = floored.toFixed(6).replace(/\.?0+$/, "");
    setAmountInput(cleaned || "0");
  };

  const handleSelect = (c: CashoutChoice) => {
    if (!amountValid) return;
    if (c === "instant" && (instantDisabled || instantBufferShort)) return;
    setChoice(c);
    if (c === "instant" && onInstant) {
      onInstant(amountNum);
      setTimeout(onClose, 150);
      return;
    }
    if (c === "scheduled" && onScheduled) {
      onScheduled(amountNum);
      setTimeout(onClose, 150);
      return;
    }
    setStep("done");
  };

  return (
    <AnimatePresence>
      {open && (
        <>
          {/* Backdrop */}
          <motion.div
            className="fixed inset-0 bg-black/80 backdrop-blur-sm z-50"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.25 }}
            onClick={onClose}
          />

          {/* Sheet */}
          <motion.div
            className="fixed left-1/2 bottom-0 -translate-x-1/2 w-full max-w-[520px] z-50 px-4 pb-8"
            initial={{ y: 600, opacity: 0 }}
            animate={{ y: 0, opacity: 1 }}
            exit={{ y: 600, opacity: 0 }}
            transition={APPLE_SPRING}
          >
            <div className="relative rounded-[28px] bg-[#0a0a0a] border border-white/10 p-8 shadow-2xl">
              {/* Close */}
              <button
                onClick={onClose}
                className="absolute top-5 right-5 p-2 rounded-full text-white/40 hover:text-white hover:bg-white/5 transition-colors"
                aria-label="Close"
              >
                <X className="h-4 w-4" />
              </button>

              {step === "choose" ? (
                <>
                  <h3
                    className="text-3xl font-bold text-white"
                    style={{ letterSpacing: "-0.03em" }}
                  >
                    Cash out your Dol
                  </h3>
                  <p className="mt-2 text-sm text-white/50">
                    Choose how much to cash out. You have {balance.toFixed(4)} Dol.
                  </p>

                  {/* Amount input with Max button */}
                  <div className="mt-6">
                    <label htmlFor="cashout-amount" className="sr-only">
                      Amount to cash out
                    </label>
                    <div
                      className={`flex items-center gap-3 rounded-2xl border px-5 py-4 transition-colors ${
                        overBalance
                          ? "border-red-500/40 bg-red-500/[0.03]"
                          : "border-white/10 bg-white/[0.03] focus-within:border-white/25"
                      }`}
                    >
                      <input
                        id="cashout-amount"
                        type="text"
                        inputMode="decimal"
                        autoComplete="off"
                        autoCorrect="off"
                        spellCheck={false}
                        placeholder="0.00"
                        value={amountInput}
                        onChange={(e) => {
                          // Strip non-digits/non-dot (kills e, E, -, +,
                          // commas, spaces, scientific notation).
                          let v = e.target.value.replace(/[^\d.]/g, "");
                          // Collapse multiple dots to a single dot after
                          // the first one.
                          const firstDot = v.indexOf(".");
                          if (firstDot !== -1) {
                            v =
                              v.slice(0, firstDot + 1) +
                              v.slice(firstDot + 1).replace(/\./g, "");
                          }
                          // Trim leading zeros on the whole part so
                          // "0001.5" normalizes to "1.5". Keep a lone
                          // "0" or "0.X" form intact.
                          v = v.replace(/^0+(\d)/, "$1");
                          // Cap fractional part to 6 decimals (USDC).
                          if (v.includes(".")) {
                            const [whole, frac = ""] = v.split(".");
                            v = whole + "." + frac.slice(0, 6);
                          }
                          setAmountInput(v);
                        }}
                        onKeyDown={(e) => {
                          // Enter eats so an accidental key press
                          // can't submit anything unexpected.
                          if (e.key === "Enter") e.preventDefault();
                        }}
                        className="min-w-0 flex-1 bg-transparent text-[22px] font-semibold leading-none text-white placeholder-white/30 focus:outline-none"
                        aria-label={`Amount in Dol to cash out. Balance ${balance.toFixed(4)}`}
                      />
                      {/* Right cluster — suffix + Max pill, both vertically
                          centered to the optical midline of the 22px number.
                          `leading-none` on each child removes the per-element
                          line-height that was pushing the 11px pill up
                          relative to the tall input baseline. */}
                      <div className="flex shrink-0 items-center gap-2">
                        <span className="text-[15px] leading-none text-white/45">
                          Dol
                        </span>
                        <button
                          type="button"
                          onClick={onMax}
                          className="inline-flex h-8 items-center rounded-full border border-white/15 px-3 text-[11px] font-medium uppercase leading-none tracking-wider text-white/70 hover:border-white/30 hover:text-white transition-colors"
                        >
                          Max
                        </button>
                      </div>
                    </div>
                    {overBalance && (
                      <p
                        role="alert"
                        className="mt-2 flex items-center gap-1.5 text-xs text-red-400"
                      >
                        <AlertTriangle
                          className="h-3.5 w-3.5 shrink-0"
                          aria-hidden="true"
                        />
                        Amount exceeds your balance.
                      </p>
                    )}
                  </div>

                  <div className="mt-6 space-y-3">
                    {/* Instant — Plan A active. Defensive disabled fallback.
                       instantBufferShort is a transient soft-disable when
                       the vault's USDC buffer can't cover the payout; we
                       grey out the tile and surface a one-line caption so
                       the user picks Scheduled instead of hitting the raw
                       MetaMask gas error. */}
                    {instantDisabled || instantBufferShort ? (
                      <button
                        type="button"
                        disabled
                        className={`flex w-full cursor-not-allowed items-center justify-between rounded-2xl border p-5 text-left ${
                          instantBufferShort
                            ? "border-amber-400/40 bg-amber-400/[0.05]"
                            : "border-white/10 bg-white/[0.03]"
                        }`}
                      >
                        <div className="flex items-center gap-4">
                          <div
                            className={`flex h-11 w-11 items-center justify-center rounded-full ${
                              instantBufferShort
                                ? "bg-amber-400/20 text-amber-300"
                                : "bg-white/15 text-white/80"
                            }`}
                          >
                            <Zap className="h-5 w-5" />
                          </div>
                          <div>
                            <div
                              className={`text-[17px] font-semibold ${
                                instantBufferShort
                                  ? "text-amber-100"
                                  : "text-white/75"
                              }`}
                            >
                              Get it now
                            </div>
                            <div
                              className={`mt-0.5 text-xs ${
                                instantBufferShort
                                  ? "text-amber-200/90"
                                  : "text-white/55"
                              }`}
                            >
                              {instantBufferShort
                                ? "Temporarily unavailable \u00B7 use Scheduled"
                                : "Coming soon"}
                            </div>
                          </div>
                        </div>
                        <span
                          className={`text-[10px] uppercase tracking-widest ${
                            instantBufferShort
                              ? "text-amber-300"
                              : "text-white/55"
                          }`}
                        >
                          {instantBufferShort ? "Retry later" : "Soon"}
                        </span>
                      </button>
                    ) : (
                      <motion.button
                        whileHover={amountValid ? { scale: 1.01 } : undefined}
                        whileTap={amountValid ? { scale: 0.99 } : undefined}
                        transition={APPLE_SPRING}
                        onClick={() => handleSelect("instant")}
                        disabled={!amountValid}
                        className={`w-full flex items-center justify-between p-5 rounded-2xl bg-white/[0.04] border text-left transition-colors ${
                          amountValid
                            ? "border-white/10 hover:border-white/25"
                            : "border-white/5 opacity-40 cursor-not-allowed"
                        }`}
                      >
                        <div className="flex items-center gap-4">
                          <div className="flex h-11 w-11 items-center justify-center rounded-full bg-white text-black">
                            <Zap className="h-5 w-5" />
                          </div>
                          <div>
                            <div className="text-[17px] font-semibold text-white">
                              Get it now
                            </div>
                            <div className="text-xs text-white/40 mt-0.5">
                              In seconds &middot; Small fee (0.05%)
                            </div>
                          </div>
                        </div>
                        <span className="text-white/55 text-sm">&rsaquo;</span>
                      </motion.button>
                    )}

                    {/* Scheduled */}
                    <motion.button
                      whileHover={amountValid ? { scale: 1.01 } : undefined}
                      whileTap={amountValid ? { scale: 0.99 } : undefined}
                      transition={APPLE_SPRING}
                      onClick={() => handleSelect("scheduled")}
                      disabled={!amountValid}
                      className={`w-full flex items-center justify-between p-5 rounded-2xl bg-white/[0.04] border text-left transition-colors ${
                        amountValid
                          ? "border-white/10 hover:border-white/25"
                          : "border-white/5 opacity-40 cursor-not-allowed"
                      }`}
                    >
                      <div className="flex items-center gap-4">
                        <div className="flex h-11 w-11 items-center justify-center rounded-full bg-white/10 text-white border border-white/20">
                          <Clock className="h-5 w-5" />
                        </div>
                        <div>
                          <div className="text-[17px] font-semibold text-white">
                            Get it in 30 minutes
                          </div>
                          <div className="text-xs text-white/40 mt-0.5">
                            No fee
                          </div>
                        </div>
                      </div>
                      <span className="text-white/55 text-sm">&rsaquo;</span>
                    </motion.button>
                  </div>

                  <p className="mt-6 text-center text-[11px] text-white/55">
                    You can always change your mind. Cash out anytime.
                  </p>
                </>
              ) : (
                /* Confirmed */
                <motion.div
                  initial={{ opacity: 0, y: 10 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ duration: 0.4, ease: [0.05, 0.7, 0.1, 1.0] }}
                  className="text-center py-4"
                >
                  <div className="mx-auto flex h-16 w-16 items-center justify-center rounded-full bg-white">
                    <Check className="h-8 w-8 text-black" strokeWidth={3} />
                  </div>
                  <h3
                    className="mt-6 text-3xl font-bold text-white"
                    style={{ letterSpacing: "-0.03em" }}
                  >
                    {choice === "instant" ? "Done." : "Scheduled."}
                  </h3>
                  <p className="mt-3 text-[15px] text-white/50 leading-relaxed">
                    {choice === "instant"
                      ? "It's back in your wallet."
                      : "We'll let you know when it's ready."}
                  </p>
                  {choice === "scheduled" && (
                    <p className="mt-2 text-[11px] text-white/55">
                      Preview feature. Available in the next update.
                    </p>
                  )}
                  <button
                    onClick={onClose}
                    className="mt-8 px-8 py-3 rounded-full bg-white text-black font-medium text-[15px] hover:bg-white/90 transition-colors"
                  >
                    Done
                  </button>
                </motion.div>
              )}
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  );
}
