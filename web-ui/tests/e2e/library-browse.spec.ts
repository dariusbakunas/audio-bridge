import { expect, Page, test } from "@playwright/test";

async function waitForAlbumsLoaded(page: Page): Promise<void> {
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(page.locator(".album-grid .album-card")).toHaveCount(4, { timeout: 30000 });
}

test("renders album grid from fixtures", async ({ page }) => {
  await page.goto("/");
  await waitForAlbumsLoaded(page);

  await expect(page.locator(".pill").filter({ hasText: "4 albums" })).toBeVisible();
  await expect(page.getByText("True North", { exact: true })).toBeVisible();
  await expect(page.getByText("Charlotte de Witte", { exact: true })).toBeVisible();
  await expect(page.getByText("Random Access Memories", { exact: false })).toBeVisible();
  await expect(page.getByText("Daydream Nation", { exact: false })).toBeVisible();
});

test("opens album view and shows track list", async ({ page }) => {
  await page.goto("/");
  await waitForAlbumsLoaded(page);

  await page.locator(".album-card-link").filter({ hasText: "True North" }).first().click();

  await expect(page.getByRole("heading", { name: /True North/i })).toBeVisible();
  await expect(page.locator(".album-meta-line").filter({ hasText: "A-Ha" })).toBeVisible();
  await expect(page.getByText("I'm In", { exact: true })).toBeVisible();
  await expect(page.getByText("12 tracks", { exact: true })).toBeVisible();
});

test("switches from grid to list and opens album from list row", async ({ page }) => {
  await page.goto("/");
  await waitForAlbumsLoaded(page);

  await page.locator('button[title="List view"]').click();
  await expect(page.locator(".album-list .album-list-row")).toHaveCount(4);

  const ramRow = page.locator(".album-list-row").filter({ hasText: "Random Access Memories" }).first();
  await ramRow.locator(".album-list-actions .btn.ghost.small").click();

  await expect(page.getByRole("heading", { name: /Random Access Memories/i })).toBeVisible();
});

test("search filters albums and shows empty state", async ({ page }) => {
  await page.goto("/");
  await waitForAlbumsLoaded(page);

  const search = page.getByRole("searchbox", { name: "Search albums" });
  await search.fill("sonic");
  await expect(page.locator(".album-grid .album-card")).toHaveCount(1);
  await expect(page.getByText("Daydream Nation", { exact: false })).toBeVisible();

  await search.fill("zzzz-no-match");
  await expect(page.locator(".album-grid .album-card")).toHaveCount(0);
  await expect(page.getByText("No albums found.", { exact: true })).toBeVisible();
});
