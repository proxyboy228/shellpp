// The key records of the personal vault (`keys_list`, no secrets), reloaded
// on `records:changed` with class 3 (keys). Used by the "+ -> Key" dialog and
// the linked key field of the host panel (UI 3.1).
//
// Owner: S7.

import { useEffect, useState } from 'react';
import { keysList, subscribe, type KeyView } from '../../ipc';

/** Record class code of keys (docs/data-model.md, section 2.2). */
const CLASS_KEYS = 0x03;

/** The keys, or null until the first answer (an empty list when the query fails). */
export function useKeys(): readonly KeyView[] | null {
  const [keys, setKeys] = useState<readonly KeyView[] | null>(null);
  const [reloadKey, setReloadKey] = useState(0);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void subscribe('records:changed', (event) => {
      if (event.classes.includes(CLASS_KEYS)) {
        setReloadKey((key) => key + 1);
      }
    }).then(
      (stop) => {
        if (disposed) {
          stop();
        } else {
          unlisten = stop;
        }
      },
      () => undefined,
    );
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    keysList().then(
      (list) => {
        if (!cancelled) {
          setKeys(list);
        }
      },
      () => {
        if (!cancelled) {
          setKeys([]);
        }
      },
    );
    return () => {
      cancelled = true;
    };
  }, [reloadKey]);

  return keys;
}
