// Form state of a record panel: what the user typed, the stored record it is
// based on, validation, refusals by the core and autosave (UI 1.3).

import { useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { toIpcError, type Uuid } from '../../ipc';
import { t } from '../../i18n';
import { showToast } from '../../app/toast';
import {
  activeServerErrors,
  buildData,
  formFromData,
  keyForField,
  syncForm,
  toServerErrors,
  validateForm,
  type Data,
  type FieldTable,
  type ServerError,
} from './form';
import type { PathError } from './validators';
import { useAutosave, type SaveState } from './useAutosave';

/** The parts of a record view the form needs. */
export interface FormRecord {
  record_id: Uuid;
  revision: number;
  data: Data;
}

export interface RecordFormOptions<F, V extends FormRecord> {
  table: FieldTable<F>;
  /** The member that must be valid before a new record is created. */
  requiredKey: keyof F;
  /** Record id at mount; `null` for a new record (a draft). */
  recordId: Uuid | null;
  /** The record as the screen's lists currently show it. */
  view: V | undefined;
  /** Data of a new record before its first save (read once). */
  draftData: Data;
  /** Writes `data` (create when `recordId` is null) and resolves with the stored view. */
  persist: (recordId: Uuid | null, data: Record<string, unknown>) => Promise<V>;
  /** Unsaved state outside the form, such as a typed password. */
  extraDirty?: boolean;
  /** After every successful save; `created` for the first save of a draft. */
  onSaved?: (view: V, created: boolean) => void;
  /** The record no longer exists (removed elsewhere). */
  onMissing?: () => void;
}

export interface RecordForm<F, V extends FormRecord> {
  recordId: Uuid | null;
  form: F;
  /** The stored record the form is based on (null for a draft). */
  base: V | null;
  /** Client validation errors and refusals by the core, with full paths. */
  errors: PathError[];
  saveState: SaveState;
  update: <K extends keyof F>(key: K, value: F[K]) => void;
  /** Marks a change outside the form (restarts the autosave delay). */
  touch: () => void;
  flush: () => void;
}

/** The newer of the list's view and the last saved view. */
function newest<V extends FormRecord>(view: V | undefined, saved: V | null, recordId: Uuid | null): V | null {
  const fromList = view !== undefined && view.record_id === recordId ? view : null;
  if (fromList === null) {
    return saved !== null && saved.record_id === recordId ? saved : null;
  }
  if (saved === null || saved.record_id !== recordId || fromList.revision >= saved.revision) {
    return fromList;
  }
  return saved;
}

const MAX_ATTEMPTS = 3;

export function useRecordForm<F, V extends FormRecord>(options: RecordFormOptions<F, V>): RecordForm<F, V> {
  const { table, requiredKey } = options;
  const [recordId, setRecordId] = useState<Uuid | null>(options.recordId);
  const [savedView, setSavedView] = useState<V | null>(null);
  const [draftData] = useState<Data>(options.draftData);
  const base = newest(options.view, savedView, recordId);
  const baseData = base?.data ?? draftData;

  const [form, setForm] = useState<F>(() => formFromData(table, baseData));
  const [syncedData, setSyncedData] = useState<Data>(baseData);
  if (syncedData !== baseData) {
    // A newer stored record: fields that were in sync take its values.
    setSyncedData(baseData);
    setForm((current) => syncForm(table, current, syncedData, baseData));
  }

  const [changeKey, setChangeKey] = useState(0);
  const [serverErrors, setServerErrors] = useState<ServerError<F>[]>([]);
  // Fields the user edited: an empty required field is reported only after that.
  const [touched, setTouched] = useState<ReadonlySet<keyof F>>(() => new Set());

  const validation = useMemo(() => validateForm(table, form), [table, form]);
  const active = useMemo(() => activeServerErrors(serverErrors, form), [serverErrors, form]);
  const skip = useMemo(() => new Set(active.map((entry) => entry.key)), [active]);
  const built = useMemo(() => buildData(table, baseData, validation, skip), [table, baseData, validation, skip]);

  const isNew = recordId === null;
  const requiredValid = !validation.invalid.has(requiredKey) && !skip.has(requiredKey);
  const canSave = !isNew || requiredValid;
  const dirty = built.changed || options.extraDirty === true;

  const latestRef = useRef({ options, recordId, baseData, form, serverErrors });
  useLayoutEffect(() => {
    latestRef.current = { options, recordId, baseData, form, serverErrors };
  });

  const save = useCallback(async (): Promise<boolean> => {
    const latest = latestRef.current;
    let refused = activeServerErrors(latest.serverErrors, latest.form);
    for (let attempt = 0; attempt < MAX_ATTEMPTS; attempt += 1) {
      const skipKeys = new Set(refused.map((entry) => entry.key));
      const currentValidation = validateForm(table, latest.form);
      const { data, changed } = buildData(table, latest.baseData, currentValidation, skipKeys);
      if (!changed && latest.options.extraDirty !== true && latest.recordId !== null) {
        return true;
      }
      if (latest.recordId === null && (currentValidation.invalid.has(requiredKey) || skipKeys.has(requiredKey))) {
        return false;
      }
      try {
        const view = await latest.options.persist(latest.recordId, data);
        const created = latest.recordId === null;
        // Later saves in the same burst must update this record, never create another.
        latestRef.current = { ...latestRef.current, recordId: view.record_id, baseData: view.data };
        setRecordId(view.record_id);
        setSavedView(view);
        latest.options.onSaved?.(view, created);
        return true;
      } catch (reason) {
        const error = toIpcError(reason);
        if (error.code === 'validation_failed') {
          refused = [...refused, ...toServerErrors(table, latest.form, error.fieldErrors)];
          setServerErrors(refused);
          if (error.fieldErrors.length === 0) {
            return false;
          }
          // Save the other changes without the refused fields.
          continue;
        }
        if (error.code === 'not_found' && latest.recordId !== null) {
          latest.options.onMissing?.();
          return false;
        }
        if (error.code === 'locked') {
          // The window is locking; the unlock screen replaces the panel.
          return false;
        }
        showToast({ message: t(`errors.${error.code}`), level: 'error', testId: 'panels-save-error-toast' });
        return false;
      }
    }
    return false;
  }, [table, requiredKey]);

  const autosave = useAutosave({ changeKey, dirty, canSave, isNew, save });

  const update = useCallback(<K extends keyof F>(key: K, value: F[K]) => {
    setForm((current) => ({ ...current, [key]: value }));
    setChangeKey((count) => count + 1);
    setTouched((current) => (current.has(key) ? current : new Set([...current, key])));
  }, []);

  const touch = useCallback(() => setChangeKey((count) => count + 1), []);

  const errors = useMemo(() => {
    const shown = validation.errors.filter((error) => {
      if (error.reason !== 'required') {
        return true;
      }
      const key = keyForField(table, error.path);
      return key !== undefined && touched.has(key);
    });
    return [...shown, ...active.map((entry) => entry.error)];
  }, [table, validation, active, touched]);

  return { recordId, form, base, errors, saveState: autosave.state, update, touch, flush: autosave.flush };
}
