import { expect, test } from "@playwright/test";

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

  await page.getByRole("button", { name: "Create new session" }).click();
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
