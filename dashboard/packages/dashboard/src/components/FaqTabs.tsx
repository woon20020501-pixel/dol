"use client";

import { useState, useMemo } from "react";
import Link from "next/link";
import { renderMarkdown } from "@/lib/markdown";
import type { FaqCategory } from "@/lib/faq";

/**
 * Tabbed FAQ view — Liminal pattern.
 *
 *   [All] [General] [Buying & Holding] [Cashing Out] [Safety & Trust] ...
 *   ── underline under active tab ──
 *
 *   ### Question (anchor link)
 *   <answer paragraphs rendered through lib/markdown.tsx>
 *
 * No accordion, no collapse. Users skim or Cmd-F. No search bar in
 * Phase 1. Questions get stable id anchors so people can
 * deep-link to a specific Q&A.
 *
 * Markdown body for each answer is rendered via the shared
 * `renderMarkdown` helper, so the same inline link / bold / italic /
 * code / list / blockquote semantics used on /docs and /legal work
 * here too.
 */

interface FaqTabsProps {
  categories: FaqCategory[];
}

const ALL_TAB = "all";

export function FaqTabs({ categories }: FaqTabsProps) {
  const [active, setActive] = useState<string>(ALL_TAB);

  // Flat list of every question in source order — used when "All" is
  // active. Memoised so the list doesn't rebuild on every tab click.
  const allQuestions = useMemo(
    () =>
      categories.flatMap((c) =>
        c.questions.map((q) => ({ ...q, categoryLabel: c.label })),
      ),
    [categories],
  );

  const visible =
    active === ALL_TAB
      ? allQuestions
      : categories.find((c) => c.id === active)?.questions.map((q) => ({
          ...q,
          categoryLabel: "",
        })) ?? [];

  return (
    <div className="mt-10">
      {/* Tab row */}
      <div
        className="-mx-4 flex gap-x-6 gap-y-2 overflow-x-auto px-4 pb-3 md:mx-0 md:flex-wrap md:overflow-visible md:px-0"
        role="tablist"
        aria-label="FAQ categories"
      >
        <TabButton
          label="All"
          active={active === ALL_TAB}
          onClick={() => setActive(ALL_TAB)}
        />
        {categories.map((c) => (
          <TabButton
            key={c.id}
            label={c.label}
            active={active === c.id}
            onClick={() => setActive(c.id)}
          />
        ))}
      </div>

      {/* Question list */}
      <div className="mt-10 space-y-12">
        {visible.map((q) => (
          <section
            key={`${active}-${q.id}`}
            id={q.id}
            aria-labelledby={`q-${q.id}`}
            className="scroll-mt-24"
          >
            <h3
              id={`q-${q.id}`}
              className="group text-xl font-semibold text-white"
              style={{ letterSpacing: "-0.01em" }}
            >
              <Link
                href={`#${q.id}`}
                className="hover:underline decoration-white/30 underline-offset-4"
              >
                {q.question}
              </Link>
            </h3>
            <div className="mt-2">{renderMarkdown(q.answer)}</div>
          </section>
        ))}
      </div>
    </div>
  );
}

function TabButton({
  label,
  active,
  onClick,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      onClick={onClick}
      className={`shrink-0 border-b-2 pb-2 text-[13px] font-medium transition-colors ${
        active
          ? "border-white text-white"
          : "border-transparent text-white/40 hover:text-white/70"
      }`}
    >
      {label}
    </button>
  );
}
