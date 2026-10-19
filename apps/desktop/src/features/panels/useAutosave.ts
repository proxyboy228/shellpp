// Autosave for the record panels (UI 1.3, 3.2; architecture 2.5): a new
// record stays a draft until its required fields are valid; after that every
// change is saved 500 ms after the last edit. One save runs at a time; edits
// made while it runs are saved right after it. Unmounting the panel (close,
// switch to another record) saves pending changes at once; a draft is
// discarded.

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';

/** Value of the panel's `data-save-state` attribute. */
export type SaveState = 'draft' | 'pending' | 'saving' | 'saved' | 'error';

export const AUTOSAVE_DELAY_MS = 500;

export interface AutosaveOptions {
  /** Increases on every edit; each increase restarts the delay. */
  changeKey: number;
  /** The form differs from the stored record (or holds a secret change). */
  dirty: boolean;
  /** False while a required field of a new record is not valid (the draft rule). */
  canSave: boolean;
  /** The record has not been created yet. */
  isNew: boolean;
  /**
   * Saves the current form. Resolves `true` on success and `false` when the
   * core refused it (the panel shows the reasons); a rejection counts as a
   * failure too. It must read the latest form and record id itself.
   */
  save: () => Promise<boolean>;
  delayMs?: number;
}

export interface Autosave {
  state: SaveState;
  /** Saves pending changes now (no effect for a draft or without changes). */
  flush: () => void;
}

export function useAutosave(options: AutosaveOptions): Autosave {
  const { changeKey, dirty, canSave, isNew, delayMs = AUTOSAVE_DELAY_MS } = options;
  const [saving, setSaving] = useState(false);
  // The change key of the last successful and of the last failed save.
  const [savedKey, setSavedKey] = useState<number | null>(null);
  const [failedKey, setFailedKey] = useState<number | null>(null);

  const optionsRef = useRef(options);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const inFlightRef = useRef(false);
  const rerunRef = useRef(false);
  const mountedRef = useRef(false);

  useLayoutEffect(() => {
    optionsRef.current = options;
  });

  useLayoutEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const run = useCallback(function runSave(): void {
    if (inFlightRef.current) {
      rerunRef.current = true;
      return;
    }
    const current = optionsRef.current;
    if (!current.dirty || !current.canSave) {
      return;
    }
    const key = current.changeKey;
    inFlightRef.current = true;
    setSaving(true);
    void current
      .save()
      .catch(() => false)
      .then((succeeded) => {
        inFlightRef.current = false;
        setSaving(false);
        if (succeeded) {
          setSavedKey(key);
          setFailedKey(null);
        } else {
          setFailedKey(key);
        }
        if (rerunRef.current) {
          rerunRef.current = false;
          runSave();
        }
      });
  }, []);

  const shouldSchedule = dirty && canSave && savedKey !== changeKey && failedKey !== changeKey;

  useEffect(() => {
    if (!shouldSchedule) {
      return undefined;
    }
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      run();
    }, delayMs);
    return () => {
      if (timerRef.current !== null) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
        // Unmounting with a pending change: save it now.
        if (!mountedRef.current) {
          run();
        }
      }
    };
  }, [shouldSchedule, changeKey, delayMs, run]);

  const flush = useCallback(() => {
    if (timerRef.current !== null) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
      run();
    }
  }, [run]);

  let state: SaveState;
  if (isNew && !canSave) {
    state = 'draft';
  } else if (saving) {
    state = 'saving';
  } else if (failedKey === changeKey) {
    state = 'error';
  } else if (shouldSchedule) {
    state = 'pending';
  } else {
    state = 'saved';
  }
  return { state, flush };
}
