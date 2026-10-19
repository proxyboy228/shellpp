// Element lookup by `data-testid` only (docs/architecture.md, section 9.4;
// D22). Repeated elements share a test id and carry their identity in a data
// attribute (`data-record-id`, `data-group-id`, `data-tag`, `data-id`,
// `data-index`), so lookups combine the test id with that attribute.
//
// WebdriverIO's `strictSelectors` is on: `$` throws when a selector matches
// more than one element. Use `els()` for repeated elements and narrow `el()`
// with an attribute or a parent.

/** CSS selector for a test id, optionally followed by more selector text (attributes). */
export function tid(id: string, extra = ''): string {
  return `[data-testid="${id}"]${extra}`;
}

/** CSS selector for a test id with one data attribute, e.g. `tidWith('host-card', 'record-id', id)`. */
export function tidWith(id: string, attribute: string, value: string): string {
  return tid(id, `[data-${attribute}="${value}"]`);
}

/** CSS selector for a test id inside an element with another test id. */
export function tidIn(parentId: string, childId: string, parentExtra = ''): string {
  return `${tid(parentId, parentExtra)} ${tid(childId)}`;
}

/** The single element with this test id (plus optional attribute selector text). */
export function el(id: string, extra = '') {
  return $(tid(id, extra));
}

/** All elements with this test id (plus optional attribute selector text). */
export function els(id: string, extra = '') {
  return $$(tid(id, extra));
}

/** Number of elements with this test id currently in the DOM. */
export async function count(id: string, extra = ''): Promise<number> {
  return browser.execute((selector: string) => document.querySelectorAll(selector).length, tid(id, extra));
}

/** True when at least one element with this test id is in the DOM (no waiting). */
export async function exists(id: string, extra = ''): Promise<boolean> {
  return (await count(id, extra)) > 0;
}

/** Waits until an element with this test id exists and returns it. */
export async function waitFor(id: string, extra = '', timeout?: number) {
  const element = el(id, extra);
  await element.waitForExist(timeout === undefined ? {} : { timeout });
  return element;
}

/**
 * Waits for a validation error that a form may show while typing or only on
 * submit: waits briefly, then clicks `submitId` (when enabled) and waits again.
 */
export async function waitForFieldError(errorId: string, submitId: string, timeout = 10_000): Promise<void> {
  try {
    await el(errorId).waitForExist({ timeout: 1_500 });
    return;
  } catch {
    // Not shown while typing.
  }
  const submit = el(submitId);
  if (await submit.isEnabled()) {
    await submit.click();
  }
  await el(errorId).waitForExist({ timeout });
}

/** Waits until no element with this test id exists. */
export async function waitForAbsent(id: string, extra = '', timeout = 10_000): Promise<void> {
  await browser.waitUntil(async () => (await count(id, extra)) === 0, {
    timeout,
    timeoutMsg: `expected ${tid(id, extra)} to be absent`,
  });
}

/** Attribute of every element with this test id, in document order. */
export async function attrOfAll(id: string, attribute: string, extra = ''): Promise<(string | null)[]> {
  return browser.execute(
    (selector: string, name: string) =>
      Array.from(document.querySelectorAll(selector)).map((node) => node.getAttribute(name)),
    tid(id, extra),
    attribute,
  );
}

/** Text of every element with this test id, in document order. */
export async function textOfAll(id: string, extra = ''): Promise<string[]> {
  return browser.execute(
    (selector: string) =>
      Array.from(document.querySelectorAll(selector)).map((node) => (node as HTMLElement).innerText.trim()),
    tid(id, extra),
  );
}

/**
 * Test ids of the menu items that belong directly to the menu with this test
 * id (not to its open submenus), in display order. Separators have no test id
 * and are skipped.
 */
export async function menuItemIds(menuId: string): Promise<string[]> {
  return browser.execute((selector: string) => {
    const menu = document.querySelector(selector);
    if (!menu) {
      return [];
    }
    const items = menu.querySelectorAll(
      '[role="menuitem"][data-testid], [role="menuitemcheckbox"][data-testid], [role="menuitemradio"][data-testid]',
    );
    return Array.from(items)
      .filter((item) => item.parentElement?.closest('[role="menu"]') === menu)
      .map((item) => item.getAttribute('data-testid') ?? '');
  }, tid(menuId));
}

/** Asserts that none of these test ids is in the DOM (elements of later stages, D3). */
export async function expectAbsent(ids: readonly string[]): Promise<void> {
  const present = await browser.execute(
    (list: string[]) => list.filter((id) => document.querySelector(`[data-testid="${id}"]`) !== null),
    [...ids],
  );
  expect(present).toEqual([]);
}

/** Asserts that every one of these test ids is in the DOM. */
export async function expectPresent(ids: readonly string[]): Promise<void> {
  const missing = await browser.execute(
    (list: string[]) => list.filter((id) => document.querySelector(`[data-testid="${id}"]`) === null),
    [...ids],
  );
  expect(missing).toEqual([]);
}
