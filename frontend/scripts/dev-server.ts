const transpiler = new Bun.Transpiler({
  loader: "ts",
  target: "browser"
});

const server = Bun.serve({
  port: Number(process.env.MEETNOTE_FRONTEND_PORT ?? 1420),
  async fetch(request: Request) {
    const url = new URL(request.url);
    const path = url.pathname === "/" ? "/index.html" : url.pathname;

    // Serve node_modules for importmap-resolved bare specifiers
    if (path.startsWith("/node_modules/")) {
      const nmFile = Bun.file(new URL(`..${path}`, import.meta.url));
      if (await nmFile.exists()) {
        const ct = path.endsWith(".mjs") || path.endsWith(".js") || path.endsWith(".cjs")
          ? "application/javascript; charset=utf-8"
          : contentTypeFor(path);
        return new Response(nmFile, { headers: { "content-type": ct } });
      }
      return new Response("Not found", { status: 404 });
    }

    const resolvedPath = path.endsWith(".ts") ? path : `${path}.ts`;
    const sourceFile = Bun.file(new URL(`..${resolvedPath}`, import.meta.url));

    if (path.endsWith(".ts") || (await sourceFile.exists())) {
      const source = await sourceFile.text();
      return new Response(transpiler.transformSync(source), {
        headers: { "content-type": "application/javascript; charset=utf-8" }
      });
    }

    const file = Bun.file(new URL(`..${path}`, import.meta.url));
    if (await file.exists()) {
      return new Response(file, {
        headers: { "content-type": contentTypeFor(path) }
      });
    }

    return new Response("Not found", { status: 404 });
  }
});

console.log(`[frontend] serving http://localhost:${server.port}`);

function contentTypeFor(path: string): string {
  if (path.endsWith(".html")) return "text/html; charset=utf-8";
  if (path.endsWith(".js")) return "application/javascript; charset=utf-8";
  if (path.endsWith(".css")) return "text/css; charset=utf-8";
  if (path.endsWith(".json")) return "application/json; charset=utf-8";
  return "text/plain; charset=utf-8";
}
