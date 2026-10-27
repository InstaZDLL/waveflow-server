// Which of the two navigations the viewport draws, asked once it has drawn one.

import type { Page } from "@playwright/test";

/**
 * Whether the page shows the phone's bottom navigation rather than the
 * sidebar.
 *
 * `isVisible()` answers at once, without waiting. Asked while the shell was
 * still mounting, it saw neither navigation, and a test on a phone then looked
 * for its links in the sidebar a phone never draws, until it timed out — on a
 * loaded machine, now and then. This waits for whichever one the viewport
 * draws, then asks.
 */
export async function onPhoneNavigation(page: Page): Promise<boolean> {
  await page
    .locator(".mobile-navigation:visible, .primary-navigation:visible")
    .first()
    .waitFor();
  return page.locator(".mobile-navigation").isVisible();
}
