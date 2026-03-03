import { expect, Page, test } from "@playwright/test";

async function waitForAlbumsLoaded(page: Page): Promise<void> {
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(page.locator(".album-grid .album-card")).toHaveCount(4, { timeout: 30000 });
}

async function switchToLocalSession(page: Page): Promise<void> {
  const sessionSelect = page.getByRole("combobox", { name: "Playback session" });
  await expect(sessionSelect).toBeEnabled();
  await sessionSelect.selectOption({ label: "Local" });
  await expect(sessionSelect).toHaveValue(/sess:/);
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

test("album view format column matches fixture audio formats", async ({ page }) => {
  await page.goto("/");
  await waitForAlbumsLoaded(page);

  const cases = [
    { album: "True North", expected: "FLAC 96kHz/24bit" },
    { album: "Random Access Memories", expected: "FLAC 88.2kHz/24bit" },
    { album: "Charlotte de Witte", expected: "FLAC 44.1kHz/24bit" },
    { album: "Daydream Nation", expected: "FLAC 44.1kHz/16bit" }
  ];

  for (const item of cases) {
    await page.locator(".album-card-link").filter({ hasText: item.album }).first().click();
    await expect(page.getByRole("heading", { name: new RegExp(item.album, "i") })).toBeVisible();
    await expect(page.locator(".album-track-row .album-track-format").first()).toContainText(item.expected);
    await page.getByRole("button", { name: "Back", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  }
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

test("supports back/forward navigation between albums and album detail", async ({ page }) => {
  await page.goto("/");
  await waitForAlbumsLoaded(page);

  await page.locator(".album-card-link").filter({ hasText: "Daydream Nation" }).first().click();
  await expect(page.getByRole("heading", { name: /Daydream Nation/i })).toBeVisible();

  await page.getByRole("button", { name: "Back", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(page.locator(".album-grid .album-card")).toHaveCount(4);

  await page.getByRole("button", { name: "Forward" }).click();
  await expect(page.getByRole("heading", { name: /Daydream Nation/i })).toBeVisible();
});

test("search is case-insensitive and supports artist/year terms", async ({ page }) => {
  await page.goto("/");
  await waitForAlbumsLoaded(page);

  const search = page.getByRole("searchbox", { name: "Search albums" });

  await search.fill("DAFT PUNK");
  await expect(page.locator(".album-grid .album-card")).toHaveCount(1);
  await expect(page.getByText("Random Access Memories", { exact: false })).toBeVisible();

  await search.fill("1988");
  await expect(page.locator(".album-grid .album-card")).toHaveCount(1);
  await expect(page.getByText("Daydream Nation", { exact: false })).toBeVisible();

  await search.fill("");
  await expect(page.locator(".album-grid .album-card")).toHaveCount(4);
});

test("local session can play an album from grid and populate queue", async ({ page }) => {
  await page.goto("/");
  await waitForAlbumsLoaded(page);
  await switchToLocalSession(page);

  await page
    .locator(".album-card")
    .filter({ hasText: "True North" })
    .first()
    .locator('.album-play[aria-label="Play True North"]')
    .click();

  await expect(page.locator(".player-left .track-title")).toContainText("I'm In");

  await page.getByRole("button", { name: "Queue" }).click();
  await expect(page.getByRole("complementary", { name: "Queue" })).toBeVisible();
  await expect(page.locator(".queue-list .queue-row")).toHaveCount(12);
  await expect(page.locator(".queue-list .queue-row").first()).toContainText("I'm In");
});

test("local session can jump playback by playing a specific queue item", async ({ page }) => {
  await page.goto("/");
  await waitForAlbumsLoaded(page);
  await switchToLocalSession(page);

  await page.locator(".album-card-link").filter({ hasText: "True North" }).first().click();
  await expect(page.getByRole("heading", { name: /True North/i })).toBeVisible();
  await page.getByRole("button", { name: "Play album" }).click();

  await page.getByRole("button", { name: "Queue" }).click();
  const queuePanel = page.getByRole("complementary", { name: "Queue" });
  await expect(queuePanel).toBeVisible();
  await queuePanel.getByRole("button", { name: "Play As If" }).click();

  await expect(page.locator(".player-left .track-title")).toContainText("As If");
});

test("preserves search query and list mode when navigating album detail and back", async ({ page }) => {
  await page.goto("/");
  await waitForAlbumsLoaded(page);

  const search = page.getByRole("searchbox", { name: "Search albums" });
  await search.fill("daft");
  await page.locator('button[title="List view"]').click();
  await expect(page.locator(".album-list .album-list-row")).toHaveCount(1);
  await expect(page.getByText("Random Access Memories", { exact: false })).toBeVisible();

  await page.locator(".album-list-row").first().locator(".btn.ghost.small").click();
  await expect(page.getByRole("heading", { name: /Random Access Memories/i })).toBeVisible();

  await page.getByRole("button", { name: "Back", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(search).toHaveValue("daft");
  await expect(page.locator('button[title="List view"]')).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(".album-list .album-list-row")).toHaveCount(1);
});

test("local session toggles play button state between pause and resume", async ({ page }) => {
  await page.goto("/");
  await waitForAlbumsLoaded(page);
  await switchToLocalSession(page);

  const trueNorthPlay = page
    .locator(".album-card")
    .filter({ hasText: "True North" })
    .first()
    .locator('.album-play[aria-label="Play True North"]');

  await trueNorthPlay.click();
  await expect(page.locator(".player-left .track-title")).toContainText("I'm In");
  await expect(
    page
      .locator(".album-card")
      .filter({ hasText: "True North" })
      .first()
      .locator(".album-play")
  ).toHaveAttribute("aria-label", "Pause playback");

  await page
    .locator(".album-card")
    .filter({ hasText: "True North" })
    .first()
    .locator(".album-play")
    .click();
  await expect(
    page
      .locator(".album-card")
      .filter({ hasText: "True North" })
      .first()
      .locator(".album-play")
  ).toHaveAttribute("aria-label", "Resume playback");

  await page
    .locator(".album-card")
    .filter({ hasText: "True North" })
    .first()
    .locator(".album-play")
    .click();
  await expect(
    page
      .locator(".album-card")
      .filter({ hasText: "True North" })
      .first()
      .locator(".album-play")
  ).toHaveAttribute("aria-label", "Pause playback");
});

test("queue clear confirmation can be opened and canceled without clearing items", async ({ page }) => {
  await page.goto("/");
  await waitForAlbumsLoaded(page);
  await switchToLocalSession(page);

  await page
    .locator(".album-card")
    .filter({ hasText: "True North" })
    .first()
    .locator('.album-play[aria-label="Play True North"]')
    .click();

  await page.getByRole("button", { name: "Queue" }).click();
  const queuePanel = page.getByRole("complementary", { name: "Queue" });
  await expect(queuePanel).toBeVisible();
  await expect(queuePanel.locator(".queue-list .queue-row")).toHaveCount(12);

  await queuePanel.getByRole("button", { name: "Clear queue" }).click();
  const clearModal = page.locator(".modal");
  await expect(clearModal.getByText("Clear queue?", { exact: true })).toBeVisible();
  await expect(clearModal.getByLabel("Clear queue")).toBeChecked();
  await expect(clearModal.getByLabel("Clear history")).not.toBeChecked();

  await clearModal.getByRole("button", { name: "Cancel" }).click();
  await expect(clearModal).toBeHidden();
  await expect(queuePanel.locator(".queue-list .queue-row")).toHaveCount(12);
});
