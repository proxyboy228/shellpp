// Mock sessions, local terminal and history (stage 2 plan, section 6.6).
//
// An in-memory echo shell that follows the shell contract of the russh test
// server (plan 4.1: `echo`, `exit [n]`, `read-secret`, `alt-screen`, `size`,
// `term`, `lines`, `bell`, `url`, `title`, `unicode`, `color`, `later`,
// `drop`, ...) so Vitest and `pnpm dev` exercise the terminal:
//
// - SSH sessions walk the connect steps (resolving, connecting, host key,
//   authenticating, opening session) with prompts: TOFU for an address that
//   is not in `ctx.knownHosts` (a changed key after `rotateHostKey`; a revoked
//   entry fails with `host_key_revoked`), the username when none is set, then
//   the password when the host has no saved one. Any non-empty password is
//   accepted except one that starts with `wrong` (mock rule for the error
//   path; tests generate their passwords at run time). Username `otp` adds a
//   keyboard-interactive question.
// - Output goes through the attached channel (`ctx.channels`, `ArrayBuffer`
//   of UTF-8) after a replay of the last 256 KiB.
// - Every change emits `sessions:changed` with an increasing `rev`.
// - `exit 0` closes, `exit n` ends, `drop` drops; with `terminal.autoreconnect`
//   attempts follow after 1, 2, 4, 8, 16, 32 s, then every 60 s (real timers,
//   so Vitest fake timers drive them); while locked they wait for unlock.
// - History: typed candidates that pass a minimal detector go to
//   `ctx.history` (the real capture rules and the secret detector are tested
//   in Rust); a line with injected input is never recorded.
//
// Keys and fingerprints are made-up strings, never real key material.
//
// Owner: S6.

import { IpcError } from '../errors';
import type {
  Backspace,
  ConnectStep,
  HistoryCandidate,
  HostData,
  HostKeyInfo,
  LocalShell,
  PromptKind,
  PromptReply,
  RecordRef,
  SessionError,
  SessionInfo,
  SessionTarget,
  SshReason,
  ThemeRef,
} from '../types';
import { CLASS_CONNECTION_LOG, CLASS_HISTORY, CLASS_HOSTS, CLASS_KNOWN_HOSTS, type MockHandler, type Stage2MockContext } from './context';

export type SessionCommand =
  | 'session_open'
  | 'session_attach'
  | 'session_write'
  | 'session_inject'
  | 'session_resize'
  | 'session_prompt_reply'
  | 'session_cancel'
  | 'session_close'
  | 'session_reconnect'
  | 'session_ack'
  | 'local_shells'
  | 'local_open'
  | 'history_list'
  | 'history_delete'
  | 'history_clear';

export type SessionHandlers = { [C in SessionCommand]: MockHandler<C> };

/** Test helpers for sessions (exposed as `backend.control.sessions`). */
export interface SessionMockControl {
  /** Sessions of this window, newest last. */
  list(): SessionInfo[];
  /** Simulates a transport drop of the session. */
  drop(sessionId: string): void;
  /** Bytes written to the session so far (typed and injected), as text. */
  input(sessionId: string): string;
  /** Pushes output to the session as the server would. */
  output(sessionId: string, text: string): void;
  /** While on, new connection attempts fail with `ssh_error` / `refused` (a closed port). */
  setRefusing(refusing: boolean): void;
  /** The mock server at this address presents a new host key (the next connection sees a changed key). */
  rotateHostKey(address: string, port?: number): void;
}

export interface SessionMock {
  handlers: SessionHandlers;
  control: SessionMockControl;
  /** For `state_snapshot.sessions`. */
  snapshot(): SessionInfo[];
  /** Active (not closed) sessions, for `app_exit` / `window_close`. */
  activeCount(): number;
  /** Called on lock and unlock (reconnect attempts wait for unlock). */
  onLockChanged(locked: boolean): void;
}

const STEP_DELAY_MS = import.meta.env.MODE === 'test' ? 0 : 150;
const REPLAY_LIMIT = 256 * 1024;
const MAX_WRITE_BYTES = 1024 * 1024;
const BACKOFF_MS = [1000, 2000, 4000, 8000, 16_000, 32_000];
const BACKOFF_STEADY_MS = 60_000;
const MOTD = 'Welcome to shellpp-test-sshd\r\n';
const SECRET_PROMPT = '[sudo] password for test: ';

