// Toasts: short messages at the bottom of the window. Used for notification
// toasts (UI 1.5, 5 seconds) and for the undo toast of a removal (UI 2.4).
// Any feature can call `showToast`; the app shell renders `<ToastRegion/>`.
//
// Owner: W4. The API below is the W4/W5 contract.

import type { NotificationLevel } from '../ipc';

export interface ToastAction {
  label: string;
  onAction: () => void;
  /** `data-testid` of the action button. */
  testId: string;
}

export interface ToastInput {
  message: string;
  level?: NotificationLevel;
  action?: ToastAction;
  /** Default 5000 ms. */
  durationMs?: number;
  /** `data-testid` of the toast element. */
  testId: string;
  /** Rendered as `data-id` (for example the notification id of a notification toast). */
  dataId?: string | number;
}

export interface Toast extends Required<Omit<ToastInput, 'action' | 'dataId'>> {
  id: number;
  action?: ToastAction;
  dataId?: string | number;
}

type Listener = (toasts: readonly Toast[]) => void;

let nextId = 1;
let toasts: readonly Toast[] = [];
const listeners = new Set<Listener>();
const timers = new Map<number, ReturnType<typeof setTimeout>>();

function publish(): void {
  for (const listener of listeners) {
    listener(toasts);
  }
}

/** Removes a toast (also called when its time is up). */
export function dismissToast(id: number): void {
  const timer = timers.get(id);
  if (timer !== undefined) {
    clearTimeout(timer);
    timers.delete(id);
  }
  const remaining = toasts.filter((toast) => toast.id !== id);
  if (remaining.length !== toasts.length) {
    toasts = remaining;
    publish();
  }
}

/** Shows a toast and returns a function that dismisses it early. */
export function showToast(input: ToastInput): () => void {
  const toast: Toast = {
    id: nextId++,
    message: input.message,
    level: input.level ?? 'info',
    durationMs: input.durationMs ?? 5000,
    testId: input.testId,
    action: input.action,
    dataId: input.dataId,
  };
  toasts = [...toasts, toast];
  timers.set(
    toast.id,
    setTimeout(() => dismissToast(toast.id), toast.durationMs),
  );
  publish();
  return () => dismissToast(toast.id);
}

/** The current toast list, oldest first. */
export function getToasts(): readonly Toast[] {
  return toasts;
}

/** Subscribes to the toast list (used by `<ToastRegion/>`). */
export function subscribeToasts(listener: Listener): () => void {
  listeners.add(listener);
  listener(toasts);
  return () => {
    listeners.delete(listener);
  };
}

/** Removes every toast (on lock, and between tests). */
export function clearToasts(): void {
  for (const timer of timers.values()) {
    clearTimeout(timer);
  }
  timers.clear();
  toasts = [];
  publish();
}
