/**
 * FAQ markdown parser.
 *
 * The source document at `src/content/faq.md` follows a deterministic
 * shape:
 *
 *   # FAQ
 *   **Version ...**
 *   > Format note for dashboard integration: ...   <- strip
 *   ---
 *   ## Category 1 — General
 *   ### Question A
 *   paragraph paragraph
 *   ### Question B
 *   ...
 *   ---
 *   ## Category 2 — ...
 *   ...
 *   ## Category metadata (for dashboard tab generation)   <- strip everything from here on
 *
 * Answers can span multiple paragraphs. Category dividers are `---`
 * horizontal rules, which we ignore since the `## Category` heading
 * is the actual boundary. The last thing in the body (after stripping
 * the metadata section) might still contain a trailing horizontal
 * rule + italics footer line — those fall off naturally because we
 * only look inside the `## Category` blocks.
 */

export type FaqQuestion = {
  id: string;
  question: string;
  answer: string; // raw markdown body, renderable by lib/markdown.tsx
};

export type FaqCategory = {
  id: string;
  label: string;
  questions: FaqQuestion[];
};

function slugify(text: string): string {
  return text
    .toLowerCase()
    .replace(/[^\w\s-]/g, "")
    .replace(/\s+/g, "-")
    .replace(/-+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 80);
}

// Stable category id derived from the "## Category N — Label" heading.
// Maps each heading label to the id set VP used in the source metadata
// block at the bottom of the file, so we can rely on stable URL anchors.
const LABEL_TO_ID: Record<string, string> = {
  General: "general",
  "Buying & Holding": "buying",
  "Cashing Out": "cashout",
  "Safety & Trust": "safety",
  "Where Dol Works": "where",
  "Fees & Taxes": "fees",
  Support: "support",
};

export function parseFaq(raw: string): FaqCategory[] {
  const text = raw.replace(/\r\n/g, "\n");

  // Drop the Agent-C format-note blockquote wherever it sits near the top.
  // Using [\s\S] instead of the `s` (dotall) flag for es2017 compat.
  const sansNote = text.replace(
    /^>\s*\*\*Format note for dashboard[\s\S]*?(?=\n\n|\n#)/m,
    "",
  );

  // Drop everything from the category metadata block onward. That block
  // is VP-facing authoring context, not user content.
  const metaIdx = sansNote.indexOf("## Category metadata");
  const body = metaIdx >= 0 ? sansNote.slice(0, metaIdx) : sansNote;

  // Collect every `## Category N — Label` match with its byte offset so
  // we can carve the body into per-category slices. Using a plain exec
  // loop rather than matchAll to stay target-compat without needing
  // downlevelIteration.
  type RawMatch = { index: number; length: number; label: string };
  const categoryMatches: RawMatch[] = [];
  const categoryRegex = /^## Category \d+\s*[—-]\s*(.+)$/gm;
  let cm: RegExpExecArray | null;
  while ((cm = categoryRegex.exec(body)) !== null) {
    categoryMatches.push({
      index: cm.index,
      length: cm[0].length,
      label: cm[1].trim(),
    });
  }

  const categories: FaqCategory[] = [];
  for (let i = 0; i < categoryMatches.length; i++) {
    const m = categoryMatches[i];
    const id = LABEL_TO_ID[m.label] ?? slugify(m.label);
    const start = m.index + m.length;
    const end =
      i + 1 < categoryMatches.length ? categoryMatches[i + 1].index : body.length;
    const block = body.slice(start, end);

    const questionMatches: { index: number; length: number; question: string }[] = [];
    const questionRegex = /^### (.+)$/gm;
    let qm: RegExpExecArray | null;
    while ((qm = questionRegex.exec(block)) !== null) {
      questionMatches.push({
        index: qm.index,
        length: qm[0].length,
        question: qm[1].trim(),
      });
    }

    const questions: FaqQuestion[] = [];
    for (let j = 0; j < questionMatches.length; j++) {
      const qMatch = questionMatches[j];
      const qStart = qMatch.index + qMatch.length;
      const qEnd =
        j + 1 < questionMatches.length
          ? questionMatches[j + 1].index
          : block.length;
      let answer = block.slice(qStart, qEnd);
      answer = answer.replace(/\n---\s*$/, "").trim();

      questions.push({
        id: slugify(qMatch.question),
        question: qMatch.question,
        answer,
      });
    }

    if (questions.length > 0) {
      categories.push({ id, label: m.label, questions });
    }
  }

  return categories;
}
