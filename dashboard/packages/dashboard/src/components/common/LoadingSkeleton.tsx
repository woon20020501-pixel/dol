"use client";

import { Skeleton } from "@/components/ui/skeleton";

export function ChartSkeleton() {
  return (
    <div className="rounded-2xl border border-dark-border bg-dark-surface p-5">
      <Skeleton className="h-4 w-32 bg-dark-surface-2" />
      <div className="mt-4 flex items-end gap-1 h-[160px]">
        {[...Array(12)].map((_, i) => (
          <Skeleton
            key={i}
            className="flex-1 rounded-t bg-dark-surface-2"
            style={{ height: `${30 + ((i * 37) % 70)}%` }}
          />
        ))}
      </div>
    </div>
  );
}

export function TableSkeleton() {
  return (
    <div className="rounded-2xl border border-dark-border bg-dark-surface p-5">
      <Skeleton className="h-4 w-28 bg-dark-surface-2" />
      <div className="mt-4 space-y-2">
        <Skeleton className="h-8 w-full bg-dark-surface-2" />
        <Skeleton className="h-8 w-full bg-dark-surface-2" />
        <Skeleton className="h-8 w-full bg-dark-surface-2" />
      </div>
    </div>
  );
}

export function VaultCardSkeleton() {
  return (
    <div className="rounded-2xl border border-dark-border bg-dark-surface p-5">
      <Skeleton className="h-4 w-20 bg-dark-surface-2" />
      <div className="mt-4 space-y-3">
        <Skeleton className="h-10 w-full bg-dark-surface-2" />
        <Skeleton className="h-10 w-full bg-dark-surface-2" />
      </div>
    </div>
  );
}
