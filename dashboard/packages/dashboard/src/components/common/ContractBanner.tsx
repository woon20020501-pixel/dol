"use client";

import { Info } from "lucide-react";

export function ContractBanner({ deployed }: { deployed: boolean }) {
  if (deployed) return null;

  return (
    <div
      role="status"
      className="mb-4 flex items-center gap-2 rounded-xl border border-senior/30 bg-senior/5 px-4 py-2.5 text-[13px] text-senior"
    >
      <Info className="h-4 w-4 shrink-0" aria-hidden="true" />
      <span>
        Contract not deployed yet. Showing demo data. Vault reads will
        activate automatically once the contract is deployed.
      </span>
    </div>
  );
}
