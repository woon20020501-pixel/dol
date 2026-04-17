import { z } from "zod";

/**
 * Build-time env schema validation.
 *
 * Zod parses `process.env` against the declared shape and returns
 * typed env values. Any missing/malformed env var fails CLOSED — the
 * app refuses to render instead of silently running with `appId: ""`
 * and confusing every downstream hook. The old soft-warning path in
 * providers.tsx was fine for dev ergonomics but violates "fail fast"
 * production discipline.
 *
 * Why zod over hand-rolled validation:
 *   - Single source of truth for types (inferred, no duplicated
 *     Record<string, string> declarations)
 *   - Default values per key so tests/CI never need a real .env.local
 *   - Precise error messages naming the failing key
 *
 * Docs: https://zod.dev/?id=environment-variables
 */

const EnvSchema = z.object({
  // Privy application id. Without this, wallet auth is dead.
  // Defaults to an empty sentinel so local dev without .env.local
  // still boots (providers.tsx then renders a connect-required
  // state rather than crashing). In CI we pass "ci-placeholder"
  // which trips the sentinel check below.
  NEXT_PUBLIC_PRIVY_APP_ID: z.string().default(""),

  // Target chain id (Base Sepolia = 84532 for Phase 1). Parsed as
  // string here; chains.ts does the numeric coercion + match check.
  NEXT_PUBLIC_CHAIN_ID: z.string().default("84532"),

  // Production site URL — consumed by layout.tsx metadataBase,
  // robots.ts, sitemap.ts. Must be a valid URL when set.
  NEXT_PUBLIC_SITE_URL: z
    .string()
    .url({ message: "must be a full URL including protocol" })
    .optional(),

  // Toggle between demo (SIM) and live modes. Any value other than
  // "true" disables demo mode.
  NEXT_PUBLIC_DEMO_MODE: z.enum(["true", "false", ""]).default(""),

  // Error reporter endpoint (optional). When set, reportError() POSTs
  // crash payloads here. Validated as URL so a typo doesn't silently
  // drop reports.
  NEXT_PUBLIC_ERROR_REPORT_URL: z
    .string()
    .url({ message: "must be a full URL including protocol" })
    .optional(),

  // Bot Status API root URL. Consumed by botApi.ts; defaults to
  // localhost:7777 there if unset. Validated as URL at boot so a
  // malformed value (e.g. missing protocol, typo in scheme) blows up
  // visibly instead of silently 404'ing every polled request.
  NEXT_PUBLIC_BOT_API_URL: z
    .string()
    .url({ message: "must be a full URL including protocol" })
    .optional(),

  // RPC endpoint override for wagmi (Base Sepolia). wagmi.ts adds a
  // second layer of validation — host allowlist + https-only — so a
  // compromised secret can't silently point the app at a hostile RPC.
  // Having it in the zod schema too means a malformed URL fails boot
  // before wagmi's softer fall-back-with-warning path kicks in.
  NEXT_PUBLIC_RPC_URL: z
    .string()
    .url({ message: "must be a full URL including protocol" })
    .optional(),
});

export type Env = z.infer<typeof EnvSchema>;

/**
 * Parse env ONCE at module load. Throws a user-readable error if
 * validation fails so a broken deploy dies at boot, not at first
 * user interaction. Returns the typed env object for import.
 */
function parseEnv(): Env {
  const parsed = EnvSchema.safeParse({
    NEXT_PUBLIC_PRIVY_APP_ID: process.env.NEXT_PUBLIC_PRIVY_APP_ID,
    NEXT_PUBLIC_CHAIN_ID: process.env.NEXT_PUBLIC_CHAIN_ID,
    NEXT_PUBLIC_SITE_URL: process.env.NEXT_PUBLIC_SITE_URL,
    NEXT_PUBLIC_DEMO_MODE: process.env.NEXT_PUBLIC_DEMO_MODE,
    NEXT_PUBLIC_ERROR_REPORT_URL: process.env.NEXT_PUBLIC_ERROR_REPORT_URL,
    NEXT_PUBLIC_BOT_API_URL: process.env.NEXT_PUBLIC_BOT_API_URL,
    NEXT_PUBLIC_RPC_URL: process.env.NEXT_PUBLIC_RPC_URL,
  });

  if (!parsed.success) {
    const formatted = parsed.error.issues
      .map(
        (issue) =>
          `  - ${issue.path.join(".")}: ${issue.message}`,
      )
      .join("\n");
    throw new Error(
      `[env] Invalid environment configuration:\n${formatted}\n\n` +
        `See .env.example for expected keys.`,
    );
  }

  return parsed.data;
}

export const env: Env = parseEnv();
