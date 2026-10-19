// App menu (UI 1.4) and generic context menu helpers.

import { pressEscape } from './app.ts';
import { count, el, tid, waitFor, waitForAbsent } from './selectors.ts';

/** Opens the app menu with the title bar button. */
export async function openAppMenu(): Promise<void> {
  if ((await count('app-menu')) > 0) {
    return;
  }
  await (await waitFor('titlebar-menu-button')).click();
  await waitFor('app-menu');
}

/** Closes the app menu with Escape (no-op when closed). */
export async function closeAppMenu(): Promise<void> {
  if ((await count('app-menu')) === 0) {
    return;
  }
  await pressEscape();
  // A submenu may need a second Escape.
  if ((await count('app-menu')) > 0) {
    await pressEscape();
  }
  await waitForAbsent('app-menu');
}

/**
 * Opens a submenu item (`X`) and waits for its menu (`X-menu`). Hover opens
 * submenus in most menus; a click is the fallback.
 */
export async function openSubmenu(itemId: string): Promise<void> {
  const item = await waitFor(itemId);
  await item.moveTo();
  try {
    await el(`${itemId}-menu`).waitForExist({ timeout: 1_500 });
    return;
  } catch {
    // Not opened on hover.
  }
  await item.click();
  await waitFor(`${itemId}-menu`);
}

/**
 * Activates an item of the app menu, opening the submenus on the way:
 * `appMenu('app-menu-settings')`, `appMenu('app-menu-view', 'app-menu-view-zoom-in')`.
 */
export async function appMenu(...path: string[]): Promise<void> {
  await openAppMenu();
  for (const submenu of path.slice(0, -1)) {
    await openSubmenu(submenu);
  }
  const last = path.at(-1);
  if (last === undefined) {
    return;
  }
  await (await waitFor(last)).click();
}

/**
 * Activates an item path in an open context menu:
 * `menuPath('host-menu-move-to', 'host-menu-move-to-top-level')`.
 */
export async function menuPath(...path: string[]): Promise<void> {
  for (const submenu of path.slice(0, -1)) {
    await openSubmenu(submenu);
  }
  const last = path.at(-1);
  if (last !== undefined) {
    await (await waitFor(last)).click();
  }
}

/** Selector of an element's info icon inside a menu item (`X-info`). */
export function infoIcon(itemId: string): string {
  return tid(`${itemId}-info`);
}
