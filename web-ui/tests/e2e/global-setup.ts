/// <reference types="node" />

import type { FullConfig } from "@playwright/test";
import {
  E2E_API_BASE,
  composePrintDiagnostics,
  composeDown,
  composeUp,
  createDockerState,
  type DockerState,
  ensureDockerAvailable,
  ensureWebUiDistBuilt,
  removeStateFile,
  removeTempRoot,
  saveState,
  waitForHubHealthy
} from "./docker-e2e";

async function globalSetup(_config: FullConfig): Promise<void> {
  ensureDockerAvailable();
  ensureWebUiDistBuilt();
  const multiStack = process.env.E2E_MULTI_STACK === "1";
  let states: DockerState[];
  if (multiStack) {
    states = [
      await createDockerState({ stateKey: "chromium", hubPort: process.env.E2E_HUB_PORT_CHROMIUM ?? "18081" }),
      await createDockerState({ stateKey: "firefox", hubPort: process.env.E2E_HUB_PORT_FIREFOX ?? "18082" }),
      await createDockerState({ stateKey: "webkit", hubPort: process.env.E2E_HUB_PORT_WEBKIT ?? "18083" })
    ];
  } else {
    states = [await createDockerState()];
  }
  await saveState(states.length === 1 ? states[0] : states);
  try {
    for (const state of states) {
      composeUp(state);
    }
    if (multiStack) {
      await Promise.all(
        states.map((state) => waitForHubHealthy(`http://127.0.0.1:${state.hubPort}`, 180000))
      );
    } else {
      await waitForHubHealthy(E2E_API_BASE, 180000);
    }
  } catch (error) {
    for (const state of states) {
      composePrintDiagnostics(state);
      try {
        composeDown(state);
      } catch {
        // Keep original setup error.
      }
      await removeTempRoot(state);
    }
    await removeStateFile();
    throw error;
  }
}

export default globalSetup;
