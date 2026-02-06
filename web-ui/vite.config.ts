import { defineConfig, loadEnv } from "vite";
import react from "@vitejs/plugin-react";
import { execSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const proxyPaths = [
  "/library",
  "/play",
  "/pause",
  "/stop",
  "/seek",
  "/queue",
  "/artists",
  "/albums",
  "/tracks",
  "/outputs",
  "/providers",
  "/stream",
  "/swagger-ui"
];

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "VITE_");
  const apiTarget = env.VITE_API_BASE || "http://localhost:8080";
  const rootDir = dirname(fileURLToPath(import.meta.url));
  const pkg = JSON.parse(readFileSync(join(rootDir, "package.json"), "utf-8")) as {
    version?: string;
  };
  let gitSha = process.env.GIT_SHA || "unknown";
  if (gitSha === "unknown") {
    try {
      gitSha = execSync("git rev-parse --short HEAD", {
        cwd: rootDir,
        stdio: ["ignore", "pipe", "ignore"]
      })
        .toString()
        .trim();
    } catch {
      gitSha = "unknown";
    }
  }
  const appVersion = pkg.version ?? "0.0.0";
  const proxy = proxyPaths.reduce<Record<string, { target: string; changeOrigin: boolean }>>(
    (acc, path) => {
      acc[path] = { target: apiTarget, changeOrigin: true };
      return acc;
    },
    {}
  );

  return {
    plugins: [react()],
    define: {
      __APP_VERSION__: JSON.stringify(appVersion),
      __GIT_SHA__: JSON.stringify(gitSha),
      __BUILD_MODE__: JSON.stringify(mode)
    },
    server: {
      port: 5173,
      proxy
    }
  };
});
