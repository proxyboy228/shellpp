// A view setting with an optimistic local override (Hosts and Known Hosts
// screens: layout and sort): the new value shows at once and the stored value
// takes over when `settings` reports a change.

import { useCallback, useState } from 'react';
import { settingsSet, type SettingsValues } from '../../ipc';
import { showErrorToast } from './actions';

export function useSettingValue<T extends string>(
  settings: SettingsValues,
  key: string,
  values: readonly T[],
  fallback: T,
): [T, (value: T) => void] {
  const raw = settings[key];
  const stored = values.includes(raw as T) ? (raw as T) : fallback;
  const [pending, setPending] = useState<{ value: T; stored: T } | null>(null);
  // Any change of the stored value (this window's write or another window's)
  // ends the override, so a later change back to the old stored value is not
  // hidden behind it (UI 1.4: changes appear in all windows).
  if (pending !== null && pending.stored !== stored) {
    setPending(null);
  }
  const value = pending !== null && pending.stored === stored ? pending.value : stored;
  const set = useCallback(
    (next: T) => {
      setPending({ value: next, stored });
      settingsSet({ values: { [key]: next } }).catch((reason: unknown) => {
        setPending(null);
        showErrorToast(reason);
      });
    },
    [key, stored],
  );
  return [value, set];
}
