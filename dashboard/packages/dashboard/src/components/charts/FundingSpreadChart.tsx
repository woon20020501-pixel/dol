"use client";

import {
  AreaChart,
  Area,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  ResponsiveContainer,
  Legend,
} from "recharts";
import { StaleIndicator } from "@/components/common/StaleIndicator";
import { ChartSkeleton } from "@/components/common/LoadingSkeleton";

type FundingPoint = {
  time: string;
  pacifica: number;
  lighter: number;
  spread: number;
};

type FundingSpreadChartProps =
  | { state: "loaded"; data: FundingPoint[]; stale?: boolean; staleAge?: number }
  | { state: "loading" }
  | { state: "error"; message: string }
  | { state: "empty" };

export function FundingSpreadChart(props: FundingSpreadChartProps) {
  if (props.state === "loading") {
    return <ChartSkeleton />;
  }

  if (props.state === "error") {
    return (
      <div className="rounded-2xl border border-dark-border bg-dark-surface p-5">
        <p className="flex h-[160px] items-center justify-center text-[13px] text-carry-red">
          {props.message}
        </p>
      </div>
    );
  }

  if (props.state === "empty") {
    return (
      <div className="rounded-2xl border border-dark-border bg-dark-surface p-5">
        <p className="text-[12px] font-medium uppercase tracking-[0.06em] text-dark-secondary">
          Funding Spread (24h)
        </p>
        <p className="flex h-[160px] items-center justify-center text-[13px] text-dark-tertiary">
          Waiting for first funding observation...
        </p>
      </div>
    );
  }

  return (
    <div className="rounded-2xl border border-dark-border bg-dark-surface p-5 transition-colors hover:border-dark-border-strong">
      <div className="flex items-center justify-between mb-3">
        <p className="text-[12px] font-medium uppercase tracking-[0.06em] text-dark-secondary">
          Funding Spread (24h)
        </p>
        <StaleIndicator
          visible={props.stale ?? false}
          ageSeconds={props.staleAge}
        />
      </div>
      <ResponsiveContainer width="100%" height={160}>
        <AreaChart
          data={props.data}
          margin={{ top: 4, right: 4, left: -20, bottom: 0 }}
        >
          <defs>
            <linearGradient id="spreadFill" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor="#2dd4bf" stopOpacity={0.2} />
              <stop offset="100%" stopColor="#2dd4bf" stopOpacity={0.02} />
            </linearGradient>
          </defs>
          <CartesianGrid
            strokeDasharray="3 3"
            stroke="#2a2a2d"
            vertical={false}
          />
          <XAxis
            dataKey="time"
            tick={{ fontSize: 10, fill: "#86868b" }}
            tickLine={false}
            axisLine={false}
            interval="preserveStartEnd"
            minTickGap={40}
          />
          <YAxis
            tick={{ fontSize: 10, fill: "#86868b" }}
            tickLine={false}
            axisLine={false}
            tickFormatter={(v: number) => `${v.toFixed(1)}`}
            domain={["dataMin - 0.3", "dataMax + 0.3"]}
          />
          <Tooltip
            contentStyle={{
              backgroundColor: "#1c1c1f",
              border: "1px solid #2a2a2d",
              borderRadius: "12px",
              fontSize: "12px",
              color: "#f5f5f7",
            }}
            formatter={(value, name) => [
              `${Number(value).toFixed(3)} bps`,
              name === "pacifica"
                ? "Pacifica"
                : name === "lighter"
                  ? "Lighter"
                  : "Spread",
            ]}
          />
          <Legend
            wrapperStyle={{ fontSize: "11px", paddingTop: "4px" }}
            formatter={(value: string) =>
              value === "pacifica"
                ? "Pacifica"
                : value === "lighter"
                  ? "Lighter"
                  : "Spread"
            }
          />
          <Area
            type="monotone"
            dataKey="spread"
            stroke="#2dd4bf"
            fill="url(#spreadFill)"
            strokeWidth={1.5}
            dot={false}
          />
          <Area
            type="monotone"
            dataKey="pacifica"
            stroke="#2dd4bf"
            fill="none"
            strokeWidth={1.5}
            dot={false}
          />
          <Area
            type="monotone"
            dataKey="lighter"
            stroke="#f59e0b"
            fill="none"
            strokeWidth={1.5}
            strokeDasharray="4 2"
            dot={false}
          />
        </AreaChart>
      </ResponsiveContainer>
    </div>
  );
}
