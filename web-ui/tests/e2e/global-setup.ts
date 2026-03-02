/// <reference types="node" />

import type { FullConfig } from "@playwright/test";
import {
  E2E_API_BASE,
  composePrintDiagnostics,
  composeDown,
  composeUp,
  createDockerState,
  ensureDockerAvailable,
  removeStateFile,
  removeTempRoot,
  saveState,
  waitForHubHealthy
} from "./docker-e2e";

async function globalSetup(_config: FullConfig): Promise<void> {
  ensureDockerAvailable();
  const state = await createDockerState();
  await saveState(state);
  try {
    composeUp(state);
    await waitForHubHealthy(E2E_API_BASE, 180000);
  } catch (error) {
    composePrintDiagnostics(state);
    try {
      composeDown(state);
    } catch {
      // Keep original setup error.
    }
    await removeTempRoot(state);
    await removeStateFile();
    throw error;
  }
}

export default globalSetup;
