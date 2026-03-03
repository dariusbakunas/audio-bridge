/// <reference types="node" />

import type { FullConfig } from "@playwright/test";
import { composeDown, loadState, removeStateFile, removeTempRoot } from "./docker-e2e";

async function globalTeardown(_config: FullConfig): Promise<void> {
  const loaded = await loadState();
  if (!loaded) {
    return;
  }
  const states = Array.isArray(loaded) ? loaded : [loaded];

  try {
    for (const state of states) {
      composeDown(state);
    }
  } finally {
    for (const state of states) {
      await removeTempRoot(state);
    }
    await removeStateFile();
  }
}

export default globalTeardown;