/** A deterministic, non-cryptographic hash (mock data only). */
function fnv(text: string, seed = 0x811c9dc5): number {
  let hash = seed >>> 0;
  for (let index = 0; index < text.length; index += 1) {
    hash ^= text.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash >>> 0;
}

const B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';

function fakeBase64(text: string, length: number): string {
  let out = '';
  let state = fnv(text);
  while (out.length < length) {
    state = fnv(String(state), state ^ 0x9e3779b9);
    out += B64[state % 64];
  }
  return out;
}

/** The made-up fingerprint of a mock public key (`SHA256:` + 43 characters). */
export function mockFingerprint(publicKey: string): string {
  return `SHA256:${fakeBase64(`fp:${publicKey}`, 43)}`;
}

/** The made-up `ssh-ed25519` public key the mock server at an address presents (after `rotation` rotations). */
export function mockHostPublicKey(address: string, port = 22, rotation = 0): string {
  return `AAAAC3NzaC1lZDI1NTE5AAAAI${fakeBase64(`${address.toLowerCase()}:${port}:${rotation}`, 43)}`;
}

const SHELLS_PC: readonly LocalShell[] = [
  { id: 'powershell', label: 'PowerShell' },
  { id: 'cmd', label: 'Command Prompt' },
];
const SHELLS_MAC: readonly LocalShell[] = [
  { id: 'zsh', label: 'zsh' },
  { id: 'bash', label: 'bash' },
];

const PASSWORD_PROMPT = /\b(password|passwd|passphrase|passcode|pin|otp|one[- ]time|verification code|token|2fa|authenticator|secret)\b/i;
const SECRET_PATTERNS: readonly RegExp[] = [
  /(^|\s)-p\S+/,
  /--password/i,
  /[a-z0-9_]*password=/i,
  /[a-z0-9_]*token=/i,
  /secret/i,
  /api[_-]?key/i,
  /[^\s:@/]+:[^\s@/]+@/,
  /authorization:/i,
  /\bbearer\b/i,
  /\b(ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36}\b/,
  /\b(AKIA|ASIA)[A-Z0-9]{16}\b/,
  /-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----/,
];

/** A minimal stand-in for the core's capture rules and secret detector (docs/security.md 10.1-10.2). */
export function mockHistoryAccepts(candidate: HistoryCandidate): boolean {
  const command = candidate.command;
  if (candidate.alt_screen || command.trim() === '' || command.startsWith(' ') || command.length > 4096) {
    return false;
  }
  if (candidate.typed_printable && /^[*•]+$/.test(command)) {
    return false;
  }
  if (PASSWORD_PROMPT.test(candidate.prompt_text)) {
    return false;
  }
  return !SECRET_PATTERNS.some((pattern) => pattern.test(command));
}

class Aborted extends Error {}

interface PendingPrompt {
  promptId: number;
  resolve(reply: PromptReply): void;
  reject(reason: Aborted): void;
}

type AttemptKind = 'first' | 'retry' | 'reconnect';

interface MockSession {
  info: SessionInfo;
  cols: number;
  rows: number;
  replay: string[];
  replaySize: number;
  channelId: number | null;
  input: string;
  // Echo shell state.
  line: string;
  mode: 'normal' | 'secret' | 'alt';
  escape: string | null;
  shellPrompt: string;
  lineInjected: boolean;
  // Connection state.
  host: RecordRef | null;
  username: string | undefined;
  savedPassword: string | undefined;
  keptPassword: string | null;
  keptUsername: string | null;
  onceKey: string | null;
  pending: PendingPrompt | null;
  /** Changes whenever the current attempt must stop (cancel, close, a new attempt). */
  token: number;
  timers: Set<ReturnType<typeof setTimeout>>;
  reconnectTimer: ReturnType<typeof setTimeout> | null;
}

/** Drops `undefined` members: optional output fields are omitted, never null. */
function clean(info: SessionInfo): SessionInfo {
  const copy = { ...info } as Record<string, unknown>;
  for (const key of Object.keys(copy)) {
    if (copy[key] === undefined) {
      delete copy[key];
    }
  }
  return structuredClone(copy) as unknown as SessionInfo;
}

function validateSize(cols: number, rows: number): void {
  const errors: { field: string; reason: 'out_of_range' }[] = [];
  if (!Number.isInteger(cols) || cols < 1 || cols > 1000) {
    errors.push({ field: 'cols', reason: 'out_of_range' });
  }
  if (!Number.isInteger(rows) || rows < 1 || rows > 1000) {
    errors.push({ field: 'rows', reason: 'out_of_range' });
  }
  if (errors.length > 0) {
    throw new IpcError('validation_failed', { errors });
  }
}

function validation(field: string, reason: 'required' | 'invalid' | 'out_of_range' | 'too_long'): IpcError {
  return new IpcError('validation_failed', { errors: [{ field, reason }] });
}

export function createSessionMock(ctx: Stage2MockContext): SessionMock {
  const sessions = new Map<string, MockSession>();
  const rotations = new Map<string, number>();
  let promptCounter = 0;
  let refusing = false;

  const encoder = new TextEncoder();
  const settings = () => ctx.settings();
  const boolSetting = (key: string, fallback: boolean) => {
    const value = settings()[key];
    return typeof value === 'boolean' ? value : fallback;
  };
  const defaultTheme = (): ThemeRef => {
    const value = settings()['terminal.default_theme'];
    return typeof value === 'object' && value !== null && !Array.isArray(value) && typeof value.builtin === 'string'
      ? { builtin: value.builtin }
      : { builtin: 'graphite' };
  };
  const shells = (): readonly LocalShell[] => (settings()['shortcuts.preset'] === 'mac' ? SHELLS_MAC : SHELLS_PC);

  const find = (sessionId: string): MockSession => {
    const session = sessions.get(sessionId);
    if (session === undefined) {
      throw new IpcError('not_found');
    }
    return session;
  };

  // --- Events and output ------------------------------------------------------

  const update = (session: MockSession, patch: Partial<SessionInfo>) => {
    session.info = clean({ ...session.info, ...patch, rev: session.info.rev + 1 });
    ctx.emit('sessions:changed', clean(session.info));
    if (session.info.state === 'closed') {
      for (const timer of session.timers) {
        clearTimeout(timer);
      }
      session.timers.clear();
      if (session.reconnectTimer !== null) {
        clearTimeout(session.reconnectTimer);
      }
      sessions.delete(session.info.session_id);
    }
  };

  const deliver = (channelId: number, text: string) => {
    const bytes = encoder.encode(text);
    const buffer = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
    queueMicrotask(() => ctx.channels.get(channelId)?.(buffer));
  };

  const output = (session: MockSession, text: string) => {
    if (text === '') {
      return;
    }
    session.replay.push(text);
    session.replaySize += text.length;
    while (session.replaySize > REPLAY_LIMIT && session.replay.length > 1) {
      session.replaySize -= (session.replay.shift() as string).length;
    }
    if (session.channelId !== null) {
      deliver(session.channelId, text);
    }
  };

  const later = (session: MockSession, ms: number, run: () => void) => {
    const timer = setTimeout(() => {
      session.timers.delete(timer);
      run();
    }, ms);
    session.timers.add(timer);
  };

  const delay = (session: MockSession, token: number) =>
    new Promise<void>((resolve, reject) => {
      later(session, STEP_DELAY_MS, () => (session.token === token ? resolve() : reject(new Aborted())));
    });

  // --- Host keys and known hosts ------------------------------------------------

  const hostKey = (address: string, port: number): HostKeyInfo & { public_key: string } => {
    const rotation = rotations.get(`${address.toLowerCase()}:${port}`) ?? 0;
    const publicKey = mockHostPublicKey(address, port, rotation);
    return { address, port, key_type: 'ssh-ed25519', fingerprint: mockFingerprint(publicKey), public_key: publicKey };
  };

  const entryMatches = (entry: Stage2MockContext['knownHosts'][number], address: string, port: number) => {
    const host = entry.host.toLowerCase();
    const lower = address.toLowerCase();
    return (host === lower && (entry.port ?? 22) === port) || host === `[${lower}]:${port}`;
  };

  const checkKnownHost = (address: string, port: number, publicKey: string) => {
    if (ctx.knownHosts.some((entry) => entry.revoked && entry.public_key === publicKey)) {
      return { kind: 'revoked' as const };
    }
    const entries = ctx.knownHosts.filter((entry) => !entry.revoked && entryMatches(entry, address, port));
    if (entries.some((entry) => entry.public_key === publicKey)) {
      return { kind: 'match' as const };
    }
    const old = entries[0];
    return old === undefined ? { kind: 'unknown' as const } : { kind: 'changed' as const, old };
  };

  const saveKnownHost = (address: string, port: number, key: HostKeyInfo & { public_key: string }, replace: boolean) => {
    const at = new Date(ctx.now()).toISOString();
    const ids: string[] = [];
    if (replace) {
      for (let index = ctx.knownHosts.length - 1; index >= 0; index -= 1) {
        const entry = ctx.knownHosts[index];
        if (entry !== undefined && !entry.revoked && entryMatches(entry, address, port)) {
          ids.push(entry.record_id);
          ctx.knownHosts.splice(index, 1);
        }
      }
    }
    const id = ctx.newId();
    ctx.knownHosts.push({
      record_id: id,
      host: address,
      port: port === 22 ? undefined : port,
      key_type: key.key_type,
      public_key: key.public_key,
      revoked: false,
      source: replace ? 'replaced' : 'tofu',
      created_at: at,
      modified_at: at,
    });
    ctx.recordsChanged([id, ...ids], [CLASS_KNOWN_HOSTS]);
  };

  // --- Prompts --------------------------------------------------------------------

  const ask = (session: MockSession, token: number, prompt: PromptKind): Promise<PromptReply> => {
    if (session.token !== token) {
      return Promise.reject(new Aborted());
    }
    promptCounter += 1;
    const promptId = promptCounter;
    return new Promise<PromptReply>((resolve, reject) => {
      session.pending = { promptId, resolve, reject };
      update(session, { prompt: { ...prompt, prompt_id: promptId } });
    });
  };

  /** Removes an answered prompt from the session view (no event when there is none). */
  const promptAnswered = (session: MockSession) => {
    if (session.info.prompt !== undefined) {
      update(session, { prompt: undefined });
    }
  };

  const clearPrompt = (session: MockSession) => {
    if (session.pending !== null) {
      const pending = session.pending;
      session.pending = null;
      pending.reject(new Aborted());
    }
  };

  // --- Shell ---------------------------------------------------------------------

  const printPrompt = (session: MockSession) => output(session, session.shellPrompt);

  const endSession = (session: MockSession, status: number) => {
    session.token += 1;
    if (status === 0) {
      update(session, { state: 'closed', prompt: undefined, step: undefined });
    } else {
      update(session, { state: 'ended', exit_status: status, step: undefined, reconnect: undefined, error: undefined });
    }
  };

  const run = (session: MockSession, line: string) => {
    const trimmed = line.trim();
    if (trimmed === '') {
      printPrompt(session);
      return;
    }
    const [command = '', ...args] = trimmed.split(/\s+/);
    const rest = trimmed.slice(command.length).trim();
    const lines = (...texts: string[]) => output(session, texts.map((text) => `${text}\r\n`).join(''));
    switch (command) {
      case 'echo':
        lines(rest);
        break;
      case 'exit': {
        const status = args[0] === undefined ? 0 : Number.parseInt(args[0], 10);
        endSession(session, Number.isFinite(status) ? status : 0);
        return;
      }
      case 'read-secret':
        output(session, SECRET_PROMPT);
        session.mode = 'secret';
        return;
      case 'alt-screen':
        output(session, '\x1b[?1049hALT SCREEN (press q)');
        session.mode = 'alt';
        return;
      case 'size':
        lines(`${session.cols}x${session.rows}`);
        break;
      case 'term': {
        const term = settings()['terminal.term'];
        lines(typeof term === 'string' ? term : 'xterm-256color');
        break;
      }
      case 'env':
        lines('');
        break;
      case 'agent-keys':
        lines('agent: not forwarded');
        break;
      case 'lines': {
        const count = Math.min(10_000, Math.max(0, Number.parseInt(args[0] ?? '0', 10) || 0));
        lines(...Array.from({ length: count }, (_value, index) => `line ${index + 1}`));
        break;
      }
      case 'bell':
        output(session, '\x07');
        break;
      case 'url':
        lines('https://example.com/docs');
        break;
      case 'title':
        output(session, `\x1b]0;${rest}\x07`);
        break;
      case 'unicode':
        lines('wide: 日本語 ✓ 👍');
        break;
      case 'color':
        lines(Array.from({ length: 16 }, (_value, index) => `\x1b[${index < 8 ? 30 + index : 82 + index}m${index}`).join(' ') + '\x1b[0m');
        break;
      case 'sleep': {
        const seconds = Math.min(60, Math.max(0, Number(args[0] ?? 0) || 0));
        later(session, seconds * 1000, () => printPrompt(session));
        return;
      }
      case 'later': {
        const seconds = Math.min(60, Math.max(0, Number(args[0] ?? 0) || 0));
        const text = args.slice(1).join(' ');
        printPrompt(session);
        later(session, seconds * 1000, () => {
          if (sessions.has(session.info.session_id) && session.info.state === 'connected') {
            output(session, `\r\n${text}\r\n${session.shellPrompt}${session.line}`);
          }
        });
        return;
      }
      case 'drop':
        if (session.info.kind === 'ssh') {
          drop(session);
          return;
        }
        lines('drop: command not found');
        break;
      default:
        lines(`${command}: command not found`);
    }
    printPrompt(session);
  };

  const shellInput = (session: MockSession, data: string) => {
    for (const char of data) {
      if (session.info.state !== 'connected') {
        return;
      }
      if (session.escape !== null) {
        session.escape += char;
        const body = session.escape;
        const complete = body.length >= 2 && (body[1] === '[' ? /[\x40-\x7e]$/.test(body) && body.length > 2 : true);
        if (complete) {
          session.escape = null;
        }
        continue;
      }
      if (char === '\x1b') {
        session.escape = char;
        continue;
      }
      if (session.mode === 'secret') {
        if (char === '\r' || char === '\n') {
          session.mode = 'normal';
          output(session, '\r\nok\r\n');
          printPrompt(session);
        }
        continue;
      }
      if (session.mode === 'alt') {
        if (char === 'q') {
          session.mode = 'normal';
          output(session, '\x1b[?1049l');
          printPrompt(session);
        }
        continue;
      }
      const code = char.codePointAt(0) ?? 0;
      if (char === '\r' || char === '\n') {
        output(session, '\r\n');
        const line = session.line;
        session.line = '';
        run(session, line);
      } else if (code === 0x7f || code === 0x08) {
        if (session.line !== '') {
          session.line = [...session.line].slice(0, -1).join('');
          output(session, '\x08 \x08');
        }
      } else if (code === 0x03) {
        session.line = '';
        output(session, '^C\r\n');
        printPrompt(session);
      } else if (code === 0x04) {
        if (session.line === '') {
          endSession(session, 0);
          return;
        }
      } else if (code >= 0x20) {
        session.line += char;
        output(session, char);
      }
    }
  };

  const startShell = (session: MockSession, motd: boolean) => {
    session.line = '';
    session.mode = 'normal';
    session.escape = null;
    if (motd) {
      output(session, MOTD);
    }
    printPrompt(session);
  };

  // --- Connect flow --------------------------------------------------------------

  const step = (session: MockSession, token: number, next: ConnectStep) => {
    if (session.token === token) {
      update(session, { state: 'connecting', step: next });
    }
  };

  const fail = (reason: SshReason): never => {
    throw Object.assign(new Error('ssh'), { sessionError: { code: 'ssh_error', reason } as SessionError });
  };

  const connectFlow = async (session: MockSession, token: number): Promise<void> => {
    const address = session.info.address ?? '';
    const port = session.info.port ?? 22;
    step(session, token, 'resolving');
    await delay(session, token);
    if (refusing) {
      fail('refused');
    }
    step(session, token, 'connecting');
    await delay(session, token);
    step(session, token, 'host_key');
    const key = hostKey(address, port);
    const keyInfo: HostKeyInfo = { address: key.address, port: key.port, key_type: key.key_type, fingerprint: key.fingerprint };
    const check = session.onceKey === key.public_key ? { kind: 'match' as const } : checkKnownHost(address, port, key.public_key);
    if (check.kind === 'revoked') {
      throw Object.assign(new Error('revoked'), { sessionError: { code: 'host_key_revoked' } as SessionError, hostKey: keyInfo });
    }
    if (check.kind === 'unknown') {
      const reply = await ask(session, token, { kind: 'host_key_unknown', key: keyInfo });
      session.pending = null;
      if (reply.kind !== 'host_key' || reply.decision === 'cancel' || reply.decision === 'replace') {
        fail('host_key_rejected');
      } else if (reply.decision === 'accept_save') {
        saveKnownHost(address, port, key, false);
      } else {
        session.onceKey = key.public_key;
      }
    } else if (check.kind === 'changed') {
      const reply = await ask(session, token, {
        kind: 'host_key_changed',
        key: keyInfo,
        old_key_type: check.old.key_type,
        old_fingerprint: mockFingerprint(check.old.public_key),
      });
      session.pending = null;
      if (reply.kind !== 'host_key' || reply.decision !== 'replace') {
        fail('host_key_rejected');
      }
      saveKnownHost(address, port, key, true);
    }
    promptAnswered(session);
    step(session, token, 'authenticating');
    await delay(session, token);
    let username = session.username ?? session.keptUsername ?? undefined;
    if (username === undefined || username === '') {
      const reply = await ask(session, token, { kind: 'username' });
      session.pending = null;
      if (reply.kind !== 'text' || reply.value.trim() === '') {
        fail('auth_failed');
      }
      username = (reply as { value: string }).value.trim();
      session.keptUsername = username;
      update(session, { prompt: undefined, username });
    }
    let password = session.savedPassword ?? session.keptPassword ?? undefined;
    let error = false;
    for (let round = 0; password === undefined || password === '' || password.startsWith('wrong'); round += 1) {
      if (round >= 3) {
        fail('auth_failed');
      }
      const reply = await ask(session, token, {
        kind: 'password',
        username,
        error: error || (password?.startsWith('wrong') ?? false),
        can_save: session.host !== null,
      });
      session.pending = null;
      if (reply.kind !== 'text') {
        fail('auth_failed');
      }
      const value = (reply as { value: string; save?: boolean }).value;
      error = value === '' || value.startsWith('wrong');
      if (!error) {
        session.keptPassword = value;
        if ((reply as { save?: boolean }).save === true && session.host !== null) {
          const record = ctx.records.get(session.host.record_id);
          if (record !== undefined) {
            record.secrets['ssh.password'] = value;
            session.savedPassword = value;
            ctx.recordsChanged([record.record_id], [CLASS_HOSTS]);
          }
        }
      }
      password = error ? undefined : value;
      promptAnswered(session);
    }
    if (username === 'otp') {
      const reply = await ask(session, token, {
        kind: 'keyboard_interactive',
        name: 'mock-server',
        instruction: 'Enter the code from your authenticator.',
        prompt: 'Verification code: ',
        echo: false,
        index: 0,
        count: 1,
        error: false,
      });
      session.pending = null;
      if (reply.kind !== 'text' || reply.value === '') {
        fail('auth_failed');
      }
      promptAnswered(session);
    }
    step(session, token, 'opening_session');
    await delay(session, token);
  };

  const recordConnection = (session: MockSession) => {
    const info = session.info;
    const address = info.address ?? '';
    const port = info.port ?? 22;
    const at = new Date(ctx.now()).toISOString();
    const existing = ctx.connections.findIndex((entry) =>
      session.host !== null
        ? entry.host?.record_id === session.host.record_id
        : entry.host === undefined && entry.address === address && entry.port === port && entry.username === info.username,
    );
    const row = {
      ...(session.host === null ? {} : { host: { ...session.host } }),
      title: info.title,
      address,
      port,
      ...(info.username === undefined ? {} : { username: info.username }),
      last_at: at,
    };
    if (existing >= 0) {
      ctx.connections.splice(existing, 1);
    }
    ctx.connections.unshift(row);
    ctx.recordsChanged([ctx.newId()], [CLASS_CONNECTION_LOG]);
  };

  const backoff = (attempt: number) => BACKOFF_MS[attempt - 1] ?? BACKOFF_STEADY_MS;

  const scheduleReconnect = (session: MockSession, attempt: number) => {
    if (session.reconnectTimer !== null) {
      clearTimeout(session.reconnectTimer);
      session.reconnectTimer = null;
    }
    if (ctx.lockState() !== 'unlocked') {
      update(session, { reconnect: { attempt, waiting_for_unlock: true, in_progress: false } });
      return;
    }
    const wait = backoff(attempt);
    update(session, { reconnect: { attempt, next_in_ms: wait, waiting_for_unlock: false, in_progress: false } });
    session.reconnectTimer = setTimeout(() => {
      session.reconnectTimer = null;
      if (sessions.has(session.info.session_id) && session.info.state === 'dropped') {
        startAttempt(session, 'reconnect', attempt);
      }
    }, wait);
  };

  const startAttempt = (session: MockSession, kind: AttemptKind, attempt = 1) => {
    session.token += 1;
    const token = session.token;
    clearPrompt(session);
    if (session.reconnectTimer !== null) {
      clearTimeout(session.reconnectTimer);
      session.reconnectTimer = null;
    }
    update(session, {
      state: 'connecting',
      step: 'resolving',
      error: undefined,
      exit_status: undefined,
      host_key: undefined,
      prompt: undefined,
      reconnect: kind === 'reconnect' ? { attempt, waiting_for_unlock: false, in_progress: true } : undefined,
    });
    connectFlow(session, token).then(
      () => {
        if (session.token !== token || !sessions.has(session.info.session_id)) {
          return;
        }
        update(session, { state: 'connected', step: undefined, prompt: undefined, reconnect: undefined, error: undefined });
        session.shellPrompt = `${session.info.username ?? 'user'}@shellpp-test:~$ `;
        recordConnection(session);
        startShell(session, true);
      },
      (reason: unknown) => {
        if (session.token !== token || !sessions.has(session.info.session_id)) {
          return;
        }
        session.pending = null;
        const detail = reason as { sessionError?: SessionError; hostKey?: HostKeyInfo };
        const error: SessionError = detail.sessionError ?? { code: 'cancelled' };
        if (kind === 'reconnect' && error.code !== 'host_key_revoked') {
          update(session, { state: 'dropped', step: undefined, prompt: undefined, error });
          if (boolSetting('terminal.autoreconnect', true)) {
            scheduleReconnect(session, attempt + 1);
          } else {
            update(session, { reconnect: undefined });
          }
          return;
        }
        update(session, {
          state: 'failed',
          step: undefined,
          prompt: undefined,
          reconnect: undefined,
          error,
          host_key: detail.hostKey,
        });
      },
    );
  };

  function drop(session: MockSession): void {
    session.token += 1;
    clearPrompt(session);
    session.line = '';
    session.mode = 'normal';
    update(session, {
      state: 'dropped',
      step: undefined,
      prompt: undefined,
      error: { code: 'ssh_error', reason: 'disconnected' },
    });
    if (boolSetting('terminal.autoreconnect', true)) {
      scheduleReconnect(session, 1);
    }
  }

  // --- Sessions --------------------------------------------------------------------

  const newSession = (info: Omit<SessionInfo, 'rev' | 'window' | 'session_id'>, extra: Partial<MockSession>): MockSession => {
    const session: MockSession = {
      info: clean({ ...info, session_id: crypto.randomUUID(), rev: 1, window: ctx.windowLabel } as SessionInfo),
      cols: 80,
      rows: 24,
      replay: [],
      replaySize: 0,
      channelId: null,
      input: '',
      line: '',
      mode: 'normal',
      escape: null,
      shellPrompt: '$ ',
      lineInjected: false,
      host: null,
      username: undefined,
      savedPassword: undefined,
      keptPassword: null,
      keptUsername: null,
      onceKey: null,
      pending: null,
      token: 0,
      timers: new Set(),
      reconnectTimer: null,
      ...extra,
    };
    sessions.set(session.info.session_id, session);
    return session;
  };

  const resolveTarget = (target: SessionTarget) => {
    const recordHistory = boolSetting('terminal.record_history', true);
    if (target.kind === 'host') {
      if (target.vault_id !== ctx.vaultId) {
        throw new IpcError('not_found');
      }
      const view = ctx.records.getView(target.record_id);
      if (view.type !== 'host') {
        throw new IpcError('not_found');
      }
      const data = view.data as HostData;
      const port = data.ssh?.port ?? view.inherited.port?.value ?? 22;
      const username = data.ssh?.username ?? view.inherited.username?.value;
      const label = typeof data.label === 'string' && data.label.trim() !== '' ? data.label : data.address;
      const savedPassword = view.secrets_set.includes('ssh.password') ? ctx.records.secret(target.record_id, 'ssh.password') : undefined;
      const record = data.record_history;
      return {
        title: label,
        address: data.address,
        port,
        username,
        host: { vault_id: target.vault_id, record_id: target.record_id },
        savedPassword,
        theme: data.terminal_theme ?? defaultTheme(),
        backspace: (data.backspace ?? 'default') as Backspace,
        recordHistory: record === 'on' ? true : record === 'off' ? false : recordHistory,
      };
    }
    const address = target.address.trim();
    if (address === '') {
      throw validation('address', 'required');
    }
    if (address.length > 255 || /\s/.test(address)) {
      throw validation('address', 'invalid');
    }
    const port = target.port ?? 22;
    if (!Number.isInteger(port) || port < 1 || port > 65_535) {
      throw validation('port', 'out_of_range');
    }
    const username = target.username?.trim();
    if (username !== undefined && (username === '' || username.length > 256)) {
      throw validation('username', 'invalid');
    }
    return {
      title: address,
      address,
      port,
      username,
      host: null,
      savedPassword: undefined,
      theme: defaultTheme(),
      backspace: 'default' as Backspace,
      recordHistory,
    };
  };

  const handlers: SessionHandlers = {
    session_open: ({ target, cols, rows }) => {
      validateSize(cols, rows);
      const resolved = resolveTarget(target);
      const session = newSession(
        {
          kind: 'ssh',
          state: 'connecting',
          step: 'resolving',
          title: resolved.title,
          target: structuredClone(target),
          address: resolved.address,
          port: resolved.port,
          username: resolved.username,
          terminal_theme: resolved.theme,
          backspace: resolved.backspace,
          record_history: resolved.recordHistory,
        },
        {
          cols,
          rows,
          host: resolved.host,
          username: resolved.username,
          savedPassword: resolved.savedPassword,
          shellPrompt: `${resolved.username ?? 'user'}@shellpp-test:~$ `,
        },
      );
      const info = clean(session.info);
      later(session, STEP_DELAY_MS, () => {
        if (sessions.has(session.info.session_id) && session.info.state === 'connecting' && session.token === 0) {
          startAttempt(session, 'first');
        }
      });
      return info;
    },
    session_attach: ({ session_id, on_output }) => {
      const session = find(session_id);
      const channelId = (on_output as unknown as { id?: unknown }).id;
      if (typeof channelId !== 'number') {
        throw validation('on_output', 'invalid');
      }
      session.channelId = channelId;
      const replay = session.replay.join('');
      if (replay !== '') {
        deliver(channelId, replay);
      }
      return clean(session.info);
    },
    session_write: ({ session_id, data, history }) => {
      const session = find(session_id);
      if (encoder.encode(data).length > MAX_WRITE_BYTES) {
        throw validation('data', 'too_long');
      }
      if (session.info.state !== 'connected') {
        return null;
      }
      session.input += data;
      if (history !== undefined && session.info.record_history && !session.lineInjected && mockHistoryAccepts(history)) {
        const id = ctx.newId();
        ctx.history.push({
          record_id: id,
          ...(session.host === null ? {} : { host: { ...session.host } }),
          command: history.command,
          executed_at: new Date(ctx.now()).toISOString(),
        });
        ctx.recordsChanged([id], [CLASS_HISTORY]);
      }
      if (data.includes('\r')) {
        session.lineInjected = false;
      }
      shellInput(session, data);
      return null;
    },
    session_inject: ({ session_id, data }) => {
      const session = find(session_id);
      if (encoder.encode(data).length > MAX_WRITE_BYTES) {
        throw validation('data', 'too_long');
      }
      if (session.info.state !== 'connected') {
        return null;
      }
      session.input += data;
      session.lineInjected = !data.endsWith('\r');
      shellInput(session, data);
      return null;
    },
    session_resize: ({ session_id, cols, rows }) => {
      const session = find(session_id);
      validateSize(cols, rows);
      session.cols = cols;
      session.rows = rows;
      return null;
    },
    session_prompt_reply: ({ session_id, prompt_id, reply }) => {
      const session = find(session_id);
      const pending = session.pending;
      const prompt = session.info.prompt;
      if (pending === null || prompt === undefined || pending.promptId !== prompt_id) {
        throw new IpcError('not_found');
      }
      const hostKeyPrompt = prompt.kind === 'host_key_unknown' || prompt.kind === 'host_key_changed';
      if (reply.kind !== 'cancel' && (reply.kind === 'host_key') !== hostKeyPrompt) {
        throw validation('reply', 'invalid');
      }
      session.pending = null;
      if (reply.kind === 'cancel') {
        const reconnecting = session.info.reconnect !== undefined;
        session.token += 1;
        pending.reject(new Aborted());
        update(
          session,
          reconnecting
            ? { state: 'dropped', step: undefined, prompt: undefined, reconnect: undefined, error: { code: 'cancelled' } }
            : { state: 'failed', step: undefined, prompt: undefined, error: { code: 'cancelled' } },
        );
        return null;
      }
      pending.resolve(reply);
      return null;
    },
    session_cancel: ({ session_id }) => {
      const session = find(session_id);
      const state = session.info.state;
      if (state === 'connecting') {
        const reconnecting = session.info.reconnect !== undefined;
        session.token += 1;
        clearPrompt(session);
        update(
          session,
          reconnecting
            ? { state: 'dropped', step: undefined, prompt: undefined, reconnect: undefined }
            : { state: 'failed', step: undefined, prompt: undefined, error: { code: 'cancelled' } },
        );
      } else if (state === 'dropped' && session.info.reconnect !== undefined) {
        if (session.reconnectTimer !== null) {
          clearTimeout(session.reconnectTimer);
          session.reconnectTimer = null;
        }
        update(session, { reconnect: undefined });
      }
      return null;
    },
    session_close: ({ session_id }) => {
      const session = find(session_id);
      session.token += 1;
      clearPrompt(session);
      update(session, { state: 'closed', prompt: undefined, step: undefined, reconnect: undefined });
      return null;
    },
    session_reconnect: ({ session_id }) => {
      const session = find(session_id);
      const state = session.info.state;
      if (state === 'connecting' || state === 'connected') {
        throw new IpcError('wrong_state');
      }
      if (session.info.kind === 'local') {
        session.token += 1;
        update(session, { state: 'connected', exit_status: undefined, error: undefined });
        startShell(session, false);
        return clean(session.info);
      }
      startAttempt(session, state === 'dropped' ? 'reconnect' : 'retry', 1);
      return clean(session.info);
    },
    session_ack: ({ session_id, bytes }) => {
      find(session_id);
      if (!Number.isSafeInteger(bytes) || bytes < 0) {
        throw validation('bytes', 'invalid');
      }
      return null;
    },
    local_shells: () => {
      const list = shells();
      return { shells: list.map((shell) => ({ ...shell })), default_id: list[0]?.id };
    },
    local_open: ({ shell_id, cols, rows }) => {
      validateSize(cols, rows);
      const list = shells();
      const configured = settings()['terminal.local_shell'];
      const wanted = shell_id ?? (typeof configured === 'string' ? configured : null) ?? list[0]?.id;
      const shell = list.find((candidate) => candidate.id === wanted);
      if (shell === undefined) {
        if (shell_id !== undefined && shell_id !== null) {
          throw validation('shell_id', 'invalid');
        }
        throw new IpcError('unavailable');
      }
      const session = newSession(
        {
          kind: 'local',
          state: 'connected',
          title: shell.label,
          shell_id: shell.id,
          terminal_theme: defaultTheme(),
          backspace: 'default',
          record_history: boolSetting('terminal.record_history', true),
        },
        {
          cols,
          rows,
          shellPrompt: shell.id === 'powershell' ? 'PS C:\\mock> ' : shell.id === 'cmd' ? 'C:\\mock>' : 'mock % ',
        },
      );
      startShell(session, false);
      return clean(session.info);
    },
    history_list: ({ host, text, limit }) => {
      const max = Math.min(5000, Math.max(1, limit ?? 500));
      const needle = text?.trim().toLowerCase() ?? '';
      return ctx.history
        .filter((entry) =>
          host === undefined || host === null ? entry.host === undefined : entry.host?.record_id === host.record_id,
        )
        .filter((entry) => needle === '' || entry.command.toLowerCase().includes(needle))
        .sort((a, b) => b.executed_at.localeCompare(a.executed_at) || b.record_id.localeCompare(a.record_id))
        .slice(0, max)
        .map((entry) => ({ vault_id: ctx.vaultId, record_id: entry.record_id, command: entry.command, executed_at: entry.executed_at }));
    },
    history_delete: ({ vault_id, record_ids }) => {
      if (vault_id !== ctx.vaultId || !record_ids.every((id) => ctx.history.some((entry) => entry.record_id === id))) {
        throw new IpcError('not_found');
      }
      for (const id of record_ids) {
        const index = ctx.history.findIndex((entry) => entry.record_id === id);
        if (index >= 0) {
          ctx.history.splice(index, 1);
        }
      }
      ctx.recordsChanged(record_ids, [CLASS_HISTORY]);
      return null;
    },
    history_clear: ({ host }) => {
      const removed: string[] = [];
      for (let index = ctx.history.length - 1; index >= 0; index -= 1) {
        const entry = ctx.history[index];
        if (entry === undefined) {
          continue;
        }
        const matches = host === undefined || host === null ? entry.host === undefined : entry.host?.record_id === host.record_id;
        if (matches) {
          removed.push(entry.record_id);
          ctx.history.splice(index, 1);
        }
      }
      ctx.recordsChanged(removed, [CLASS_HISTORY]);
      return { removed: removed.length };
    },
  };

  return {
    handlers,
    control: {
      list: () => [...sessions.values()].map((session) => clean(session.info)),
      drop: (sessionId) => {
        const session = sessions.get(sessionId);
        if (session !== undefined && session.info.state === 'connected' && session.info.kind === 'ssh') {
          drop(session);
        }
      },
      input: (sessionId) => sessions.get(sessionId)?.input ?? '',
      output: (sessionId, text) => {
        const session = sessions.get(sessionId);
        if (session !== undefined) {
          output(session, text);
        }
      },
      setRefusing: (value) => {
        refusing = value;
      },
      rotateHostKey: (address, port = 22) => {
        const key = `${address.toLowerCase()}:${port}`;
        rotations.set(key, (rotations.get(key) ?? 0) + 1);
      },
    },
    snapshot: () => [...sessions.values()].map((session) => clean(session.info)),
    activeCount: () => sessions.size,
    onLockChanged: (locked) => {
      for (const session of sessions.values()) {
        if (locked) {
          // Kept credentials are wiped; pending prompts are cancelled (decision 27).
          session.keptPassword = null;
          if (session.pending !== null) {
            const reconnecting = session.info.reconnect !== undefined;
            session.token += 1;
            clearPrompt(session);
            if (reconnecting) {
              update(session, { state: 'dropped', step: undefined, prompt: undefined, error: { code: 'cancelled' } });
            } else {
              update(session, { state: 'failed', step: undefined, prompt: undefined, error: { code: 'cancelled' } });
              continue;
            }
          }
          if (session.info.state === 'dropped' && session.info.reconnect !== undefined) {
            if (session.reconnectTimer !== null) {
              clearTimeout(session.reconnectTimer);
              session.reconnectTimer = null;
            }
            update(session, {
              reconnect: { attempt: session.info.reconnect.attempt, waiting_for_unlock: true, in_progress: false },
            });
          }
        } else if (session.info.state === 'dropped' && session.info.reconnect?.waiting_for_unlock === true) {
          startAttempt(session, 'reconnect', session.info.reconnect.attempt);
        }
      }
    },
  };
}
