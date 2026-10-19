// Window session: boot with the snapshot protocol and keep the window stores
// in sync with the core (docs/architecture.md, section 3.5; plan, "Events").
//
// 1. Subscribe to the broadcast events with a buffer.
// 2. Call `state_snapshot`, fill the stores, release the buffer with the
//    snapshot's `v` (only newer events are applied).
// 3. On `lock:changed` and `vaults:changed`: take a new snapshot (events that
//    arrive meanwhile are queued and applied afterwards when newer).
//    `settings:changed` reloads the settings; `notifications:changed` updates
//    the list and shows a toast for a new notification; `records:changed` is
//    handled by the screens that show records.
// Window-targeted events: `link:open` (host link) and `menu:action` (macOS
// native menu bar).
//
// Owner: lead (stage 2 wiring).

import { t } from '../i18n';
import {
  appInfo,
  createEventBuffer,
  getBackend,
  settingsGet,
  stateSnapshot,
  subscribe,
  toIpcError,
  type BufferedEvent,
  type NotificationsChangedEvent,
  type StateSnapshot,
  type Unlisten,
} from '../ipc';
import { applySnapshotSessions } from '../features/terminal';
import { runAppMenuAction } from './AppMenu/actions';
import {
  appInfoStore,
  dialogsStore,
  INITIAL_DIALOGS_STATE,
  lockStore,
  notificationsStore,
  selectRequestStore,
  settingsStore,
  updateView,
  vaultsStore,
  viewStore,
  windowStore,
} from './store';
import { clearToasts, showToast } from './toast';

/** How long a notification toast stays (UI 1.5). */
export const NOTIFICATION_TOAST_MS = 5000;
const MAX_NOTIFICATIONS = 200;

interface Session {
  resync(): void;
}

let active: Session | null = null;
let linkNonce = 0;

/** Takes a new snapshot in the running session (after unlock, lock, recovery). */
export function requestResync(): void {
  active?.resync();
}

