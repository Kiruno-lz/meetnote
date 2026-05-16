import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";

const root = new URL("..", import.meta.url).pathname;
const outdir = join(root, "dist");

await mkdir(outdir, { recursive: true });

await Bun.build({
  entrypoints: [join(root, "src/main.ts")],
  outdir,
  target: "browser",
  minify: false,
  sourcemap: "external"
});

const html = await readFile(join(root, "index.html"), "utf8");
await writeFile(join(outdir, "index.html"), html.replace("/src/main.ts", "./main.js"));

console.log(`[frontend] built ${outdir}`);
