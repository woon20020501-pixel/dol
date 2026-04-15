import { NextResponse, type NextRequest } from "next/server";

/**
 * Edge geo-block middleware.
 *
 * Tier A whitelist — these are the only jurisdictions where Dol is
 * cleared to go live right now.
 * Everyone else gets bounced to /unavailable with a waitlist form.
 *
 * Behavior:
 *   - Vercel sets `req.geo.country` from its edge network. If we can't
 *     read it (local dev, preview without geo, self-hosted), we fall
 *     open so development and QA keep working. Production on Vercel
 *     will always populate it.
 *   - /unavailable is always reachable so blocked users land somewhere.
 *   - /legal/* is always reachable — people need to be able to read
 *     the terms even if they can't sign up.
 *   - Static assets and Next internals are excluded via the matcher.
 */
const TIER_A = new Set(["VN", "TR", "PH", "MX", "AR"]);

const ALWAYS_ALLOW = [
  "/unavailable",
  "/legal",
  "/api", // reserve — if we add API routes later, they handle their own auth
];

export function middleware(req: NextRequest) {
  const { pathname } = req.nextUrl;

  if (ALWAYS_ALLOW.some((p) => pathname === p || pathname.startsWith(`${p}/`))) {
    return NextResponse.next();
  }

  // `geo` is populated by Vercel's edge runtime. Local dev = undefined.
  // We fall open on undefined so `pnpm dev` isn't blocked.
  const country = req.geo?.country;
  if (!country) {
    return NextResponse.next();
  }

  if (TIER_A.has(country)) {
    return NextResponse.next();
  }

  const url = req.nextUrl.clone();
  url.pathname = "/unavailable";
  url.search = `?from=${encodeURIComponent(country)}`;
  return NextResponse.rewrite(url);
}

export const config = {
  matcher: [
    /*
     * Match every path except:
     *   - _next/static, _next/image (build output)
     *   - favicon and other root-level static files
     *   - public assets (anything with a file extension)
     */
    "/((?!_next/static|_next/image|favicon.ico|.*\\..*).*)",
  ],
};
