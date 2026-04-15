"use client";

import { StaleIndicator } from "@/components/common/StaleIndicator";
import { TableSkeleton } from "@/components/common/LoadingSkeleton";
import { formatUsd, formatPct, pnlColor } from "@/lib/format";

type Position = {
  venue: string;
  symbol: string;
  side: "LONG" | "SHORT";
  notionalUsd: number;
  entryPrice: number;
  unrealizedPnl: number;
  fundingRate8h?: number;
  fundingApy?: number;
};

type PositionTableProps =
  | {
      state: "loaded";
      pacifica: Position | null;
      hedge: Position | null;
      stale?: boolean;
      staleAge?: number;
    }
  | { state: "loading" }
  | { state: "error"; message: string }
  | { state: "empty" };

export function PositionTable(props: PositionTableProps) {
  if (props.state === "loading") {
    return <TableSkeleton />;
  }

  if (props.state === "error") {
    return (
      <div className="rounded-2xl border border-dark-border bg-dark-surface p-5">
        <p className="flex h-[100px] items-center justify-center text-[13px] text-carry-red">
          {props.message}
        </p>
      </div>
    );
  }

  if (props.state === "empty") {
    return (
      <div className="rounded-2xl border border-dark-border bg-dark-surface p-5">
        <p className="text-[12px] font-medium uppercase tracking-[0.06em] text-dark-secondary">
          Positions
        </p>
        <p className="flex h-[80px] items-center justify-center text-[13px] text-dark-tertiary">
          Bot is observing the market. Positions will appear when a carry opportunity is captured.
        </p>
      </div>
    );
  }

  const { pacifica, hedge } = props;
  const rows = [
    pacifica ? { ...pacifica, venue: "Pacifica" } : null,
    hedge ? { ...hedge } : null,
  ].filter(Boolean) as Position[];

  return (
    <div className="rounded-2xl border border-dark-border bg-dark-surface p-5 transition-colors hover:border-dark-border-strong">
      <div className="flex items-center justify-between mb-3">
        <p className="text-[12px] font-medium uppercase tracking-[0.06em] text-dark-secondary">
          Positions
        </p>
        <StaleIndicator
          visible={props.stale ?? false}
          ageSeconds={props.staleAge}
        />
      </div>
      <div className="overflow-x-auto">
        <table className="w-full text-[13px]" role="table">
          <thead>
            <tr className="border-b border-dark-border text-left text-[11px] uppercase tracking-[0.06em] text-dark-secondary">
              <th className="px-3 py-2">Venue</th>
              <th className="px-3 py-2">Pair</th>
              <th className="px-3 py-2">Side</th>
              <th className="px-3 py-2 text-right">Notional</th>
              <th className="px-3 py-2 text-right">Entry</th>
              <th className="px-3 py-2 text-right">Unreal.</th>
              <th className="hidden px-3 py-2 sm:table-cell text-right">
                Funding APY
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr
                key={row.venue}
                className="border-b border-dark-border/50 last:border-0 transition-colors hover:bg-dark-surface-2"
              >
                <td className="px-3 py-2.5 font-medium text-dark-primary">
                  {row.venue}
                </td>
                <td className="px-3 py-2.5 font-mono text-dark-primary">{row.symbol}</td>
                <td className="px-3 py-2.5">
                  <span
                    className={
                      row.side === "LONG"
                        ? "text-carry-green font-medium"
                        : "text-carry-red font-medium"
                    }
                  >
                    {row.side}
                  </span>
                </td>
                <td className="px-3 py-2.5 text-right font-mono text-dark-primary">
                  {formatUsd(row.notionalUsd)}
                </td>
                <td className="px-3 py-2.5 text-right font-mono text-dark-primary">
                  {row.entryPrice.toFixed(2)}
                </td>
                <td
                  className={`px-3 py-2.5 text-right font-mono ${pnlColor(row.unrealizedPnl)}`}
                >
                  {row.unrealizedPnl >= 0 ? "+" : ""}
                  {formatUsd(row.unrealizedPnl)}
                </td>
                <td className="hidden px-3 py-2.5 text-right font-mono sm:table-cell text-dark-primary">
                  {row.fundingApy !== undefined
                    ? formatPct(row.fundingApy)
                    : "\u2014"}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
