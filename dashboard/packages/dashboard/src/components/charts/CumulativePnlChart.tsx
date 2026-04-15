"use client";

import {
  AreaChart,
  Area,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  ResponsiveContainer,
} from "recharts";
import { StaleIndicator } from "@/components/common/StaleIndicator";
import { ChartSkeleton } from "@/components/common/LoadingSkeleton";

type PnlPoint = {
  time: string;
  pnl: number;
};

type CumulativePnlChartProps =
  | { state: "loaded"; data: PnlPoint[]; stale?: boolean; staleAge?: number }
  | { state: "loading" }
  | { state: "error"; message: string }
  | { state: "empty" };

export function CumulativePnlChart(props: CumulativePnlChartProps) {
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
          Cumulative Funding Earned
        </p>
        <p className="flex h-[160px] items-center justify-center text-[13px] text-dark-tertiary">
          Waiting for first funding period...
        </p>
      </div>
    );
  }

  return (
    <div className="rounded-2xl border border-dark-border bg-dark-surface p-5 transition-colors hover:border-dark-border-strong">
      <div className="flex items-center justify-between mb-3">
        <p className="text-[12px] font-medium uppercase tracking-[0.06em] text-dark-secondary">
          Cumulative Funding Earned
        </p>
        <StaleIndicator
          visible={props.stale ?? false}
          ageSeconds={props.staleAge}
        />
      </div>
      <ResponsiveContainer width="100%" height={160}>
        <AreaChart
          data={props.data}
          margin={{ top: 4, right: 4, left: -10, bottom: 0 }}
        >
          <defs>
            <linearGradient id="pnlFill" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor="#30d158" stopOpacity={0.2} />
              <stop offset="100%" stopColor="#30d158" stopOpacity={0.02} />
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
            minTickGap={60}
          />
          <YAxis
            tick={{ fontSize: 10, fill: "#86868b" }}
            tickLine={false}
            axisLine={false}
            tickFormatter={(v: number) => `$${v.toFixed(0)}`}
          />
          <Tooltip
            contentStyle={{
              backgroundColor: "#1c1c1f",
              border: "1px solid #2a2a2d",
              borderRadius: "12px",
              fontSize: "12px",
              color: "#f5f5f7",
            }}
            formatter={(value) => [`$${Number(value).toFixed(2)}`, "Funding Earned"]}
          />
          <Area
            type="monotone"
            dataKey="pnl"
            stroke="#30d158"
            fill="url(#pnlFill)"
            strokeWidth={2}
            dot={false}
          />
        </AreaChart>
      </ResponsiveContainer>
    </div>
  );
}
