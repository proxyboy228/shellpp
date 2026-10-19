// A minimal backend for the connect UI tests: records every command and
// answers from the given handlers (others resolve with null). Events are
// emitted by the test with `emit`.
//
// Owner: S7.

import { IpcError, type Backend } from '../../../ipc';
import type { CommandInput, CommandName, CommandOutput, EventMap, EventName, IpcChannel } from '../../../ipc';

export interface RecordedCall {
  command: CommandName;
  input: unknown;
}

export type RecordingHandlers = { [C in CommandName]?: (input: CommandInput<C>) => CommandOutput<C> | Promise<CommandOutput<C>> };

export interface RecordingBackend extends Backend {
  readonly calls: RecordedCall[];
  callsOf(command: CommandName): unknown[];
  emit<E extends EventName>(event: E, payload: EventMap[E]): void;
  /** Makes the next call of `command` fail with `error`. */
  failNext(command: CommandName, error: IpcError): void;
}

export function createRecordingBackend(handlers: RecordingHandlers = {}): RecordingBackend {
  const calls: RecordedCall[] = [];
  const listeners = new Map<EventName, Set<(payload: unknown) => void>>();
  const failures = new Map<CommandName, IpcError>();
  return {
    kind: 'mock',
    calls,
    callsOf: (command) => calls.filter((call) => call.command === command).map((call) => call.input),
    invoke<C extends CommandName>(command: C, input: CommandInput<C>): Promise<CommandOutput<C>> {
      calls.push({ command, input: input === undefined ? null : structuredClone(input) });
      const failure = failures.get(command);
      if (failure !== undefined) {
        failures.delete(command);
        return Promise.reject(failure);
      }
      const handler = handlers[command] as ((value: CommandInput<C>) => CommandOutput<C> | Promise<CommandOutput<C>>) | undefined;
      if (handler === undefined) {
        return Promise.resolve(null as CommandOutput<C>);
      }
      try {
        return Promise.resolve(handler(input));
      } catch (reason) {
        return Promise.reject(reason);
      }
    },
    listen(event, handler) {
      const set = listeners.get(event) ?? new Set();
      const wrapped = handler as (payload: unknown) => void;
      set.add(wrapped);
      listeners.set(event, set);
      return Promise.resolve(() => {
        set.delete(wrapped);
      });
    },
    emit(event, payload) {
      for (const handler of [...(listeners.get(event) ?? [])]) {
        handler(payload);
      }
    },
    failNext(command, error) {
      failures.set(command, error);
    },
    createChannel: <T,>() => ({ __ipcChannel: true as const, close: () => undefined }) as IpcChannel<T> & { close(): void },
    window: {
      label: 'w-1',
      minimize: () => Promise.resolve(),
      toggleMaximize: () => Promise.resolve(),
      isMaximized: () => Promise.resolve(false),
      close: () => Promise.resolve(),
      startDragging: () => Promise.resolve(),
      setFullscreen: () => Promise.resolve(),
      onResized: () => Promise.resolve(() => undefined),
    },
  };
}
