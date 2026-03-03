import { expect, test } from "@playwright/test";

test.describe.configure({ mode: "serial" });

test("app connects to real server and renders main albums view", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Server offline" })).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Connecting" })).toHaveCount(0);

  const sessionSelect = page.getByRole("combobox", { name: "Playback session" });
  await expect(sessionSelect).toBeEnabled();
  await expect
    .poll(async () => (await sessionSelect.locator("option").allTextContents()).join("|"))
    .toContain("Default");
});

test("creates a new remote playback session through UI", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  const initialSessionSelect = page.getByRole("combobox", { name: "Playback session" });
  await expect
    .poll(async () => initialSessionSelect.isEnabled(), {
      timeout: 30000
    })
    .toBe(true);
  await expect
    .poll(async () => (await initialSessionSelect.locator("option").allTextContents()).join("|"), {
      timeout: 15000
    })
    .toContain("Default");

  const createSessionButton = page.getByRole("button", { name: "Create new session" });
  await expect(createSessionButton).toBeEnabled();
  await createSessionButton.click();
  await expect(page.locator(".modal .card-header span").filter({ hasText: "Create session" })).toBeVisible();

  const name = `E2E Session ${Date.now()}`;
  await page.getByLabel("Name").fill(name);
  await page.getByRole("button", { name: "Create", exact: true }).click();

  const sessionSelect = page.getByRole("combobox", { name: "Playback session" });
  await expect(sessionSelect).toHaveValue(/sess:/);
  await expect
    .poll(async () => (await sessionSelect.locator("option").allTextContents()).join("|"))
    .toContain(name);
});

test("shows dummy bridge outputs and allows selecting one", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();

  const outputButton = page.locator(".player-action-output");
  await expect(outputButton).toBeVisible();
  await outputButton.click();

  const outputsModal = page.locator(".modal");
  await expect(
    outputsModal.locator(".card-header span").filter({ hasText: "Outputs" })
  ).toBeVisible();
  await expect(
    outputsModal.locator(".output-title", { hasText: "Dummy Output Fixed 48k" })
  ).toHaveCount(2);
  await expect(
    outputsModal.locator(".output-title", { hasText: "Dummy Output 44.1k/96k (exclusive)" })
  ).toHaveCount(2);

  const targetRow = outputsModal
    .locator(".output-row")
    .filter({ hasText: "Dummy Output Fixed 48k" })
    .first();
  const selectedName = ((await targetRow.locator(".output-title").textContent()) ?? "").trim();
  await targetRow.click();

  await expect(outputsModal.locator(".output-row.active .output-title")).toContainText(
    "Dummy Output Fixed 48k"
  );
  if (selectedName.length > 0) {
    await expect(page.locator(".player-action-output .player-action-label")).toHaveText(
      selectedName
    );
  } else {
    await expect(page.locator(".player-action-output .player-action-label")).toContainText(
      "Dummy Output Fixed 48k"
    );
  }
});

test("switching outputs during active playback preserves track and play/pause state", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  const initialSessionSelect = page.getByRole("combobox", { name: "Playback session" });
  await expect
    .poll(async () => initialSessionSelect.isEnabled(), {
      timeout: 30000
    })
    .toBe(true);
  await expect
    .poll(async () => (await initialSessionSelect.locator("option").allTextContents()).join("|"), {
      timeout: 15000
    })
    .toContain("Default");

  const outputButton = page.locator(".player-action-output");
  await outputButton.click();
  const outputsModal = page.locator(".modal");
  await expect(outputsModal.locator(".card-header span").filter({ hasText: "Outputs" })).toBeVisible();

  const unlockedRows = outputsModal.locator(".output-row:not(.locked)").filter({ hasText: "Dummy Output" });
  const unlockedCount = await unlockedRows.count();
  expect(unlockedCount).toBeGreaterThanOrEqual(2);

  const unlockedNames = await unlockedRows.locator(".output-title").allTextContents();
  const firstOutputName = (unlockedNames[0] ?? "").trim();
  const secondOutputName = (unlockedNames[1] ?? "").trim();
  expect(firstOutputName.length).toBeGreaterThan(0);
  expect(secondOutputName.length).toBeGreaterThan(0);

  const firstOutput = unlockedRows.nth(0);
  await firstOutput.click();
  await expect(page.locator(".player-action-output .player-action-label")).toHaveText(firstOutputName);
  await outputsModal.getByRole("button", { name: "Close" }).click();
  await expect(outputsModal).toBeHidden();

  await page.locator(".album-card").filter({ hasText: "True North" }).first().locator(".album-play").click();
  await expect(page.locator(".player-left .track-title")).toContainText("I'm In");
  await expect(page.locator(".player-controls .primary .lucide-pause")).toBeVisible();
  await page.waitForTimeout(3000);

  await outputButton.click();
  await expect(outputsModal.locator(".card-header span").filter({ hasText: "Outputs" })).toBeVisible();
  await outputsModal.locator(".output-row").filter({ hasText: secondOutputName }).first().click();
  await outputsModal.getByRole("button", { name: "Close" }).click();
  await expect(outputsModal).toBeHidden();
  if (secondOutputName.length > 0) {
    await expect(page.locator(".player-action-output .player-action-label")).toHaveText(secondOutputName);
  }
  await expect(page.locator(".player-left .track-title")).toContainText("I'm In");
  await expect(page.locator(".player-controls .primary .lucide-pause")).toBeVisible();

  await page.getByRole("button", { name: "Play or pause" }).click();
  await expect(page.locator(".player-controls .primary .lucide-play")).toBeVisible();

  await outputButton.click();
  await expect(outputsModal.locator(".card-header span").filter({ hasText: "Outputs" })).toBeVisible();
  await outputsModal.locator(".output-row").filter({ hasText: firstOutputName }).first().click();
  await outputsModal.getByRole("button", { name: "Close" }).click();
  await expect(outputsModal).toBeHidden();
  if (firstOutputName.length > 0) {
    await expect(page.locator(".player-action-output .player-action-label")).toHaveText(firstOutputName);
  }
  await expect(page.locator(".player-left .track-title")).toContainText("I'm In");
  await expect(page.locator(".player-controls .primary .lucide-play")).toBeVisible();
});