/** Starts the session of this window; returns a function that stops it. */
export function startSession(): () => void {
  let disposed = false;
  const unlisteners: Unlisten[] = [];
  let currentV = -1;
  let lastSeenNotificationId = 0;
  let resyncing = false;
  let resyncAgain = false;
  let queued: BufferedEvent[] | null = null;
  let settingsRequest = 0;

  const track = (unlisten: Unlisten) => {
    if (disposed) {
      unlisten();
    } else {
      unlisteners.push(unlisten);
    }
  };

  const applySnapshot = (snapshot: StateSnapshot) => {
    currentV = snapshot.v;
    const previousState = lockStore.get()?.state;
    windowStore.set({ phase: 'ready', errorCode: null, errorFound: null, label: snapshot.window_label });
    settingsStore.set(snapshot.settings);
    notificationsStore.set(snapshot.notifications);
    vaultsStore.set(snapshot.vaults);
    for (const item of snapshot.notifications.items) {
      lastSeenNotificationId = Math.max(lastSeenNotificationId, item.id);
    }
    if (snapshot.lock.state !== 'unlocked' && previousState !== snapshot.lock.state) {
      // Locked: drop transient UI (toasts, dialogs); the view state stays.
      clearToasts();
      dialogsStore.set(INITIAL_DIALOGS_STATE);
    }
    if (snapshot.lock.state !== 'first_run' && snapshot.lock.state !== 'unlocked') {
      updateView({ firstRunInProgress: false });
    }
    if (snapshot.lock.state === 'unlocked' && viewStore.get().unlockMode !== 'password') {
      // Unlocked by any path (another window, too): the next lock shows the
      // unlock screen with the password field, not the recovery screen (UI 13.2).
      updateView({ unlockMode: 'password' });
    }
    lockStore.set(snapshot.lock);
    // Stage 2: this window's sessions (and, once, the intent of a new window).
    applySnapshotSessions(snapshot.sessions, snapshot.intent, {
      settings: snapshot.settings,
      launchedAt: snapshot.launched_at,
      unlocked: snapshot.lock.state === 'unlocked',
    });
  };

  const refreshSettings = async () => {
    settingsRequest += 1;
    const request = settingsRequest;
    try {
      const values = await settingsGet();
      if (!disposed && request === settingsRequest) {
        settingsStore.set(values);
      }
    } catch {
      // Keep the current values; the next event or snapshot refreshes them.
    }
  };

  const applyNotifications = (payload: NotificationsChangedEvent) => {
    const latest = payload.latest;
    notificationsStore.set((current) => {
      let items = current.items;
      if (latest !== undefined && !items.some((item) => item.id === latest.id)) {
        items = [latest, ...items].slice(0, MAX_NOTIFICATIONS);
      }
      if (payload.unread === 0) {
        items = items.map((item) => (item.read ? item : { ...item, read: true }));
      }
      return { items, unread: payload.unread };
    });
    if (latest !== undefined && latest.id > lastSeenNotificationId) {
      lastSeenNotificationId = latest.id;
      const unlocked = lockStore.get()?.state === 'unlocked' && !viewStore.get().firstRunInProgress;
      if (unlocked) {
        showToast({
          message: t(latest.message_key, latest.params),
          level: latest.level,
          durationMs: NOTIFICATION_TOAST_MS,
          testId: 'notification-toast',
          dataId: latest.id,
        });
      }
    }
  };

  const handleEvent = (event: BufferedEvent) => {
    if (disposed) {
      return;
    }
    if (queued !== null) {
      queued.push(event);
      return;
    }
    if (event.payload.v <= currentV) {
      return;
    }
    currentV = event.payload.v;
    switch (event.name) {
      case 'lock:changed':
      case 'vaults:changed':
        resync();
        break;
      case 'settings:changed':
        void refreshSettings();
        break;
      case 'notifications:changed':
        applyNotifications(event.payload);
        break;
      case 'records:changed':
        break;
    }
  };

  const resync = () => {
    if (disposed) {
      return;
    }
    if (resyncing) {
      resyncAgain = true;
      return;
    }
    resyncing = true;
    queued = [];
    void (async () => {
      try {
        do {
          resyncAgain = false;
          const snapshot = await stateSnapshot();
          if (disposed) {
            return;
          }
          applySnapshot(snapshot);
        } while (resyncAgain);
      } catch {
        // Keep the current state; the next event retries.
      } finally {
        resyncing = false;
        const pending = queued ?? [];
        queued = null;
        for (const event of pending) {
          handleEvent(event);
        }
      }
    })();
  };

  const session: Session = { resync };
  active = session;

  void (async () => {
    try {
      const info = await appInfo();
      if (!disposed) {
        appInfoStore.set(info);
      }
    } catch {
      // The window works without it (no OS-specific layout).
    }
    let release: ((version: number) => void) | null = null;
    try {
      const buffer = await createEventBuffer(handleEvent);
      track(buffer.unlisten);
      release = buffer.release;
      track(
        await subscribe('link:open', (payload) => {
          linkNonce += 1;
          selectRequestStore.set({ record_id: payload.record_id, nonce: linkNonce });
        }),
      );
      track(await subscribe('menu:action', (payload) => runAppMenuAction(payload.id)));
    } catch {
      // Without events the window still shows the snapshot.
    }
    if (disposed) {
      return;
    }
    try {
      const snapshot = await stateSnapshot();
      if (disposed) {
        return;
      }
      applySnapshot(snapshot);
      release?.(snapshot.v);
    } catch (reason) {
      if (disposed) {
        return;
      }
      const error = toIpcError(reason);
      windowStore.set({
        phase: 'error',
        errorCode: error.code,
        errorFound: typeof error.details.found === 'number' ? error.details.found : null,
        label: getBackend().window.label,
      });
    }
  })();

  return () => {
    disposed = true;
    if (active === session) {
      active = null;
    }
    for (const unlisten of unlisteners.splice(0)) {
      unlisten();
    }
  };
}
