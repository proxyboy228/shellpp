// Connect actions (UI 2.1, 2.4, 3.1, 3.3). Every connect goes through the
// terminal workspace (features/terminal `openSession`).
//
// Quick connect (UI 2.1): after a quick session reaches `connected`, an
// unobtrusive toast offers to save the host (`hosts-quick-connect-save-toast`).
// "Save host" creates it from the address, the port (when not 22) and the
// username the session used, then opens its panel on the Hosts screen.
//
// Owner: S7 (skeleton by the lead).

import { showToast } from '../../app/toast';
import { updateView, vaultsStore } from '../../app/store';
import { recordSave, toIpcError, type HostData, type RecordRef, type SessionInfo, type Uuid } from '../../ipc';
import { t } from '../../i18n';
import { newPanel, updateHostsViewState } from '../hosts/viewState';
import { sessionsStore, setQuickConnectOpener, watchSession, type OpenResult } from '../terminal';
import { activateTab, openSession } from './workspaceBridge';

export interface ConnectOptions {
  /** Alt held: open in a new app window (UI 2.4). */
  newWindow?: boolean;
}

/** How long the save offer stays (it has an action, so longer than a plain toast). */
export const SAVE_OFFER_MS = 10_000;

/** Connects to a saved host with its effective settings (double click, Enter, Connect). */
export function connectHost(host: RecordRef, options: ConnectOptions = {}): Promise<OpenResult | null> {
  return openSession({ kind: 'host', vault_id: host.vault_id, record_id: host.record_id }, options);
}

/** Connects to several saved hosts, one tab (or window) each (UI 2.4 multi-selection). */
export async function connectHosts(hosts: readonly RecordRef[], options: ConnectOptions = {}): Promise<void> {
  for (const host of hosts) {
    await connectHost(host, options);
  }
}

/**
 * Calls `listener` once when the session reaches `connected` (at once when it
 * already is). A failed attempt keeps watching (Retry may connect); `closed`
 * stops. Returns a function that stops watching.
 */
export function onSessionConnected(sessionId: Uuid, listener: (session: SessionInfo) => void): () => void {
  const current = sessionsStore.get().get(sessionId);
  if (current?.state === 'connected') {
    listener(current);
    return () => undefined;
  }
  let done = false;
  let unwatch: () => void = () => undefined;
  const stop = () => {
    done = true;
    unwatch();
  };
  unwatch = watchSession(sessionId, (info) => {
    if (done) {
      return;
    }
    if (info.state === 'connected') {
      stop();
      listener(info);
    } else if (info.state === 'closed') {
      stop();
    }
  });
  return stop;
}

/** The host a quick session would be saved as (UI 2.1 save offer). */
export function quickHostData(
  target: { address: string; port?: number; username?: string },
  session?: SessionInfo,
): HostData {
  const address = target.address;
  const port = session?.port ?? target.port;
  const username = session?.username ?? target.username;
  const ssh: NonNullable<HostData['ssh']> = {};
  if (port !== undefined && port !== 22) {
    ssh.port = port;
  }
  if (username !== undefined && username !== '') {
    ssh.username = username;
  }
  return Object.keys(ssh).length === 0 ? { address } : { address, ssh };
}

/** Saves a quick-connect target as a host and opens its panel on the Hosts screen. */
export async function saveQuickHost(data: HostData): Promise<Uuid | null> {
  const vault = vaultsStore.get().find((candidate) => candidate.kind === 'personal');
  if (vault === undefined) {
    return null;
  }
  try {
    const view = await recordSave({ vault_id: vault.vault_id, record_id: null, type: 'host', data });
    updateHostsViewState({
      folderId: null,
      selectedIds: [view.record_id],
      anchorId: view.record_id,
      panel: newPanel('host', view.record_id),
    });
    updateView({ vaultsSection: 'hosts' });
    activateTab('vaults');
    return view.record_id;
  } catch (reason) {
    showToast({ message: t(`errors.${toIpcError(reason).code}`), level: 'error', testId: 'hosts-error-toast' });
    return null;
  }
}

function offerToSave(target: { address: string; port?: number; username?: string }, session: SessionInfo): void {
  let used = false;
  const dismiss = showToast({
    message: t('connect.save_offer.message', { address: target.address }),
    level: 'info',
    durationMs: SAVE_OFFER_MS,
    testId: 'hosts-quick-connect-save-toast',
    action: {
      label: t('connect.save_offer.action'),
      testId: 'hosts-quick-connect-save-toast-action',
      onAction: () => {
        if (used) {
          return;
        }
        used = true;
        dismiss();
        void saveQuickHost(quickHostData(target, session));
      },
    },
  });
}

/**
 * Quick connect (UI 2.1) without a saved host; offers to save the host once
 * the session is connected. With Alt the session opens in a new window, and
 * that window makes the offer (its intent goes through this function too).
 */
export async function connectQuick(
  target: { address: string; port?: number; username?: string },
  options: ConnectOptions = {},
): Promise<OpenResult | null> {
  const result = await openSession({ kind: 'quick', ...target }, options);
  if (result !== null) {
    onSessionConnected(result.sessionId, (session) => offerToSave(target, session));
  }
  return result;
}

// A quick connect intent of a new window (Alt) opens with the save offer.
setQuickConnectOpener((target) =>
  connectQuick({
    address: target.address,
    ...(target.port === undefined ? {} : { port: target.port }),
    ...(target.username === undefined ? {} : { username: target.username }),
  }),
);
