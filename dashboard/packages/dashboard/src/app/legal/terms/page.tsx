import fs from "node:fs";
import path from "node:path";
import { LegalPageShell } from "@/components/LegalPageShell";
import { renderMarkdown } from "@/lib/markdown";

export const metadata = {
  title: "Terms of Service · Dol",
  description: "Dol Terms of Service — pre-launch draft v0.1.",
};

export default function TermsPage() {
  const md = fs.readFileSync(
    path.join(process.cwd(), "src/content/legal/terms.md"),
    "utf8",
  );
  return <LegalPageShell title="Terms">{renderMarkdown(md)}</LegalPageShell>;
}
