import fs from "node:fs";
import path from "node:path";
import { LegalPageShell } from "@/components/LegalPageShell";
import { renderMarkdown } from "@/lib/markdown";

export const metadata = {
  title: "Privacy Policy · Dol",
  description: "Dol Privacy Policy — pre-launch draft v0.1.",
};

export default function PrivacyPage() {
  const md = fs.readFileSync(
    path.join(process.cwd(), "src/content/legal/privacy.md"),
    "utf8",
  );
  return <LegalPageShell title="Privacy">{renderMarkdown(md)}</LegalPageShell>;
}
