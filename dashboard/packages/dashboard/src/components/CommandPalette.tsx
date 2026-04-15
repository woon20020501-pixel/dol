"use client";

import { useEffect, useRef, useState, useMemo } from "react";
import { useRouter } from "next/navigation";
import { Search } from "lucide-react";

/**
 * Cmd+K / Ctrl+K command palette.
 *
 * Opens on:
 *   - Cmd+K / Ctrl+K anywhere on the site
 *   - `?` on pages where useKeyboardShortcuts is active (reserved)
 *
 * Closes on:
 *   - Escape
 *   - Clicking the backdrop
 *   - Selecting a command
 *
 * No new dependencies — plain React state, plain kbd listener,
 * plain DOM focus management. No `cmdk`, no `@radix-ui/react-dialog`.
 * The filter is a straightforward substring search across command
 * labels; rank ties are broken by insertion order.
 *
 * This is the optional tier from Phase 2.5, shipped as part of the
 * consolidated trim commit.
 */

type Command = {
  id: string;
  label: string;
  hint?: string;
  group: string;
  action: () => void;
};

export function CommandPalette() {
  const router = useRouter();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  // Command list — keep this close to pages that actually exist on
  // the site, no dead shortcuts.
  const commands = useMemo<Command[]>(
    () => [
      {
        id: "home",
        label: "Home",
        hint: "H",
        group: "Navigate",
        action: () => router.push("/"),
      },
      {
        id: "deposit",
        label: "Buy a Dol",
        hint: "D",
        group: "Navigate",
        action: () => router.push("/deposit"),
      },
      {
        id: "my-dol",
        label: "My Dol",
        hint: "M",
        group: "Navigate",
        action: () => router.push("/my-dol"),
      },
      {
        id: "docs",
        label: "Documentation",
        group: "Navigate",
        action: () => router.push("/docs"),
      },
      {
        id: "faq",
        label: "FAQ",
        group: "Navigate",
        action: () => router.push("/faq"),
      },
      {
        id: "how-it-works",
        label: "How it works",
        group: "Docs",
        action: () => router.push("/docs/how-it-works"),
      },
      {
        id: "on-chain",
        label: "On-chain & verified",
        group: "Docs",
        action: () => router.push("/docs/trust/on-chain"),
      },
      {
        id: "risks",
        label: "Risks",
        group: "Docs",
        action: () => router.push("/docs/trust/risks"),
      },
      {
        id: "terms",
        label: "Terms of Service",
        group: "Legal",
        action: () => router.push("/legal/terms"),
      },
      {
        id: "privacy",
        label: "Privacy Policy",
        group: "Legal",
        action: () => router.push("/legal/privacy"),
      },
      {
        id: "risk",
        label: "Risk Disclosure",
        group: "Legal",
        action: () => router.push("/legal/risk"),
      },
    ],
    [router],
  );

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return commands;
    return commands.filter(
      (c) =>
        c.label.toLowerCase().includes(q) ||
        c.group.toLowerCase().includes(q),
    );
  }, [commands, query]);

  // Global open hotkey
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const isCmdK = (e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k";
      if (isCmdK) {
        e.preventDefault();
        setOpen((v) => !v);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Focus input when opened; reset on close
  useEffect(() => {
    if (open) {
      setActive(0);
      setQuery("");
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  // Clamp active cursor when filter changes
  useEffect(() => {
    if (active >= filtered.length) setActive(0);
  }, [filtered, active]);

  // In-dialog keyboard handling
  const onDialogKey = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      setOpen(false);
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((a) => Math.min(a + 1, filtered.length - 1));
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => Math.max(a - 1, 0));
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      const cmd = filtered[active];
      if (cmd) {
        cmd.action();
        setOpen(false);
      }
    }
  };

  if (!open) return null;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Command menu"
      className="fixed inset-0 z-[120] flex items-start justify-center bg-black/80 p-4 pt-[14vh] backdrop-blur"
      onClick={() => setOpen(false)}
      onKeyDown={onDialogKey}
    >
      <div
        className="w-full max-w-xl overflow-hidden rounded-2xl border border-white/10 bg-[#0a0a0a] shadow-2xl"
        onClick={(e) => e.stopPropagation()}
        style={{ boxShadow: "0 30px 80px rgba(0,0,0,0.6)" }}
      >
        <div className="flex items-center gap-3 border-b border-white/5 px-5 py-4">
          <Search className="h-4 w-4 text-white/40" aria-hidden />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Type a command or search…"
            className="w-full bg-transparent text-[15px] text-white placeholder-white/30 focus:outline-none"
            aria-label="Search commands"
          />
          <kbd className="hidden rounded border border-white/10 bg-white/5 px-2 py-0.5 text-[10px] text-white/40 md:block">
            ESC
          </kbd>
        </div>

        <div className="max-h-[50vh] overflow-y-auto p-2">
          {filtered.length === 0 ? (
            <div className="px-4 py-8 text-center text-sm text-white/40">
              No matches.
            </div>
          ) : (
            <ul role="listbox" className="flex flex-col gap-0.5">
              {filtered.map((cmd, idx) => (
                <li key={cmd.id} role="option" aria-selected={idx === active}>
                  <button
                    type="button"
                    onClick={() => {
                      cmd.action();
                      setOpen(false);
                    }}
                    onMouseEnter={() => setActive(idx)}
                    className={`flex w-full items-center justify-between rounded-xl px-4 py-2.5 text-left text-[14px] transition-colors ${
                      idx === active
                        ? "bg-white/[0.08] text-white"
                        : "text-white/70 hover:bg-white/[0.04]"
                    }`}
                  >
                    <span>
                      <span className="text-[11px] uppercase tracking-[0.14em] text-white/30">
                        {cmd.group}
                      </span>
                      <span className="ml-3">{cmd.label}</span>
                    </span>
                    {cmd.hint && (
                      <kbd className="rounded border border-white/10 bg-white/5 px-2 py-0.5 text-[10px] text-white/50">
                        {cmd.hint}
                      </kbd>
                    )}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>

        <div className="flex items-center justify-between border-t border-white/5 px-5 py-2 text-[10px] text-white/30">
          <span>↑↓ navigate · Enter select</span>
          <span>⌘K anywhere to open</span>
        </div>
      </div>
    </div>
  );
}
