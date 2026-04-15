/** Format a number as USD currency */
export function formatUsd(value: number): string {
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 0,
    maximumFractionDigits: value >= 1000 ? 0 : 2,
  }).format(value);
}

/** Format a number as compact USD (e.g. $28.5K) */
export function formatUsdCompact(value: number): string {
  if (value >= 1_000_000) {
    return `$${(value / 1_000_000).toFixed(1)}M`;
  }
  if (value >= 1_000) {
    return `$${(value / 1_000).toFixed(1)}K`;
  }
  return formatUsd(value);
}

/** Format as percentage with sign */
export function formatPct(value: number, decimals = 1): string {
  const sign = value > 0 ? "+" : "";
  return `${sign}${value.toFixed(decimals)}%`;
}

/** Format basis points */
export function formatBps(value: number): string {
  const sign = value > 0 ? "+" : "";
  return `${sign}${value.toFixed(2)} bps`;
}

/** Format share price */
export function formatSharePrice(value: number): string {
  return value.toFixed(4);
}

/** Format PnL with sign and color class */
export function pnlColor(value: number): string {
  if (value > 0) return "text-carry-green";
  if (value < 0) return "text-carry-red";
  return "text-muted-foreground";
}
