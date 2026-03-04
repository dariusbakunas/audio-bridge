/// <reference types="node" />

import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

export const WEB_UI_ROOT = path.resolve(__dirname, "../..");
export const REPO_ROOT = path.resolve(WEB_UI_ROOT, "..");
const stateKeyRaw = process.env.E2E_STATE_KEY ?? process.env.E2E_HUB_PORT ?? `${process.pid}`;
const stateKey = stateKeyRaw.replace(/[^a-zA-Z0-9_.-]/g, "_");
export const STATE_FILE = path.join(WEB_UI_ROOT, "test-results", `.e2e-docker-state.${stateKey}.json`);
export const COMPOSE_FILE = path.join(REPO_ROOT, "docker-compose.isolated.yml");
export const HUB_CONFIG_TEMPLATE = path.join(REPO_ROOT, "crates", "audio-hub-server", "config.docker-isolated.toml");
export const FIXTURE_MANIFEST = path.join(WEB_UI_ROOT, "tests", "fixtures", "album-fixtures.yml");
export const FIXTURE_GENERATOR = path.join(REPO_ROOT, "scripts", "gen-audio-fixtures-from-yaml.sh");
const DEFAULT_HUB_PORT = process.env.E2E_HUB_PORT ?? "18080";
export const E2E_API_BASE = process.env.E2E_API_BASE ?? `http://127.0.0.1:${DEFAULT_HUB_PORT}`;

export type DockerState = {
  composeProject: string;
  stateKey: string;
  tempRoot: string;
  mediaDir: string;
  dataDir: string;
  hubConfigPath: string;
  hubPort: string;
  subnetCidr: string;
  bridge1Ipv4: string;
  bridge2Ipv4: string;
};

export type PersistedDockerState = DockerState | DockerState[];
export type FixtureSeed = {
  tempRoot: string;
  mediaDir: string;
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

export function ensureWebUiDistBuilt(): void {
  const buildEnv: NodeJS.ProcessEnv = {
    ...process.env,
    VITE_API_BASE: "__EMPTY__"
  };
  const result = spawnSync("npm", ["run", "build"], {
    cwd: WEB_UI_ROOT,
    env: buildEnv,
    stdio: "inherit"
  });
  if (result.status !== 0) {
    throw new Error("failed to build web-ui/dist for E2E tests");
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

export async function createDockerState(options?: { hubPort?: string; stateKey?: string }): Promise<DockerState> {
  const state = await createDockerStateSkeleton(options);
  generateFixtures(state.mediaDir);
  return state;
}

export async function createDockerStateFromFixtureSeed(
  fixtureMediaDir: string,
  options?: { hubPort?: string; stateKey?: string }
): Promise<DockerState> {
  const state = await createDockerStateSkeleton(options);
  await copyFixtures(fixtureMediaDir, state.mediaDir);
  return state;
}

async function createDockerStateSkeleton(options?: { hubPort?: string; stateKey?: string }): Promise<DockerState> {
  const now = Date.now();
  const random = Math.random().toString(36).slice(2, 8);
  const key = options?.stateKey ?? stateKey;
  const composeProject = `audiohub-e2e-${process.pid}-${key}-${random}`;
  const tempRoot = path.join("/tmp", `audio-hub-e2e-${now}-${random}`);
  const mediaDir = path.join(tempRoot, "media");
  const dataDir = path.join(tempRoot, "data");
  const hubConfigPath = path.join(tempRoot, "config.docker-isolated.toml");
  const hubPort = options?.hubPort ?? DEFAULT_HUB_PORT;
  const { subnetCidr, bridge1Ipv4, bridge2Ipv4 } = computeNetworkPlan(hubPort);

  await fs.mkdir(mediaDir, { recursive: true });
  await fs.mkdir(dataDir, { recursive: true });
  await writeHubConfig(hubConfigPath, bridge1Ipv4, bridge2Ipv4);

  return {
    composeProject,
    stateKey: key,
    tempRoot,
    mediaDir,
    dataDir,
    hubConfigPath,
    hubPort,
    subnetCidr,
    bridge1Ipv4,
    bridge2Ipv4
  };
}

export async function createFixtureSeed(): Promise<FixtureSeed> {
  const seedRoot = await fs.mkdtemp(path.join("/tmp", "audio-hub-e2e-fixtures-"));
  const mediaDir = path.join(seedRoot, "media");
  await fs.mkdir(mediaDir, { recursive: true });
  generateFixtures(mediaDir);
  return { tempRoot: seedRoot, mediaDir };
}

function computeNetworkPlan(hubPort: string): { subnetCidr: string; bridge1Ipv4: string; bridge2Ipv4: string } {
  const parsedPort = Number.parseInt(hubPort, 10);
  const subnetIndex = Number.isFinite(parsedPort) ? parsedPort % 250 : Math.floor(Math.random() * 250);
  const subnetPrefix = `172.31.${subnetIndex}`;
  return {
    subnetCidr: `${subnetPrefix}.0/24`,
    bridge1Ipv4: `${subnetPrefix}.2`,
    bridge2Ipv4: `${subnetPrefix}.3`
  };
}

async function writeHubConfig(outputPath: string, bridge1Ipv4: string, bridge2Ipv4: string): Promise<void> {
  const template = await fs.readFile(HUB_CONFIG_TEMPLATE, "utf8");
  const patched = template
    .replace(/172\.30\.0\.2/g, bridge1Ipv4)
    .replace(/172\.30\.0\.3/g, bridge2Ipv4);
  await fs.writeFile(outputPath, patched, "utf8");
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

async function copyFixtures(sourceDir: string, targetDir: string): Promise<void> {
  await fs.cp(sourceDir, targetDir, { recursive: true });
}

export async function saveState(state: PersistedDockerState): Promise<void> {
  await fs.mkdir(path.dirname(STATE_FILE), { recursive: true });
  await fs.writeFile(STATE_FILE, JSON.stringify(state), "utf8");
}

export async function loadState(): Promise<PersistedDockerState | null> {
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
    AUDIO_HUB_CONFIG_FILE: state.hubConfigPath,
    AUDIO_HUB_MEDIA_DIR: state.mediaDir,
    AUDIO_HUB_DATA_DIR: state.dataDir,
    AUDIO_HUB_WEB_DIST_DIR: path.join(WEB_UI_ROOT, "dist"),
    AUDIO_HUB_UID: process.env.AUDIO_HUB_UID ?? uid,
    AUDIO_HUB_GID: process.env.AUDIO_HUB_GID ?? gid,
    AUDIO_HUB_WEB_API_BASE: "",
    AUDIO_HUB_PORT: state.hubPort,
    AUDIO_HUB_SUBNET: state.subnetCidr,
    AUDIO_BRIDGE1_IPV4: state.bridge1Ipv4,
    AUDIO_BRIDGE2_IPV4: state.bridge2Ipv4
  };
}

export function composeUp(state: DockerState): void {
  const env = composeEnv(state);
  const args = ["compose", "-f", COMPOSE_FILE, "up", "-d"];
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

export async function removeFixtureSeed(seed: FixtureSeed): Promise<void> {
  await fs.rm(seed.tempRoot, { recursive: true, force: true });
}
