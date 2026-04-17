import fs from "node:fs";
import path from "node:path";
import Link from "next/link";
import { notFound } from "next/navigation";
import { DocsSidebar } from "@/components/DocsSidebar";
import { SiteFooter } from "@/components/SiteFooter";
import { renderMarkdown } from "@/lib/markdown";

/**
 * Catch-all docs route.
 *
 *   /docs                              → src/content/docs/index.md
 *   /docs/faq                          → src/content/docs/faq.md
 *   /docs/getting-started/what-is-dol  → src/content/docs/getting-started/what-is-dol.md
 *
 * Slug segments map 1:1 to directories. If the slug doesn't resolve to
 * a real markdown file, we surface Next.js's built-in 404 rather than
 * a custom page —.
 *
 * The `/docs` landing page renders without the sidebar (marketing-style
 * intro), every sub-page renders with it (reference layout). This
 * mirrors Liminal's docs site and gives the landing room to breathe.
 *
 * Security: slug is clamped to segments made of letters, digits, dashes,
 * and underscores. That prevents `..` or absolute path escapes from
 * being fed into fs.readFileSync. Any other character means 404.
 */

const DOCS_ROOT = path.join(process.cwd(), "src/content/docs");
const SAFE_SEGMENT = /^[a-z0-9_-]+$/i;

// Statically generate every route at build time. Next.js streams each
// of these through the same `DocsPage` below, which means the fs reads
// happen once during build rather than on every request.
export async function generateStaticParams() {
  const params: { slug: string[] }[] = [];

  const walk = (dir: string, prefix: string[]) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      if (entry.isDirectory()) {
        walk(path.join(dir, entry.name), [...prefix, entry.name]);
      } else if (entry.isFile() && entry.name.endsWith(".md")) {
        if (entry.name === "index.md") {
          params.push({ slug: prefix });
        } else {
          params.push({ slug: [...prefix, entry.name.replace(/\.md$/, "")] });
        }
      }
    }
  };

  walk(DOCS_ROOT, []);
  return params;
}

type Props = { params: { slug?: string[] } };

function resolveMarkdownPath(slug: string[]): string | null {
  if (slug.some((s) => !SAFE_SEGMENT.test(s))) return null;

  if (slug.length === 0) {
    const p = path.join(DOCS_ROOT, "index.md");
    return fs.existsSync(p) ? p : null;
  }

  const leafPath = path.join(DOCS_ROOT, ...slug) + ".md";
  if (fs.existsSync(leafPath)) return leafPath;

  const indexPath = path.join(DOCS_ROOT, ...slug, "index.md");
  if (fs.existsSync(indexPath)) return indexPath;

  return null;
}

export function generateMetadata({ params }: Props) {
  const slug = params.slug ?? [];
  const title =
    slug.length === 0
      ? "Documentation"
      : slug[slug.length - 1]
          .replace(/-/g, " ")
          .replace(/\b\w/g, (c) => c.toUpperCase());
  return {
    title: `${title} · Dol Docs`,
    description: "Documentation for Dol — a dollar that grows itself.",
  };
}

export default function DocsPage({ params }: Props) {
  const slug = params.slug ?? [];
  const mdPath = resolveMarkdownPath(slug);
  if (!mdPath) notFound();

  const md = fs.readFileSync(mdPath, "utf8");
  const isLanding = slug.length === 0;

  const rendered = renderMarkdown(md);

  if (isLanding) {
    // Landing: no sidebar, marketing-style centered column.
    return (
      <main className="min-h-screen bg-black text-white">
        <DocsHeader />
        <article className="mx-auto max-w-2xl px-6 pb-24 pt-8">
          {rendered}
        </article>
        <SiteFooter />
      </main>
    );
  }

  // Sub-page: two-column layout with the fixed sidebar.
  return (
    <main className="min-h-screen bg-black text-white">
      <DocsHeader />
      <div className="flex">
        <DocsSidebar />
        <article className="mx-auto max-w-3xl flex-1 px-6 pb-24 pt-8 md:px-10">
          {rendered}
        </article>
      </div>
      <SiteFooter />
    </main>
  );
}

function DocsHeader() {
  return (
    <header className="sticky top-0 z-20 border-b border-white/5 bg-black/80 backdrop-blur">
      <div className="mx-auto flex max-w-6xl items-center justify-between px-6 py-4">
        <Link
          href="/"
          className="text-sm font-semibold text-white/80 hover:text-white"
        >
          Dol
        </Link>
        <span className="text-xs uppercase tracking-[0.14em] text-white/30">
          Documentation
        </span>
      </div>
    </header>
  );
}
