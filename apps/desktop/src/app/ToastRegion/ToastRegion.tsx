// Toast region (UI 1.5, 2.4): renders the toasts of `src/app/toast.ts` at the
// bottom of the window. Notification toasts last 5 seconds; the undo toast of a
// removal lasts 8 seconds and has an action button.
//
// Owner: W4.

import { CircleAlert, CircleCheck, Info, TriangleAlert, X } from 'lucide-react';
import { useSyncExternalStore } from 'react';
import { Button, IconButton } from '../../components';
import { t } from '../../i18n';
import type { NotificationLevel } from '../../ipc';
import { dismissToast, getToasts, subscribeToasts, type Toast } from '../toast';
import styles from './ToastRegion.module.css';

const subscribe = (listener: () => void) => subscribeToasts(() => listener());

const LEVEL_ICONS = {
  info: Info,
  success: CircleCheck,
  warning: TriangleAlert,
  error: CircleAlert,
} as const satisfies Record<NotificationLevel, unknown>;

function ToastItem({ toast }: { toast: Toast }) {
  const Icon = LEVEL_ICONS[toast.level];
  const action = toast.action;
  return (
    <div
      className={styles.toast}
      role={toast.level === 'error' ? 'alert' : 'status'}
      data-testid={toast.testId}
      data-id={toast.dataId}
      data-level={toast.level}
    >
      <span className={styles.icon} data-level={toast.level} aria-hidden="true">
        <Icon size={16} strokeWidth={1.75} />
      </span>
      <span className={styles.message}>{toast.message}</span>
      {action === undefined ? null : (
        <Button
          variant="text"
          className={styles.action}
          data-testid={action.testId}
          onClick={() => {
            action.onAction();
            dismissToast(toast.id);
          }}
        >
          {action.label}
        </Button>
      )}
      <IconButton
        size="sm"
        className={styles.dismiss}
        icon={<X />}
        aria-label={t('common.dismiss')}
        onClick={() => dismissToast(toast.id)}
      />
    </div>
  );
}

export function ToastRegion() {
  const toasts = useSyncExternalStore(subscribe, getToasts, getToasts);
  return (
    <section className={styles.region} aria-label={t('toasts.region')} data-testid="toast-region">
      {toasts.map((toast) => (
        <ToastItem key={toast.id} toast={toast} />
      ))}
    </section>
  );
}

export default ToastRegion;
