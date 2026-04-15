"use client";

type StatusType = "live" | "paused" | "offline" | "loading";

const STATUS_CONFIG: Record<
  StatusType,
  { label: string; bg: string; dot: string; text: string }
> = {
  live: {
    label: "Live",
    bg: "bg-carry-green/10",
    dot: "bg-carry-green",
    text: "text-carry-green",
  },
  paused: {
    label: "Paused",
    bg: "bg-carry-amber/10",
    dot: "bg-carry-amber",
    text: "text-carry-amber",
  },
  offline: {
    label: "Offline",
    bg: "bg-carry-red/10",
    dot: "bg-carry-red",
    text: "text-carry-red",
  },
  loading: {
    label: "Loading",
    bg: "bg-dark-surface-2",
    dot: "bg-dark-tertiary",
    text: "text-dark-tertiary",
  },
};

export function StatusPill({ status }: { status: StatusType }) {
  const config = STATUS_CONFIG[status];
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full border border-dark-border px-3 py-1 text-[12px] font-medium ${config.bg}`}
    >
      <span
        className={`h-1.5 w-1.5 rounded-full ${config.dot} ${status === "live" ? "animate-pulse" : ""}`}
      />
      <span className={config.text}>{config.label}</span>
    </span>
  );
}
