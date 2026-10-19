// Notification center (UI 1.5): the title bar button with the unread counter
// and the list popover, newest first. Opening the list marks every
// notification as read. Toasts for new notifications are shown by the window
// session (src/app/session.ts). Notifications live in memory only.
//
// Owner: W4.

import { Bell, CircleAlert, CircleCheck, Info, TriangleAlert } from 'lucide-react';
import { useEffect, useId, useRef, useState } from 'react';
import { IconButton } from '../../components';
import { t } from '../../i18n';
import { notificationsMarkRead, type Notification, type NotificationLevel } from '../../ipc';
import { notificationsStore, useNotifications } from '../store';
import styles from './NotificationCenter.module.css';

const LEVEL_ICONS = {
  info: Info,
  success: CircleCheck,
  warning: TriangleAlert,
  error: CircleAlert,
} as const satisfies Record<NotificationLevel, unknown>;

function formatTime(timestamp: string): string {
  const date = new Date(timestamp);
  return Number.isNaN(date.getTime()) ? '' : date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function markAllRead(): void {
  notificationsMarkRead()
    .then((list) => notificationsStore.set(list))
    .catch(() => undefined);
}

function NotificationItem({ item }: { item: Notification }) {
  const Icon = LEVEL_ICONS[item.level];
  return (
    <li className={styles.item} data-testid="notifications-item" data-id={item.id} data-read={item.read}>
      <span className={styles.itemIcon} data-level={item.level} aria-label={t(`notifications.level.${item.level}`)} role="img">
        <Icon size={16} strokeWidth={1.75} aria-hidden="true" />
      </span>
      <span className={styles.itemText}>{t(item.message_key, item.params)}</span>
      <time className={styles.itemTime} dateTime={item.created_at}>
        {formatTime(item.created_at)}
      </time>
    </li>
  );
}

export function NotificationCenter() {
  const notifications = useNotifications();
  const [open, setOpen] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const popoverRef = useRef<HTMLDivElement>(null);
  const titleId = useId();

  const toggle = () => setOpen((current) => !current);

  useEffect(() => {
    if (!open) {
      return undefined;
    }
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (
        target instanceof Node &&
        (popoverRef.current?.contains(target) === true || buttonRef.current?.contains(target) === true)
      ) {
        return;
      }
      setOpen(false);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !event.defaultPrevented) {
        event.preventDefault();
        setOpen(false);
        buttonRef.current?.focus();
      }
    };
    document.addEventListener('pointerdown', handlePointerDown, true);
    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown, true);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [open]);

  // Opening the list marks everything read; so do new notifications that
  // arrive while it is open.
  useEffect(() => {
    if (open && notifications.unread > 0) {
      markAllRead();
    }
  }, [open, notifications.unread]);

  const label =
    notifications.unread > 0
      ? `${t('titlebar.notifications')}, ${t('titlebar.notifications_unread', { count: notifications.unread })}`
      : t('titlebar.notifications');

  return (
    <div className={styles.wrap}>
      <IconButton
        ref={buttonRef}
        icon={<Bell />}
        aria-label={label}
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={toggle}
        data-testid="titlebar-notifications-button"
      />
      {notifications.unread > 0 ? (
        <span className={styles.badge} aria-hidden="true" data-testid="titlebar-notifications-badge">
          {notifications.unread > 99 ? '99+' : notifications.unread}
        </span>
      ) : null}
      {open ? (
        <div
          ref={popoverRef}
          className={styles.popover}
          role="dialog"
          aria-labelledby={titleId}
          data-testid="notifications-popover"
        >
          <h2 id={titleId} className={styles.title}>
            {t('notifications.title')}
          </h2>
          {notifications.items.length === 0 ? (
            <p className={styles.empty} data-testid="notifications-empty">
              {t('notifications.empty')}
            </p>
          ) : (
            <ul className={styles.list}>
              {notifications.items.map((item) => (
                <NotificationItem key={item.id} item={item} />
              ))}
            </ul>
          )}
        </div>
      ) : null}
    </div>
  );
}

export default NotificationCenter;
