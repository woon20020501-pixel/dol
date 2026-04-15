import fs from "node:fs";
import path from "node:path";
import { LegalPageShell } from "@/components/LegalPageShell";
import { renderMarkdown } from "@/lib/markdown";

export const metadata = {
  title: "Risk Disclosure · Dol",
  description: "Dol Risk Disclosure — pre-launch draft v0.1.",
};

export default function RiskPage() {
  const md = fs.readFileSync(
    path.join(process.cwd(), "src/content/legal/risk.md"),
    "utf8",
  );
  return <LegalPageShell title="Risk">{renderMarkdown(md)}</LegalPageShell>;
}
