"use client";

import { useEffect, useState } from "react";
import { Skeleton } from "@/components/ui/skeleton";
import { Check, Copy, ExternalLink, ShieldCheck } from "lucide-react";
import { useNavReporter } from "@/hooks/useNavReporter";
import { formatUsd } from "@/lib/format";

const BASESCAN = "https://sepolia.basescan.org";

export function NavReporterCard() {
  const r = useNavReporter();

  if (r.isLoading) {
    return <ReporterSkeleton />;
  }

  return (
    <div className="rounded-2xl border border-dark-border bg-dark-surface p-5 transition-colors hover:border-dark-border-strong">
      <div className="flex items-center justify-between mb-4">
        <span className="flex items-center gap-1.5 text-[12px] font-medium uppercase tracking-[0.06em] text-dark-secondary">
          <ShieldCheck
            className="h-3.5 w-3.5 text-senior"
            aria-hidden="true"
          />
          NAV Reporter
        </span>
        <ReporterPill status={r.status} />
      </div>

      <div className="space-y-2.5 text-[12px]">
        <Row label="Operator">
          {r.operatorAddress ? (
            <CopyableAddress address={r.operatorAddress} />
          ) : (
            <span className="text-dark-tertiary">&mdash;</span>
          )}
        </Row>

        <Row label="Last report">
          {r.lastReportTimestamp ? (
            <RelativeTime ts={r.lastReportTimestamp} />
          ) : (
            <span className="text-dark-tertiary">never</span>
          )}
        </Row>

        <Row label="Last NAV">
          {r.lastReportNav !== null ? (
            <span className="font-mono text-dark-primary">
              {formatUsd(r.lastReportNav)}
            </span>
          ) : (
            <span className="text-dark-tertiary">&mdash;</span>
          )}
        </Row>

        <Row label="Last tx">
          {r.lastReportTxHash ? (
            <a
              href={`${BASESCAN}/tx/${r.lastReportTxHash}`}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-1 font-mono text-senior underline decoration-senior/30 hover:decoration-senior focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-senior focus-visible:ring-offset-1 focus-visible:ring-offset-dark-bg"
            >
              {truncate(r.lastReportTxHash, 6, 4)}
              <ExternalLink className="h-3 w-3" aria-hidden="true" />
            </a>
          ) : (
            <span className="text-dark-tertiary">&mdash;</span>
          )}
        </Row>

        <Row label="Next">
          {r.nextReportInSec !== null ? (
            <span className="font-mono text-dark-primary">
              in {formatCountdown(r.nextReportInSec)}
            </span>
          ) : (
            <span className="text-dark-tertiary">&mdash;</span>
          )}
        </Row>

        {r.status === "error" && r.errorMessage ? (
          <p
            role="alert"
            className="mt-2 rounded-xl border border-carry-red/30 bg-carry-red/10 p-2.5 text-[11px] text-carry-red"
          >
            {r.errorMessage}
          </p>
        ) : null}

        {!r.isAvailable && r.status === "never" ? (
          <p className="mt-2 text-[11px] text-dark-tertiary">
            Bot reporter offline — NAV will be reported when the operator
            comes online.
          </p>
        ) : null}
      </div>
    </div>
  );
}

function Row({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-2">
      <span className="text-dark-secondary">{label}</span>
      <span className="truncate text-right">{children}</span>
    </div>
  );
}

function ReporterPill({ status }: { status: "live" | "dry-run" | "error" | "never" }) {
  const cfg = {
    live: { dot: "bg-carry-green animate-pulse", text: "text-carry-green", label: "live" },
    "dry-run": { dot: "bg-carry-amber", text: "text-carry-amber", label: "dry-run" },
    error: { dot: "bg-carry-red", text: "text-carry-red", label: "error" },
    never: { dot: "bg-dark-tertiary", text: "text-dark-tertiary", label: "idle" },
  }[status];

  return (
    <span className="inline-flex items-center gap-1 rounded-full border border-dark-border bg-dark-surface-2 px-2 py-0.5 text-[10px] font-medium">
      <span
        className={`inline-block h-1.5 w-1.5 rounded-full ${cfg.dot}`}
        aria-hidden="true"
      />
      <span className={cfg.text}>{cfg.label}</span>
    </span>
  );
}

function CopyableAddress({ address }: { address: string }) {
  const [copied, setCopied] = useState(false);

  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(address);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // ignore
    }
  };

  return (
    <span className="inline-flex items-center gap-1">
      <span className="font-mono text-dark-primary">
        {truncate(address, 6, 4)}
      </span>
      <button
        type="button"
        onClick={onCopy}
        aria-label={copied ? "Copied" : "Copy operator address"}
        className="rounded p-0.5 text-dark-tertiary transition-colors hover:text-dark-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-senior focus-visible:ring-offset-1 focus-visible:ring-offset-dark-bg"
      >
        {copied ? (
          <Check className="h-3 w-3 text-carry-green" aria-hidden="true" />
        ) : (
          <Copy className="h-3 w-3" aria-hidden="true" />
        )}
      </button>
    </span>
  );
}

function RelativeTime({ ts }: { ts: number }) {
  const [, tick] = useState(0);
  useEffect(() => {
    const id = setInterval(() => tick((n) => n + 1), 30_000);
    return () => clearInterval(id);
  }, []);

  const ageSec = Math.max(0, Math.floor(Date.now() / 1000 - ts));
  return <span className="font-mono text-dark-primary">{formatAge(ageSec)} ago</span>;
}

function ReporterSkeleton() {
  return (
    <div className="rounded-2xl border border-dark-border bg-dark-surface p-5">
      <Skeleton className="h-3 w-32 bg-dark-surface-2" />
      <div className="mt-4 space-y-2.5">
        {[...Array(5)].map((_, i) => (
          <div key={i} className="flex justify-between">
            <Skeleton className="h-3 w-16 bg-dark-surface-2" />
            <Skeleton className="h-3 w-20 bg-dark-surface-2" />
          </div>
        ))}
      </div>
    </div>
  );
}

function truncate(addr: string, head = 6, tail = 4): string {
  if (addr.length <= head + tail + 2) return addr;
  return `${addr.slice(0, head)}\u2026${addr.slice(-tail)}`;
}

function formatAge(sec: number): string {
  if (sec < 60) return `${sec}s`;
  if (sec < 3600) return `${Math.floor(sec / 60)}m`;
  if (sec < 86400) return `${Math.floor(sec / 3600)}h`;
  return `${Math.floor(sec / 86400)}d`;
}

function formatCountdown(sec: number): string {
  if (sec <= 0) return "now";
  const m = Math.floor(sec / 60);
  const s = sec % 60;
  if (m === 0) return `${s}s`;
  return `${m}m ${s.toString().padStart(2, "0")}s`;
}
