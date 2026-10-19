import { Folder, Server, type LucideIcon } from 'lucide-react';
import type { HTMLAttributes, KeyboardEvent, MouseEvent, ReactNode } from 'react';
import styles from './HostCard.module.css';

/** The eight tile colors of hosts and groups (design-system 3.1). */
export type TagColor = 'graphite' | 'jade' | 'amber' | 'coral' | 'plum' | 'moss' | 'ocean' | 'rose';

export const TAG_COLORS: readonly TagColor[] = ['graphite', 'jade', 'amber', 'coral', 'plum', 'moss', 'ocean', 'rose'];

const tileColorClass: Readonly<Record<TagColor, string>> = {
  graphite: styles.tagGraphite,
  jade: styles.tagJade,
  amber: styles.tagAmber,
  coral: styles.tagCoral,
  plum: styles.tagPlum,
  moss: styles.tagMoss,
  ocean: styles.tagOcean,
  rose: styles.tagRose,
};

export type HostCardEvent = MouseEvent<HTMLDivElement> | KeyboardEvent<HTMLDivElement>;

interface HostCardCommonProps
  extends Omit<HTMLAttributes<HTMLDivElement>, 'children' | 'color' | 'title' | 'onSelect'> {
  /** Tile color. Default: graphite. */
  color?: TagColor;
  /** Selected card: `aria-selected` and the accent border. */
  selected?: boolean;
  /** Inactive card: opacity 0.45, no hover, no callbacks, not in the tab order. */
  disabled?: boolean;
  /**
   * Click or Space. The event is passed so that the grid can read
   * Ctrl and Shift for multiple selection.
   */
  onSelect?: (event: HostCardEvent) => void;
  /** Double click or Enter: connect to a host, open a group. */
  onOpen?: (event: HostCardEvent) => void;
  /** Content at the right edge, for example a status dot or a conflict badge. */
  trailing?: ReactNode;
  'data-testid'?: string;
}

export interface HostCardHostProps extends HostCardCommonProps {
  variant?: 'host';
  /** Host label. When empty, the address is the title. */
  label?: string;
  address: string;
  /** Secondary line: protocols, username, then the group or tags. */
  subtitle?: string;
  /** OS icon from lucide-react. Default: a neutral server icon. */
  icon?: LucideIcon;
}

export interface HostCardGroupProps extends HostCardCommonProps {
  variant: 'group';
  /** Group name. */
  label: string;
  /** Number of hosts, nested groups included. */
  hostCount: number;
  /** Text of the host count. Default: `1 host`, `N hosts`. */
  formatHostCount?: (count: number) => string;
}

export type HostCardProps = HostCardHostProps | HostCardGroupProps;

export function formatHostCountDefault(count: number): string {
  return `${count} ${count === 1 ? 'host' : 'hosts'}`;
}

interface HostCardContent {
  variant: 'host' | 'group';
  title: string;
  subtitle: string | undefined;
  Icon: LucideIcon;
}

function splitProps(props: HostCardProps): { content: HostCardContent; common: HostCardCommonProps } {
  if (props.variant === 'group') {
    const { variant, label, hostCount, formatHostCount = formatHostCountDefault, ...common } = props;
    return {
      content: { variant, title: label, subtitle: formatHostCount(hostCount), Icon: Folder },
      common,
    };
  }
  const { variant = 'host', label, address, subtitle, icon = Server, ...common } = props;
  const title = label !== undefined && label.trim() !== '' ? label : address;
  return {
    content: { variant, title, subtitle: subtitle === '' ? undefined : subtitle, Icon: icon },
    common,
  };
}

/**
 * Host or group card of 64 px (ui-behavior-spec 2.3). It is an `option`:
 * place it in an element with `role="listbox"` (the card grid), or pass
 * another `role` that supports `aria-selected`.
 *
 * With a `data-testid`, the title gets `<id>-title` and the second line
 * `<id>-subtitle` (host) or `<id>-count` (group). Other `data-*` attributes
 * are passed to the root element.
 */
export function HostCard(props: HostCardProps) {
  const { content, common } = splitProps(props);
  const {
    color = 'graphite',
    selected = false,
    disabled = false,
    onSelect,
    onOpen,
    trailing,
    className,
    onClick,
    onDoubleClick,
    onKeyDown,
    tabIndex,
    'data-testid': testId,
    ...rest
  } = common;
  const { variant, title, subtitle, Icon } = content;

  const rootClassName = [
    styles.card,
    selected ? styles.selected : undefined,
    disabled ? styles.disabled : undefined,
    className,
  ]
    .filter(Boolean)
    .join(' ');

  const handleClick = (event: MouseEvent<HTMLDivElement>) => {
    if (disabled) {
      return;
    }
    onClick?.(event);
    if (!event.defaultPrevented) {
      onSelect?.(event);
    }
  };

  const handleDoubleClick = (event: MouseEvent<HTMLDivElement>) => {
    if (disabled) {
      return;
    }
    onDoubleClick?.(event);
    if (!event.defaultPrevented) {
      onOpen?.(event);
    }
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (disabled) {
      return;
    }
    onKeyDown?.(event);
    // Keys pressed inside trailing content belong to that content.
    if (event.defaultPrevented || event.target !== event.currentTarget) {
      return;
    }
    if (event.key === 'Enter') {
      event.preventDefault();
      onOpen?.(event);
    } else if (event.key === ' ') {
      event.preventDefault();
      onSelect?.(event);
    }
  };

  return (
    <div
      role="option"
      {...rest}
      className={rootClassName}
      aria-selected={selected}
      aria-disabled={disabled ? 'true' : undefined}
      tabIndex={disabled ? -1 : (tabIndex ?? 0)}
      data-variant={variant}
      data-color={color}
      data-testid={testId}
      onClick={handleClick}
      onDoubleClick={handleDoubleClick}
      onKeyDown={handleKeyDown}
    >
      <span className={`${styles.tile} ${tileColorClass[color]}`} aria-hidden="true">
        <Icon className={styles.icon} size={20} strokeWidth={1.75} aria-hidden="true" focusable="false" />
      </span>
      <span className={styles.text}>
        <span className={styles.title} title={title} data-testid={testId === undefined ? undefined : `${testId}-title`}>
          {title}
        </span>
        {subtitle === undefined ? null : (
          <span
            className={styles.subtitle}
            title={subtitle}
            data-testid={testId === undefined ? undefined : `${testId}-${variant === 'group' ? 'count' : 'subtitle'}`}
          >
            {subtitle}
          </span>
        )}
      </span>
      {trailing === undefined || trailing === null ? null : <span className={styles.trailing}>{trailing}</span>}
    </div>
  );
}

export default HostCard;
