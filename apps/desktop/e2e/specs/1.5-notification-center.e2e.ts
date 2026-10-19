// UI 1.5 Notification center: newest first, unread counter, opening marks
// all read, a toast for 5 s for a new notification, memory only. Stage 1 has
// no producer besides the e2e mock (`notify`).

import {
  attrOfAll,
  count,
  el,
  mock,
  pressEscape,
  restartAndUnlock,
  startFresh,
  waitFor,
  waitForAbsent,
  type Session,
} from '../helpers/index.ts';

/** An i18n key that exists in every build (the mock passes it to the core as is). */
const MESSAGE_KEY = 'app.name';

async function openList(): Promise<void> {
  if ((await count('notifications-popover')) === 0) {
    await el('titlebar-notifications-button').click();
  }
  await waitFor('notifications-popover');
}

async function closeList(): Promise<void> {
  if ((await count('notifications-popover')) > 0) {
    await pressEscape();
    if ((await count('notifications-popover')) > 0) {
      await el('titlebar-notifications-button').click();
    }
    await waitForAbsent('notifications-popover');
  }
}

describe('UI 1.5 Notification center', () => {
  let session: Session;

  before(async () => {
    session = await startFresh();
  });

  it('shows the empty state when there are no notifications', async () => {
    await openList();
    await expect(el('notifications-empty')).toBeDisplayed();
    expect(await count('notifications-item')).toBe(0);
    await closeList();
  });

  it('shows a new notification as a toast for about 5 seconds and counts it as unread', async () => {
    await mock({ kind: 'notify', level: 'info', message_key: MESSAGE_KEY });
    const toast = await waitFor('notification-toast');
    const shownAt = Date.now();
    await expect(el('titlebar-notifications-badge')).toHaveText('1');
    await toast.waitForExist({ reverse: true, timeout: 10_000 });
    const shownFor = Date.now() - shownAt;
    expect(shownFor).toBeGreaterThan(3_500);
    expect(shownFor).toBeLessThan(8_000);
    // The notification stays in the list.
    await expect(el('titlebar-notifications-badge')).toHaveText('1');
  });

  it('lists the notification and marks everything read when the list opens', async () => {
    await openList();
    expect(await count('notifications-item')).toBe(1);
    await waitForAbsent('titlebar-notifications-badge');
    await browser.waitUntil(
      async () => (await attrOfAll('notifications-item', 'data-read')).every((read) => read === 'true'),
      { timeout: 5_000, timeoutMsg: 'the notification was not marked read' },
    );
    await closeList();
  });

  it('lists notifications newest first', async () => {
    await mock({ kind: 'notify', level: 'warning', message_key: MESSAGE_KEY });
    await mock({ kind: 'notify', level: 'error', message_key: MESSAGE_KEY });
    await expect(el('titlebar-notifications-badge')).toHaveText('2');
    await openList();
    await browser.waitUntil(async () => (await count('notifications-item')) === 3);
    const ids = (await attrOfAll('notifications-item', 'data-id')).map((id) => Number(id));
    expect(ids.every((id) => Number.isFinite(id))).toBe(true);
    expect([...ids].sort((a, b) => b - a)).toEqual(ids);
    await waitForAbsent('titlebar-notifications-badge');
    await closeList();
  });

  it('keeps notifications in memory only (empty after a restart)', async () => {
    await mock({ kind: 'notify', level: 'info', message_key: MESSAGE_KEY });
    await restartAndUnlock(session.password);
    await expect(el('titlebar-notifications-badge')).not.toBeExisting();
    await openList();
    await expect(el('notifications-empty')).toBeDisplayed();
    await closeList();
  });
});
