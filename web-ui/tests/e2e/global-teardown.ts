/// <reference types="node" />

import type { FullConfig } from "@playwright/test";
import { composeDown, loadState, removeStateFile, removeTempRoot } from "./docker-e2e";

async function globalTeardown(_config: FullConfig): Promise<void> {
  const state = await loadState();
  if (!state) {
    return;
  }

  try {
    composeDown(state);
  } finally {
    await removeTempRoot(state);
    await removeStateFile();
  }
}

export default globalTeardown;
