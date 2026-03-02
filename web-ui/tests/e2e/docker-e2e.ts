/// <reference types="node" />

import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

export const WEB_UI_ROOT = path.resolve(__dirname, "../..");
export const REPO_ROOT = path.resolve(WEB_UI_ROOT, "..");
export const STATE_FILE = path.join(WEB_UI_ROOT, "test-results", ".e2e-docker-state.json");
export const COMPOSE_FILE = path.join(REPO_ROOT, "docker-compose.isolated.yml");
export const FIXTURE_MANIFEST = path.join(WEB_UI_ROOT, "tests", "fixtures", "album-fixtures.yml");
export const FIXTURE_GENERATOR = path.join(REPO_ROOT, "scripts", "gen-audio-fixtures-from-yaml.sh");
const DEFAULT_HUB_PORT = process.env.E2E_HUB_PORT ?? "18080";
export const E2E_API_BASE = process.env.E2E_API_BASE ?? `http://127.0.0.1:${DEFAULT_HUB_PORT}`;

export type DockerState = {
  composeProject: string;
  tempRoot: string;
  mediaDir: string;
  dataDir: string;
  hubPort: string;
};

function runOrThrow(args: string[], env: NodeJS.ProcessEnv): void {
  const result = spawnSync("docker", args, {
    cwd: REPO_ROOT,
    env,
    stdio: "inherit"
  });
  if (result.status !== 0) {
    throw new Error(`docker command failed: docker ${args.join(" ")}`);
  }
}

export function ensureDockerAvailable(): void {
  const dockerCheck = spawnSync("docker", ["--version"], { stdio: "ignore" });
  if (dockerCheck.status !== 0) {
    throw new Error("docker is required for E2E tests but was not found in PATH");
  }
  const composeCheck = spawnSync("docker", ["compose", "version"], { stdio: "ignore" });
  if (composeCheck.status !== 0) {
    throw new Error("docker compose is required for E2E tests but is not available");
  }
}

export async function waitForHubHealthy(apiBase: string, timeoutMs: number): Promise<void> {
  const start = Date.now();
  const healthUrl = `${apiBase.replace(/\/+$/, "")}/health`;
  let lastError = "unknown";

  while (Date.now() - start < timeoutMs) {
    try {
      const response = await fetch(healthUrl);
      if (response.ok) {
        return;
      }
      lastError = `HTTP ${response.status}`;
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error);
    }
    await new Promise((resolve) => setTimeout(resolve, 1000));
  }

  throw new Error(`timed out waiting for hub health at ${healthUrl}: ${lastError}`);
}

export async function createDockerState(): Promise<DockerState> {
  const now = Date.now();
  const random = Math.random().toString(36).slice(2, 8);
  const composeProject = `audiohub-e2e-${process.pid}-${random}`;
  const tempRoot = path.join("/tmp", `audio-hub-e2e-${now}-${random}`);
  const mediaDir = path.join(tempRoot, "media");
  const dataDir = path.join(tempRoot, "data");
  const hubPort = DEFAULT_HUB_PORT;

  await fs.mkdir(mediaDir, { recursive: true });
  await fs.mkdir(dataDir, { recursive: true });
  generateFixtures(mediaDir);

  return {
    composeProject,
    tempRoot,
    mediaDir,
    dataDir,
    hubPort
  };
}

function generateFixtures(outputDir: string): void {
  const result = spawnSync(FIXTURE_GENERATOR, ["--config", FIXTURE_MANIFEST, "--output-dir", outputDir], {
    cwd: REPO_ROOT,
    env: process.env,
    stdio: "inherit"
  });
  if (result.status !== 0) {
    throw new Error("failed to generate E2E fixtures from album-fixtures.yml");
  }
}

export async function saveState(state: DockerState): Promise<void> {
  await fs.mkdir(path.dirname(STATE_FILE), { recursive: true });
  await fs.writeFile(STATE_FILE, JSON.stringify(state), "utf8");
}

export async function loadState(): Promise<DockerState | null> {
  try {
    const raw = await fs.readFile(STATE_FILE, "utf8");
    return JSON.parse(raw) as DockerState;
  } catch {
    return null;
  }
}

export async function removeStateFile(): Promise<void> {
  await fs.rm(STATE_FILE, { force: true });
}

export function composeEnv(state: DockerState): NodeJS.ProcessEnv {
  const uid = typeof process.getuid === "function" ? String(process.getuid()) : "1000";
  const gid = typeof process.getgid === "function" ? String(process.getgid()) : "1000";

  return {
    ...process.env,
    COMPOSE_PROJECT_NAME: state.composeProject,
    AUDIO_HUB_MEDIA_DIR: state.mediaDir,
    AUDIO_HUB_DATA_DIR: state.dataDir,
    AUDIO_HUB_UID: process.env.AUDIO_HUB_UID ?? uid,
    AUDIO_HUB_GID: process.env.AUDIO_HUB_GID ?? gid,
    AUDIO_HUB_WEB_API_BASE: process.env.AUDIO_HUB_WEB_API_BASE ?? "",
    AUDIO_HUB_PORT: state.hubPort
  };
}

export function composeUp(state: DockerState): void {
  const env = composeEnv(state);
  const args = ["compose", "-f", COMPOSE_FILE, "up", "--build", "-d"];
  runOrThrow(args, env);
}

export function composeDown(state: DockerState): void {
  const env = composeEnv(state);
  runOrThrow(["compose", "-f", COMPOSE_FILE, "down", "-v", "--remove-orphans"], env);
}

export function composePrintDiagnostics(state: DockerState): void {
  const env = composeEnv(state);
  spawnSync("docker", ["compose", "-f", COMPOSE_FILE, "ps"], {
    cwd: REPO_ROOT,
    env,
    stdio: "inherit"
  });
  spawnSync("docker", ["compose", "-f", COMPOSE_FILE, "logs", "--no-color", "--tail=200"], {
    cwd: REPO_ROOT,
    env,
    stdio: "inherit"
  });
}

export async function removeTempRoot(state: DockerState): Promise<void> {
  await fs.rm(state.tempRoot, { recursive: true, force: true });
}
