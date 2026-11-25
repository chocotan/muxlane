import * as esbuild from "esbuild";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(__dirname, "..");

const common = {
  bundle: true,
  platform: "node",
  format: "cjs",
  target: "node22",
  external: ["electron"],
  sourcemap: true,
  logLevel: "info",
};

await Promise.all([
  esbuild.build({
    ...common,
    entryPoints: [path.join(root, "electron/main.ts")],
    outfile: path.join(root, "out/main.cjs"),
  }),
  esbuild.build({
    ...common,
    entryPoints: [path.join(root, "electron/preload.ts")],
    outfile: path.join(root, "out/preload.cjs"),
  }),
]);
