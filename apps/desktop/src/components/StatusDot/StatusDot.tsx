import type { HTMLAttributes } from 'react';
import styles from './StatusDot.module.css';

/** Connection or rule state shown by the dot (design-system 3.3). */
export type StatusDotStatus = 'connected' | 'connecting' | 'error' | 'inactive';

/** Default accessible labels; pass `label` to use localized text. */
export const STATUS_DOT_LABELS: Readonly<Record<StatusDotStatus, string>> = {
  connected: 'Connected',
  connecting: 'Connecting',
  error: 'Error',
  inactive: 'Inactive',
};

const statusClass: Readonly<Record<StatusDotStatus, string>> = {
  connected: styles.connected,
  connecting: styles.connecting,
  error: styles.error,
  inactive: styles.inactive,
};

export interface StatusDotProps extends Omit<HTMLAttributes<HTMLSpanElement>, 'children' | 'role'> {
  status: StatusDotStatus;
  /** Accessible label. Default: the English name of the status. */
  label?: string;
  /**
   * The state is already given in text next to the dot: hide the dot from
   * assistive technology instead of labelling it.
   */
  decorative?: boolean;
  'data-testid'?: string;
}

/**
 * Status dot of 8 px: connected `--success`, connecting `--warning`,
 * error `--danger`, inactive `--text-muted`.
 */
export function StatusDot({
  status,
  label,
  decorative = false,
  className,
  'data-testid': testId,
  ...rest
}: StatusDotProps) {
  const rootClassName = [styles.dot, statusClass[status], className].filter(Boolean).join(' ');
  const accessibleLabel = label ?? STATUS_DOT_LABELS[status];

  return (
    <span
      title={decorative ? undefined : accessibleLabel}
      {...rest}
      className={rootClassName}
      role={decorative ? undefined : 'img'}
      aria-label={decorative ? undefined : accessibleLabel}
      aria-hidden={decorative ? 'true' : undefined}
      data-status={status}
      data-testid={testId}
    />
  );
}

export default StatusDot;
