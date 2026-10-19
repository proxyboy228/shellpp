// Recognizes a connection address in the search bar (UI 2.1, D23):
// `host`, `user@host`, `user@host:port`, `ssh user@host -p port`.
//
// A single word without a user counts as an address only when it contains a
// dot or a colon, is an IP address, or is `localhost`; any other single word
// is only a search query. The Connect action itself is stage 2
// (`hosts-quick-connect`); stage 1 uses this parser only to decide whether
// the hidden Connect button would be enabled.

import { isIpv4, isIpv6Literal, parseAddress, parseInteger } from '../panels/validators';

export interface QuickConnectTarget {
  address: string;
  username?: string;
  port?: number;
}

function parsePort(text: string): number | null {
  const result = parseInteger(text, 1, 65535);
  return result.ok && result.value !== undefined ? result.value : null;
}

interface Destination {
  address: string;
  username?: string;
  port?: number;
}

/** `[user@]host[:port]`, `[user@][ipv6][:port]` or `[user@]ipv6`. */
function parseDestination(token: string): Destination | null {
  let rest = token;
  let username: string | undefined;
  const at = rest.lastIndexOf('@');
  if (at >= 0) {
    username = rest.slice(0, at);
    rest = rest.slice(at + 1);
    if (username === '' || /[\s@]/.test(username)) {
      return null;
    }
  }
  let host = rest;
  let port: number | undefined;
  if (rest.startsWith('[')) {
    const close = rest.indexOf(']');
    if (close < 0) {
      return null;
    }
    host = rest.slice(1, close);
    const after = rest.slice(close + 1);
    if (after !== '') {
      if (!after.startsWith(':')) {
        return null;
      }
      const parsed = parsePort(after.slice(1));
      if (parsed === null) {
        return null;
      }
      port = parsed;
    }
    if (!isIpv6Literal(host)) {
      return null;
    }
  } else {
    const colons = (rest.match(/:/g) ?? []).length;
    if (colons === 1) {
      const index = rest.indexOf(':');
      host = rest.slice(0, index);
      const parsed = parsePort(rest.slice(index + 1));
      if (parsed === null) {
        return null;
      }
      port = parsed;
    }
  }
  const address = parseAddress(host);
  if (!address.ok || address.value === undefined) {
    return null;
  }
  return { address: address.value, username, port };
}

/** A bare word without a user: an address only with a dot or a colon, an IP, or `localhost`. */
function looksLikeAddress(word: string): boolean {
  return word.includes('.') || word.includes(':') || isIpv4(word) || word.toLowerCase() === 'localhost';
}

/** The connection target typed in the search bar, or null when the text is only a search query. */
export function parseQuickConnect(text: string): QuickConnectTarget | null {
  const tokens = text.trim().split(/\s+/).filter((token) => token !== '');
  if (tokens.length === 0) {
    return null;
  }
  const explicitSsh = tokens[0] === 'ssh';
  const args = explicitSsh ? tokens.slice(1) : tokens;
  let portOption: number | undefined;
  const positional: string[] = [];
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index] ?? '';
    if (explicitSsh && arg === '-p') {
      const value = args[index + 1];
      const parsed = value === undefined ? null : parsePort(value);
      if (parsed === null) {
        return null;
      }
      portOption = parsed;
      index += 1;
    } else if (explicitSsh && /^-p\d+$/.test(arg)) {
      const parsed = parsePort(arg.slice(2));
      if (parsed === null) {
        return null;
      }
      portOption = parsed;
    } else if (arg.startsWith('-')) {
      return null;
    } else {
      positional.push(arg);
    }
  }
  if (positional.length !== 1) {
    return null;
  }
  const token = positional[0] ?? '';
  const destination = parseDestination(token);
  if (destination === null) {
    return null;
  }
  if (!explicitSsh && destination.username === undefined && destination.port === undefined && !looksLikeAddress(token)) {
    return null;
  }
  if (portOption !== undefined && destination.port !== undefined && portOption !== destination.port) {
    return null;
  }
  const target: QuickConnectTarget = { address: destination.address };
  if (destination.username !== undefined) {
    target.username = destination.username;
  }
  const port = portOption ?? destination.port;
  if (port !== undefined) {
    target.port = port;
  }
  return target;
}
