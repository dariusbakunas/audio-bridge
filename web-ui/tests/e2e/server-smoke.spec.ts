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
  await page.getByPlaceholder("My session").fill(name);
  await page.getByRole("button", { name: "Create", exact: true }).click();

  const sessionSelect = page.getByRole("combobox", { name: "Playback session" });
  await expect(sessionSelect).toHaveValue(/sess:/);
  await expect
    .poll(async () => (await sessionSelect.locator("option").allTextContents()).join("|"))
    .toContain(name);
});
