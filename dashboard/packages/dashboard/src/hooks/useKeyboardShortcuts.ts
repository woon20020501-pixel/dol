"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";

/**
 * Binds keyboard shortcuts. Only active when the user isn't typing
 * in an input/textarea/contenteditable.
 *
 * Keys:
 *   d  → /deposit
 *   m  → /my-dol
 *   h  → /
 *   ?  → (reserved for shortcuts help — future)
 *   ESC → lets parent components handle (we don't preventDefault)
 */
export function useKeyboardShortcuts() {
  const router = useRouter();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (!target) return;

      // Don't intercept when user is typing
      const tag = target.tagName.toLowerCase();
      if (
        tag === "input" ||
        tag === "textarea" ||
        tag === "select" ||
        target.isContentEditable
      ) {
        return;
      }

      // Ignore when modifier keys are pressed (don't fight Cmd+D etc.)
      if (e.metaKey || e.ctrlKey || e.altKey) return;

      // Ignore during IME composition (Korean/Chinese/Japanese input).
      // A composition-in-progress keystroke would otherwise fire d/m/h
      // while the user is typing Hangul, hijacking focus.
      if (e.isComposing || e.keyCode === 229) return;

      switch (e.key) {
        case "d":
        case "D":
          e.preventDefault();
          router.push("/deposit");
          break;
        case "m":
        case "M":
          e.preventDefault();
          router.push("/my-dol");
          break;
        case "h":
        case "H":
          e.preventDefault();
          router.push("/");
          break;
      }
    };

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [router]);
}
