"use client";

import { useState } from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { ChevronDown, Menu, X } from "lucide-react";

/**
 * Left-rail navigation for /docs/*.
 *
 * Structure is hardcoded — Phase 1 explicitly doesn't need
 * filesystem-driven generation. When a doc is added or moved, edit this
 * file by hand. Keeping it static keeps the sidebar free of runtime
 * dependencies and makes the active-item logic a simple path compare.
 *
 * Mobile collapses to a hamburger menu that opens a full-height drawer.
 * Desktop is a fixed left column at w-64.
 */

type NavLeaf = { label: string; href: string };
type NavGroup = { label: string; children: NavLeaf[] };
type NavItem = NavLeaf | NavGroup;

const NAV: NavItem[] = [
  {
    label: "Getting started",
    children: [
      { label: "What is Dol", href: "/docs/getting-started/what-is-dol" },
      { label: "How to buy a Dol", href: "/docs/getting-started/how-to-buy" },
      {
        label: "Where Dol is available",
        href: "/docs/getting-started/supported-countries",
      },
    ],
  },
  { label: "How it works", href: "/docs/how-it-works" },
  {
    label: "Trust",
    children: [
      { label: "On-chain & verified", href: "/docs/trust/on-chain" },
      { label: "Architecture", href: "/docs/trust/architecture" },
      { label: "Strategy paper", href: "/docs/trust/strategy-paper" },
      {
        label: "Framework assumptions",
        href: "/docs/trust/framework-assumptions",
      },
      { label: "Risks", href: "/docs/trust/risks" },
    ],
  },
  { label: "FAQ", href: "/docs/faq" },
  {
    label: "More",
    children: [
      { label: "Support", href: "/docs/more/support" },
      { label: "Legal", href: "/docs/more/legal" },
    ],
  },
];

function isGroup(item: NavItem): item is NavGroup {
  return (item as NavGroup).children !== undefined;
}

export function DocsSidebar() {
  const pathname = usePathname();
  const [mobileOpen, setMobileOpen] = useState(false);

  // Collapsed sections default to open if they contain the active page,
  // otherwise open if they contain any entry (keep the tree fully
  // visible until the user chooses to collapse).
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});

  const toggle = (label: string) =>
    setCollapsed((s) => ({ ...s, [label]: !s[label] }));

  const content = (
    <nav className="flex flex-col gap-6" aria-label="Documentation">
      <Link
        href="/docs"
        className="text-xs uppercase tracking-[0.14em] text-white/40 hover:text-white/70"
      >
        Documentation
      </Link>

      {NAV.map((item) => {
        if (!isGroup(item)) {
          const active = pathname === item.href;
          return (
            <NavItemLink
              key={item.label}
              href={item.href}
              label={item.label}
              active={active}
              topLevel
              onNavigate={() => setMobileOpen(false)}
            />
          );
        }

        const isCollapsed = collapsed[item.label] ?? false;
        const hasActive = item.children.some((c) => pathname === c.href);

        return (
          <div key={item.label} className="flex flex-col gap-2">
            <button
              type="button"
              onClick={() => toggle(item.label)}
              className="flex items-center justify-between text-left text-xs uppercase tracking-[0.14em] text-white/40 hover:text-white/70"
            >
              <span>{item.label}</span>
              <ChevronDown
                className={`h-3.5 w-3.5 transition-transform ${
                  isCollapsed ? "-rotate-90" : "rotate-0"
                }`}
              />
            </button>
            {!isCollapsed && (
              <div
                className={`flex flex-col gap-1 pl-0 ${
                  hasActive ? "" : ""
                }`}
              >
                {item.children.map((c) => (
                  <NavItemLink
                    key={c.href}
                    href={c.href}
                    label={c.label}
                    active={pathname === c.href}
                    onNavigate={() => setMobileOpen(false)}
                  />
                ))}
              </div>
            )}
          </div>
        );
      })}
    </nav>
  );

  return (
    <>
      {/* Mobile trigger — only visible below md */}
      <button
        type="button"
        onClick={() => setMobileOpen(true)}
        className="fixed bottom-6 right-6 z-40 flex h-12 w-12 items-center justify-center rounded-full border border-white/15 bg-[#0a0a0a] text-white shadow-2xl md:hidden"
        aria-label="Open documentation navigation"
      >
        <Menu className="h-5 w-5" />
      </button>

      {/* Mobile drawer — overlay is presentational; the actual dialog
          role lives on the inner panel. Satisfies jsx-a11y rules that
          forbid assigning click handlers to non-interactive roles. */}
      {mobileOpen && (
        <div
          className="fixed inset-0 z-50 bg-black/90 backdrop-blur md:hidden"
          onClick={() => setMobileOpen(false)}
          aria-hidden="true"
        >
          {/* stop-propagation is pointer-only (prevents overlay dismiss
              when tapping inside). No keyboard parity required — there
              is no keyboard equivalent of "tap inside to not-close". */}
          {/* eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions, jsx-a11y/click-events-have-key-events */}
          <div
            role="dialog"
            aria-modal="true"
            aria-label="Documentation navigation"
            className="h-full w-72 overflow-y-auto border-r border-white/10 bg-[#0a0a0a] p-6"
            onClick={(e) => e.stopPropagation()}
          >
            <button
              type="button"
              onClick={() => setMobileOpen(false)}
              className="mb-6 flex h-8 w-8 items-center justify-center rounded-full text-white/60 hover:bg-white/5 hover:text-white"
              aria-label="Close"
            >
              <X className="h-4 w-4" />
            </button>
            {content}
          </div>
        </div>
      )}

      {/* Desktop fixed sidebar */}
      <aside className="sticky top-0 hidden h-screen w-64 shrink-0 overflow-y-auto border-r border-white/5 bg-black/40 px-8 py-12 md:block">
        {content}
      </aside>
    </>
  );
}

function NavItemLink({
  href,
  label,
  active,
  topLevel,
  onNavigate,
}: {
  href: string;
  label: string;
  active: boolean;
  topLevel?: boolean;
  onNavigate?: () => void;
}) {
  const base = topLevel
    ? "text-xs uppercase tracking-[0.14em]"
    : "text-[13px] leading-relaxed";
  return (
    <Link
      href={href}
      onClick={onNavigate}
      className={`${base} -ml-3 border-l-2 pl-3 transition-colors ${
        active
          ? "border-white text-white"
          : "border-transparent text-white/50 hover:text-white/90"
      }`}
    >
      {label}
    </Link>
  );
}
