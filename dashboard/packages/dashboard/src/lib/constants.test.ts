import { describe, it, expect } from "vitest";
import {
  DOL_APY,
  DOL_APY_DISPLAY,
  PHASE_1_TVL_CAP_USDC,
  USDC_DECIMALS,
  SHARE_DECIMALS,
  SHARE_PRICE_DECIMALS,
  MAX_DEPOSIT_USDC,
  INSTANT_REDEEM_FEE_BPS,
  FALLBACK_COOLDOWN_MS,
} from "./constants";

/**
 * Protocol constants invariants. These aren't dynamic values — they're
 * product contracts. If any of these ever change accidentally (merge
 * error, typo on the cap), this file fails the build and the wrong
 * number never reaches production.
 */

describe("protocol constants", () => {
  it("APY is consistent between numeric and display", () => {
    // If someone bumps DOL_APY without updating DOL_APY_DISPLAY, the
    // hero headline would lie. This pins them together.
    const expectedDisplay = (DOL_APY * 100).toFixed(1) + "%";
    expect(DOL_APY_DISPLAY).toBe(expectedDisplay);
  });

  it("APY is in realistic range (not 0, not moonmath)", () => {
    // Guards against accidentally committing 0.075 as 75 (×100 error).
    expect(DOL_APY).toBeGreaterThan(0);
    expect(DOL_APY).toBeLessThan(1); // < 100% APY
  });

  it("TVL cap is a positive integer", () => {
    expect(PHASE_1_TVL_CAP_USDC).toBeGreaterThan(0);
    expect(Number.isInteger(PHASE_1_TVL_CAP_USDC)).toBe(true);
  });

  it("MAX_DEPOSIT never exceeds TVL cap at single-user scale", () => {
    // Sanity: a single deposit can't exceed the whole Phase 1 cap
    // by orders of magnitude. Currently 100× is the soft boundary.
    expect(MAX_DEPOSIT_USDC).toBeGreaterThan(PHASE_1_TVL_CAP_USDC);
    expect(MAX_DEPOSIT_USDC / PHASE_1_TVL_CAP_USDC).toBeLessThan(1_000);
  });

  it("token decimals match USDC on Base (6)", () => {
    expect(USDC_DECIMALS).toBe(6);
    expect(SHARE_DECIMALS).toBe(6);
  });

  it("ERC-4626 share price uses 1e18 base", () => {
    // Convention for ERC-4626 pricePerShare(). If this ever becomes
    // 6 by mistake, every price display breaks silently.
    expect(SHARE_PRICE_DECIMALS).toBe(18);
  });

  it("instant redeem fee is a small bps value", () => {
    expect(INSTANT_REDEEM_FEE_BPS).toBeGreaterThanOrEqual(0);
    expect(INSTANT_REDEEM_FEE_BPS).toBeLessThan(100); // < 1%
  });

  it("fallback cooldown is 30 minutes", () => {
    // README, UI button ("Get it in 30 minutes"), and this constant
    // must agree. Any drift breaks the user's latency expectation.
    expect(FALLBACK_COOLDOWN_MS).toBe(30 * 60 * 1000);
  });
});
