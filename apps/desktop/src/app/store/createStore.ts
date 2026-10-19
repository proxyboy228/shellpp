// A minimal external store for React's `useSyncExternalStore` (no state
// library: the plan allows no new dependency).
//
// Owner: W4.

import { useCallback, useSyncExternalStore } from 'react';

export interface Store<T> {
  get(): T;
  /** Replaces the value (or derives it from the previous one) and notifies subscribers when it changed. */
  set(next: T | ((previous: T) => T)): void;
  subscribe(listener: () => void): () => void;
}

export function createStore<T>(initial: T): Store<T> {
  let value = initial;
  const listeners = new Set<() => void>();
  return {
    get: () => value,
    set(next) {
      const resolved = typeof next === 'function' ? (next as (previous: T) => T)(value) : next;
      if (Object.is(resolved, value)) {
        return;
      }
      value = resolved;
      for (const listener of [...listeners]) {
        listener();
      }
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}

/** The whole value of a store. */
export function useStore<T>(store: Store<T>): T {
  return useSyncExternalStore(store.subscribe, store.get, store.get);
}

/**
 * A derived value. The selector must return a stable value (a primitive or a
 * part of the state), not a new object on every call.
 */
export function useStoreSelector<T, S>(store: Store<T>, selector: (value: T) => S): S {
  const getSnapshot = useCallback(() => selector(store.get()), [store, selector]);
  return useSyncExternalStore(store.subscribe, getSnapshot, getSnapshot);
}
