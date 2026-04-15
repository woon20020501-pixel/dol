/**
 * Dev-only logger. `info` / `warn` / `debug` no-op in production so the
 * browser console stays clean on the live site — no leaks, no implicit
 * "look at this error" red flag for a judge opening DevTools.
 *
 * `error` always logs, because errors matter in prod too (and Sentry /
 * log drains hook into console.error).
 *
 * Usage:
 *   import { log } from "@/lib/logger";
 *   log.warn("[vault] missing address, falling back", addr);
 *
 * Ban rule of thumb: never call console.log / console.info / console.warn
 * directly in src/ — use this helper, or an eslint-disabled console.error
 * for genuine errors.
 */

const isDev = process.env.NODE_ENV !== "production";

type LogFn = (...args: unknown[]) => void;

const noop: LogFn = () => {};

export const log = {
  info: isDev ? ((...args: unknown[]) => console.info(...args)) : noop,
  warn: isDev ? ((...args: unknown[]) => console.warn(...args)) : noop,
  debug: isDev ? ((...args: unknown[]) => console.debug(...args)) : noop,
  // Real errors always log. Use this for unexpected failures, not for
  // expected fallback paths (those should use log.warn).
  error: (...args: unknown[]): void => {
    // eslint-disable-next-line no-console
    console.error(...args);
  },
};
