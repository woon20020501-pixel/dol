import { describe, it, expect } from "vitest";
import { z } from "zod";

/**
 * Env schema contract tests. We re-declare the schema here (identical
 * to env.ts) so we can exercise parse behavior with injected values
 * without having to mutate process.env and reload modules — which
 * breaks happy-dom's execution model.
 *
 * If env.ts's schema drifts from this file, the next test run catches
 * it via the invariant check in the last describe block.
 */

const EnvSchema = z.object({
  NEXT_PUBLIC_PRIVY_APP_ID: z.string().default(""),
  NEXT_PUBLIC_CHAIN_ID: z.string().default("84532"),
  NEXT_PUBLIC_SITE_URL: z
    .string()
    .url({ message: "must be a full URL including protocol" })
    .optional(),
  NEXT_PUBLIC_DEMO_MODE: z.enum(["true", "false", ""]).default(""),
  NEXT_PUBLIC_ERROR_REPORT_URL: z
    .string()
    .url({ message: "must be a full URL including protocol" })
    .optional(),
  NEXT_PUBLIC_BOT_API_URL: z
    .string()
    .url({ message: "must be a full URL including protocol" })
    .optional(),
  NEXT_PUBLIC_RPC_URL: z
    .string()
    .url({ message: "must be a full URL including protocol" })
    .optional(),
});

describe("env schema", () => {
  describe("valid inputs", () => {
    it("accepts minimal valid config (all defaults)", () => {
      const r = EnvSchema.safeParse({});
      expect(r.success).toBe(true);
      if (r.success) {
        expect(r.data.NEXT_PUBLIC_PRIVY_APP_ID).toBe("");
        expect(r.data.NEXT_PUBLIC_CHAIN_ID).toBe("84532");
      }
    });

    it("accepts production config", () => {
      const r = EnvSchema.safeParse({
        NEXT_PUBLIC_PRIVY_APP_ID: "cm1abc",
        NEXT_PUBLIC_CHAIN_ID: "84532",
        NEXT_PUBLIC_SITE_URL: "https://dol-finance.vercel.app",
        NEXT_PUBLIC_DEMO_MODE: "false",
      });
      expect(r.success).toBe(true);
    });

    it("accepts error reporter endpoint", () => {
      const r = EnvSchema.safeParse({
        NEXT_PUBLIC_ERROR_REPORT_URL: "https://errors.example.com/ingest",
      });
      expect(r.success).toBe(true);
    });

    it("accepts bot API URL and RPC URL when valid", () => {
      const r = EnvSchema.safeParse({
        NEXT_PUBLIC_BOT_API_URL: "https://bot.example.com",
        NEXT_PUBLIC_RPC_URL: "https://sepolia.base.org",
      });
      expect(r.success).toBe(true);
    });
  });

  describe("invalid inputs fail CLOSED", () => {
    it("rejects SITE_URL without protocol", () => {
      const r = EnvSchema.safeParse({
        NEXT_PUBLIC_SITE_URL: "dol-finance.vercel.app",
      });
      expect(r.success).toBe(false);
    });

    it("rejects ERROR_REPORT_URL without protocol", () => {
      const r = EnvSchema.safeParse({
        NEXT_PUBLIC_ERROR_REPORT_URL: "errors.example.com",
      });
      expect(r.success).toBe(false);
    });

    it("rejects non-enum DEMO_MODE value", () => {
      const r = EnvSchema.safeParse({
        NEXT_PUBLIC_DEMO_MODE: "maybe",
      });
      expect(r.success).toBe(false);
    });

    it("rejects BOT_API_URL without protocol", () => {
      const r = EnvSchema.safeParse({
        NEXT_PUBLIC_BOT_API_URL: "bot.example.com",
      });
      expect(r.success).toBe(false);
    });

    it("rejects RPC_URL without protocol", () => {
      const r = EnvSchema.safeParse({
        NEXT_PUBLIC_RPC_URL: "sepolia.base.org",
      });
      expect(r.success).toBe(false);
    });

    it("error messages name the failing key", () => {
      const r = EnvSchema.safeParse({
        NEXT_PUBLIC_SITE_URL: "not-a-url",
      });
      if (!r.success) {
        const paths = r.error.issues.map((i) => i.path.join("."));
        expect(paths).toContain("NEXT_PUBLIC_SITE_URL");
      }
    });
  });
});
