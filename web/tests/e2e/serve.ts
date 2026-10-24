// Static server for the e2e suite: dist/ with the same CSP as `freespace
// serve`, so a style or script the CSP would block fails here too. /api/* is
// answered by the mock in each test (page.route), never by this server.
import { resolve } from "node:path";

// $GUARD_DIST serves another build: refactor-guard shoots base's and head's with one harness.
const root = process.env.GUARD_DIST ? `${resolve(process.env.GUARD_DIST)}/` : new URL("../../dist/", import.meta.url).pathname;
const CSP = "default-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

Bun.serve({
  hostname: "127.0.0.1",
  port: Number(process.env.PORT ?? 4173),
  async fetch(req) {
    const path = new URL(req.url).pathname;
    if (path.startsWith("/api/")) return new Response("e2e: /api is mocked per test", { status: 501 });
    const rel = path === "/" ? "index.html" : path.slice(1);
    const file = Bun.file(root + rel);
    if (rel.includes("..") || !(await file.exists())) return new Response("not found", { status: 404 });
    return new Response(file, {
      headers: { "content-security-policy": CSP, "x-content-type-options": "nosniff", "cache-control": "no-store" },
    });
  },
});
console.log("e2e static server ready");
