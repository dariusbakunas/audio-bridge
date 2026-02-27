import { expect, test } from "@playwright/test";

test("shows offline gate and recovers after updating API base", async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem("audioHub.apiBase", "http://127.0.0.1:9");
  });

  await page.goto("/");

  await expect(page.getByRole("heading", { name: "Server offline" })).toBeVisible({
    timeout: 15000
  });
  await expect(page.getByLabel("API base URL")).toHaveValue("http://127.0.0.1:9");

  const apiBaseInput = page.getByLabel("API base URL");
  const saveButton = page.getByRole("button", { name: "Save" });
  await apiBaseInput.fill("http://127.0.0.1:18080");
  await expect(saveButton).toBeEnabled();
  await saveButton.click();

  try {
    await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible({ timeout: 10000 });
  } catch {
    await page.getByRole("button", { name: "Reconnect" }).click();
    await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible({ timeout: 10000 });
  }
});

test("invalid API base can be saved but remains offline", async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem("audioHub.apiBase", "http://127.0.0.1:9");
  });

  await page.goto("/");

  await expect(page.getByRole("heading", { name: "Server offline" })).toBeVisible({
    timeout: 15000
  });

  const apiBaseInput = page.getByLabel("API base URL");
  const saveButton = page.getByRole("button", { name: "Save" });
  await apiBaseInput.fill("http://");
  await expect(saveButton).toBeEnabled();
  await saveButton.click();
  await expect(page.getByText("Effective base: http://")).toBeVisible();

  await page.getByRole("button", { name: "Reconnect" }).click();
  await expect(page.getByRole("heading", { name: "Server offline" })).toBeVisible({
    timeout: 10000
  });
});
