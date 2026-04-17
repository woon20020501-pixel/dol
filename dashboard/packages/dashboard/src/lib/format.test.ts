import { describe, it, expect } from "vitest";
import {
  formatUsd,
  formatUsdCompact,
  formatPct,
  formatBps,
  formatSharePrice,
  pnlColor,
} from "./format";

describe("formatUsd", () => {
  it("shows cents below $1000", () => {
    expect(formatUsd(12.34)).toBe("$12.34");
    expect(formatUsd(999.99)).toBe("$999.99");
  });

  it("drops cents at $1000+ for readability", () => {
    expect(formatUsd(1000)).toBe("$1,000");
    expect(formatUsd(10500)).toBe("$10,500");
    expect(formatUsd(1_234_567)).toBe("$1,234,567");
  });

  it("handles zero", () => {
    expect(formatUsd(0)).toBe("$0");
  });

  it("handles small fractions", () => {
    expect(formatUsd(0.01)).toBe("$0.01");
    expect(formatUsd(0.05)).toBe("$0.05");
  });
});

describe("formatUsdCompact", () => {
  it("uses K suffix at $1000+", () => {
    expect(formatUsdCompact(1_000)).toBe("$1.0K");
    expect(formatUsdCompact(28_500)).toBe("$28.5K");
    expect(formatUsdCompact(999_999)).toBe("$1000.0K");
  });

  it("uses M suffix at $1M+", () => {
    expect(formatUsdCompact(1_000_000)).toBe("$1.0M");
    expect(formatUsdCompact(12_500_000)).toBe("$12.5M");
  });

  it("falls back to plain USD below $1000", () => {
    expect(formatUsdCompact(100)).toBe("$100");
    expect(formatUsdCompact(999)).toBe("$999");
  });
});

describe("formatPct", () => {
  it("adds + sign for positive", () => {
    expect(formatPct(7.5)).toBe("+7.5%");
    expect(formatPct(0.1)).toBe("+0.1%");
  });

  it("keeps - sign for negative (no duplicate +)", () => {
    expect(formatPct(-3.2)).toBe("-3.2%");
  });

  it("no sign on zero", () => {
    expect(formatPct(0)).toBe("0.0%");
  });

  it("respects custom decimals", () => {
    expect(formatPct(7.1234, 3)).toBe("+7.123%");
    expect(formatPct(7.5, 0)).toBe("+8%");
  });
});

describe("formatBps", () => {
  it("adds + sign and bps suffix", () => {
    expect(formatBps(18.92)).toBe("+18.92 bps");
  });

  it("keeps - sign", () => {
    expect(formatBps(-0.5)).toBe("-0.50 bps");
  });

  it("always 2 decimals for bps precision", () => {
    expect(formatBps(100)).toBe("+100.00 bps");
  });
});

describe("formatSharePrice", () => {
  it("always 4 decimals (ERC-4626 convention)", () => {
    expect(formatSharePrice(1)).toBe("1.0000");
    expect(formatSharePrice(1.2345678)).toBe("1.2346");
    expect(formatSharePrice(3.2731)).toBe("3.2731");
  });

  it("handles zero", () => {
    expect(formatSharePrice(0)).toBe("0.0000");
  });
});

describe("pnlColor", () => {
  it("green for positive", () => {
    expect(pnlColor(1)).toBe("text-carry-green");
    expect(pnlColor(0.01)).toBe("text-carry-green");
  });

  it("red for negative", () => {
    expect(pnlColor(-1)).toBe("text-carry-red");
    expect(pnlColor(-0.01)).toBe("text-carry-red");
  });

  it("muted for zero", () => {
    expect(pnlColor(0)).toBe("text-muted-foreground");
  });
});
