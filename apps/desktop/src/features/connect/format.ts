// Display helpers of the connect flow (UI 4.8, 10.1, 10.2): addresses in the
// OpenSSH `[host]:port` form and the connection screen's `user@host:port`.
//
// Owner: S7.

import type { SessionError, SessionInfo } from '../../ipc';
import { t } from '../../i18n';

/** `host` for port 22 (or no port), `[host]:port` otherwise (UI 10.1, 10.2). */
export function hostKeyAddress(address: string, port: number | undefined): string {
  return port === undefined || port === 22 ? address : `[${address}]:${port}`;
}

/**
 * `user@host:port` for the connection screen: the port only when it is not
 * 22; an IPv6 address gets brackets when a port follows.
 */
export function connectionAddress(address: string, port?: number, username?: string): string {
  const user = username !== undefined && username !== '' ? `${username}@` : '';
  if (port === undefined || port === 22) {
    return `${user}${address}`;
  }
  const host = address.includes(':') ? `[${address}]` : address;
  return `${user}${host}:${port}`;
}

/** The address line of a session, or an empty string when the session has no address yet. */
export function sessionAddress(session: SessionInfo): string {
  if (session.address === undefined || session.address === '') {
    return '';
  }
  return connectionAddress(session.address, session.port, session.username);
}

/** The failure reason: `ssh_reason.<reason>` for SSH errors, else `errors.<code>`. */
export function sessionErrorMessage(error: SessionError | undefined): string {
  if (error === undefined) {
    return t('errors.internal');
  }
  if (error.code === 'ssh_error' && error.reason !== undefined) {
    return t(`ssh_reason.${error.reason}`);
  }
  return t(`errors.${error.code}`);
}
