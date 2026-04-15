import { Fragment, type ReactNode } from "react";
import Link from "next/link";

/**
 * Tiny, safe markdown renderer — no external deps.
 *
 * Why custom: react-markdown + remark would add ~50KB to the legal and
 * docs routes for what is effectively static long-form text. The drafts
 * use a small subset that we cover by hand:
 *
 *   Block:  h1/h2/h3/h4, paragraphs, ul, ol, blockquote, hr
 *   Inline: **bold**, *italic*, [text](url), `code`
 *
 * Links beginning with `/` render as Next.js `<Link>` for client-side
 * routing. External links open in a new tab with rel=noopener.
 *
 * Safety: we only emit text nodes and a fixed set of tag names. There
 * is no dangerouslySetInnerHTML anywhere. Any `<` or `>` in the source
 * becomes literal text via React's escaping. So even if a future draft
 * contains hostile content, it can't inject DOM.
 */

// ── Inline parser ─────────────────────────────────────────────────
//
// Single master regex finds the nearest inline token and we recurse on
// the remainder. Ordered so that `**` is tried before `*` to avoid
// swallowing bold markers as italic.
const INLINE_RE =
  /\*\*([^*\n]+?)\*\*|\*([^*\n]+?)\*|\[([^\]\n]+?)\]\(([^)\n]+?)\)|`([^`\n]+?)`/;

function renderInline(text: string, keyPrefix: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  let idx = 0;
  let rest = text;

  while (rest.length > 0) {
    const m = rest.match(INLINE_RE);
    if (!m || m.index === undefined) {
      nodes.push(
        <Fragment key={`${keyPrefix}-t-${idx++}`}>{rest}</Fragment>,
      );
      break;
    }

    if (m.index > 0) {
      nodes.push(
        <Fragment key={`${keyPrefix}-t-${idx++}`}>
          {rest.slice(0, m.index)}
        </Fragment>,
      );
    }

    const [full, bold, italic, linkText, linkHref, code] = m;
    const k = `${keyPrefix}-n-${idx++}`;

    if (bold !== undefined) {
      nodes.push(
        <strong key={k} className="font-semibold text-white">
          {renderInline(bold, k)}
        </strong>,
      );
    } else if (italic !== undefined) {
      nodes.push(
        <em key={k} className="italic">
          {renderInline(italic, k)}
        </em>,
      );
    } else if (linkText !== undefined && linkHref !== undefined) {
      // URL scheme allowlist — defense against a malicious or
      // mistyped markdown link such as `[click](javascript:...)`
      // ending up as a clickable XSS. Only relative paths, absolute
      // http(s), and mailto are accepted. Everything else (including
      // `javascript:`, `data:`, `vbscript:`, `file:`) falls back to
      // rendering the link text as plain, non-clickable content.
      //
      // React 18 still renders `javascript:` URLs with only a dev
      // warning; production builds strip the warning and the href
      // goes through. React 19 will auto-block them, but until then
      // the renderer has to enforce this itself.
      const href = linkHref.trim();
      const SAFE_SCHEME = /^(https?:|mailto:|\/)/i;
      if (!SAFE_SCHEME.test(href)) {
        // Unsafe scheme — surface the link text without a target so
        // the user sees the intent but can't execute the payload. A
        // dev-time console warning also helps whoever wrote the bad
        // markdown catch it before shipping.
        if (process.env.NODE_ENV !== "production") {
          // eslint-disable-next-line no-console
          console.warn(
            `[markdown] refused unsafe link scheme in "[${linkText}](${linkHref})"`,
          );
        }
        nodes.push(
          <span key={k} className="text-white/60">
            {renderInline(linkText, k)}
          </span>,
        );
      } else if (href.startsWith("/")) {
        nodes.push(
          <Link
            key={k}
            href={href}
            className="text-white underline decoration-white/30 underline-offset-4 hover:decoration-white"
          >
            {renderInline(linkText, k)}
          </Link>,
        );
      } else {
        nodes.push(
          <a
            key={k}
            href={href}
            target="_blank"
            rel="noopener noreferrer"
            className="text-white underline decoration-white/30 underline-offset-4 hover:decoration-white"
          >
            {renderInline(linkText, k)}
          </a>,
        );
      }
    } else if (code !== undefined) {
      nodes.push(
        <code
          key={k}
          className="rounded bg-white/10 px-1.5 py-0.5 font-mono text-[0.9em] text-white"
        >
          {code}
        </code>,
      );
    }

    rest = rest.slice(m.index + full.length);
  }

  return nodes;
}

// ── Block parser ──────────────────────────────────────────────────

type Block =
  | { kind: "h1"; text: string }
  | { kind: "h2"; text: string }
  | { kind: "h3"; text: string }
  | { kind: "h4"; text: string }
  | { kind: "p"; text: string }
  | { kind: "ul"; items: string[] }
  | { kind: "ol"; items: string[] }
  | { kind: "blockquote"; lines: string[] }
  | { kind: "hr" };

function parse(md: string): Block[] {
  const lines = md.replace(/\r\n/g, "\n").split("\n");
  const blocks: Block[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];

    if (line.trim() === "") {
      i++;
      continue;
    }

    // Horizontal rule — three or more dashes on a line by themselves
    if (/^-{3,}\s*$/.test(line)) {
      blocks.push({ kind: "hr" });
      i++;
      continue;
    }

    if (line.startsWith("#### ")) {
      blocks.push({ kind: "h4", text: line.slice(5).trim() });
      i++;
      continue;
    }
    if (line.startsWith("### ")) {
      blocks.push({ kind: "h3", text: line.slice(4).trim() });
      i++;
      continue;
    }
    if (line.startsWith("## ")) {
      blocks.push({ kind: "h2", text: line.slice(3).trim() });
      i++;
      continue;
    }
    if (line.startsWith("# ")) {
      blocks.push({ kind: "h1", text: line.slice(2).trim() });
      i++;
      continue;
    }

    // Blockquote — collect contiguous `> ` lines. Multi-line quotes
    // are joined into paragraphs inside the blockquote so the v0.2
    // legal "What changed from v0.1" callout renders correctly.
    if (line.startsWith(">")) {
      const qlines: string[] = [];
      while (i < lines.length && lines[i].startsWith(">")) {
        qlines.push(lines[i].replace(/^>\s?/, ""));
        i++;
      }
      blocks.push({ kind: "blockquote", lines: qlines });
      continue;
    }

    // Unordered list
    if (/^- /.test(line)) {
      const items: string[] = [];
      while (i < lines.length && /^- /.test(lines[i])) {
        items.push(lines[i].slice(2).trim());
        i++;
      }
      blocks.push({ kind: "ul", items });
      continue;
    }

    // Ordered list
    if (/^\d+\. /.test(line)) {
      const items: string[] = [];
      while (i < lines.length && /^\d+\. /.test(lines[i])) {
        items.push(lines[i].replace(/^\d+\. /, "").trim());
        i++;
      }
      blocks.push({ kind: "ol", items });
      continue;
    }

    // Paragraph — collect contiguous non-empty, non-special lines
    const paraLines: string[] = [line];
    i++;
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !lines[i].startsWith("#") &&
      !/^- /.test(lines[i]) &&
      !/^\d+\. /.test(lines[i]) &&
      !lines[i].startsWith(">") &&
      !/^-{3,}\s*$/.test(lines[i])
    ) {
      paraLines.push(lines[i]);
      i++;
    }
    blocks.push({ kind: "p", text: paraLines.join(" ") });
  }
  return blocks;
}

// ── Render ────────────────────────────────────────────────────────

export function renderMarkdown(md: string): ReactNode {
  const blocks = parse(md);
  return blocks.map((b, idx) => {
    const k = `block-${idx}`;
    switch (b.kind) {
      case "h1":
        return (
          <h1
            key={k}
            className="mt-12 text-4xl font-bold text-white"
            style={{ letterSpacing: "-0.03em" }}
          >
            {renderInline(b.text, k)}
          </h1>
        );
      case "h2":
        return (
          <h2
            key={k}
            className="mt-10 text-2xl font-semibold text-white"
            style={{ letterSpacing: "-0.02em" }}
          >
            {renderInline(b.text, k)}
          </h2>
        );
      case "h3":
        return (
          <h3 key={k} className="mt-6 text-lg font-semibold text-white">
            {renderInline(b.text, k)}
          </h3>
        );
      case "h4":
        return (
          <h4 key={k} className="mt-5 text-base font-semibold text-white/90">
            {renderInline(b.text, k)}
          </h4>
        );
      case "p":
        return (
          <p
            key={k}
            className="mt-4 text-[15px] leading-relaxed text-white/70"
          >
            {renderInline(b.text, k)}
          </p>
        );
      case "ul":
        return (
          <ul
            key={k}
            className="mt-4 list-disc space-y-2 pl-6 text-[15px] text-white/70"
          >
            {b.items.map((it, j) => (
              <li key={`${k}-li-${j}`} className="leading-relaxed">
                {renderInline(it, `${k}-${j}`)}
              </li>
            ))}
          </ul>
        );
      case "ol":
        return (
          <ol
            key={k}
            className="mt-4 list-decimal space-y-2 pl-6 text-[15px] text-white/70"
          >
            {b.items.map((it, j) => (
              <li key={`${k}-li-${j}`} className="leading-relaxed">
                {renderInline(it, `${k}-${j}`)}
              </li>
            ))}
          </ol>
        );
      case "blockquote":
        return (
          <blockquote
            key={k}
            className="mt-6 rounded-r-lg border-l-2 border-white/30 bg-white/[0.03] py-3 pl-5 pr-4 text-[15px] italic text-white/70"
          >
            {b.lines.map((ln, j) =>
              ln.trim() === "" ? (
                <div key={`${k}-br-${j}`} className="h-2" />
              ) : (
                <p key={`${k}-p-${j}`} className="leading-relaxed">
                  {renderInline(ln, `${k}-${j}`)}
                </p>
              ),
            )}
          </blockquote>
        );
      case "hr":
        return <hr key={k} className="my-10 border-white/10" />;
    }
  });
}
