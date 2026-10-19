// Pointer input through WebDriver actions. The app implements drag and drop
// with pointer events (stage plan, decision 13), which these actions drive on
// WebView2 and WebKitGTK alike.

import { Key } from 'webdriverio';

type AnyElement = WebdriverIO.Element | ReturnType<typeof $>;

async function resolve(element: AnyElement): Promise<WebdriverIO.Element> {
  const resolved = await element;
  await resolved.waitForExist();
  // DOM scrolling instead of wheel actions, which not every native driver supports.
  await browser.execute((node: HTMLElement) => node.scrollIntoView({ block: 'center', inline: 'center' }), resolved);
  return resolved;
}

/** Ctrl+click (UI 2.3 multi-selection). */
export async function ctrlClick(element: AnyElement): Promise<void> {
  await clickWithKey(element, Key.Control);
}

/** Shift+click (UI 2.3 range selection). */
export async function shiftClick(element: AnyElement): Promise<void> {
  await clickWithKey(element, Key.Shift);
}

async function clickWithKey(element: AnyElement, key: string): Promise<void> {
  const target = await resolve(element);
  // Ticks: (key down, move) (pause, press) (pause, release) (key up, pause).
  await browser.actions([
    browser.action('key').down(key).pause(20).pause(20).up(key),
    browser.action('pointer', { parameters: { pointerType: 'mouse' } }).move({ origin: target }).down().up().pause(20),
  ]);
}

/** Right click (context menu). */
export async function rightClick(element: AnyElement): Promise<void> {
  const target = await resolve(element);
  await target.click({ button: 'right' });
}

/** Double click. */
export async function doubleClick(element: AnyElement): Promise<void> {
  const target = await resolve(element);
  await target.doubleClick();
}

export type DropPosition = 'before' | 'after' | 'center';

/**
 * Drags `source` onto `target`. `before` and `after` drop near the leading or
 * trailing edge of the target along `axis` (`x` for the grid, `y` for the
 * list); `center` drops onto the middle (into a group card). Moves in small
 * steps so the 5 px drag threshold and hover feedback trigger.
 */
export async function dragTo(
  source: AnyElement,
  target: AnyElement,
  position: DropPosition = 'center',
  axis: 'x' | 'y' = 'x',
): Promise<void> {
  const from = await resolve(source);
  const to = await resolve(target);
  const size = await to.getSize();
  const extent = axis === 'x' ? size.width : size.height;
  const edge = Math.max(2, Math.floor(extent / 2) - Math.max(4, Math.floor(extent * 0.15)));
  const offset = position === 'before' ? -edge : position === 'after' ? edge : 0;
  const x = axis === 'x' ? offset : 0;
  const y = axis === 'y' ? offset : 0;
  await browser
    .action('pointer', { parameters: { pointerType: 'mouse' } })
    .move({ origin: from })
    .down()
    .pause(50)
    .move({ origin: 'pointer', x: 6, y: 6, duration: 50 })
    .move({ origin: 'pointer', x: 6, y: 6, duration: 50 })
    .move({ origin: to, x: Math.round(x / 2), y: Math.round(y / 2), duration: 150 })
    .move({ origin: to, x, y, duration: 100 })
    .pause(150)
    .up()
    .perform();
}

/** Drags an element by a pixel offset (resize handles). */
export async function dragBy(element: AnyElement, dx: number, dy = 0): Promise<void> {
  const handle = await resolve(element);
  await browser
    .action('pointer', { parameters: { pointerType: 'mouse' } })
    .move({ origin: handle })
    .down()
    .pause(50)
    .move({ origin: 'pointer', x: Math.round(dx / 2), y: Math.round(dy / 2), duration: 100 })
    .move({ origin: 'pointer', x: dx - Math.round(dx / 2), y: dy - Math.round(dy / 2), duration: 100 })
    .pause(100)
    .up()
    .perform();
}
