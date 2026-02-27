import { expect, test } from "@playwright/test";

// @ts-ignore
test("connection gate allows saving API base", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByRole("heading", { name: /Connecting|Server offline/ })).toBeVisible();

  const apiBaseInput = page.getByLabel("API base URL");
  const saveButton = page.getByRole("button", { name: "Save" });

  await expect(saveButton).toBeDisabled();
  await apiBaseInput.fill("http://127.0.0.1:8080");
  await expect(saveButton).toBeEnabled();

  await saveButton.click();
  await expect(page.getByText("Effective base: http://127.0.0.1:8080")).toBeVisible();
});
