import { defineConfig, loadEnv } from "vite";
import react from "@vitejs/plugin-react";

const proxyPaths = [
  "/library",
  "/play",
  "/pause",
  "/stop",
  "/seek",
  "/queue",
  "/outputs",
  "/providers",
  "/stream",
  "/swagger-ui"
];

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "VITE_");
  const apiTarget = env.VITE_API_BASE || "http://localhost:8080";
  const proxy = proxyPaths.reduce<Record<string, { target: string; changeOrigin: boolean }>>(
    (acc, path) => {
      acc[path] = { target: apiTarget, changeOrigin: true };
      return acc;
    },
    {}
  );

  return {
    plugins: [react()],
    server: {
      port: 5173,
      proxy
    }
  };
});
