// OS icon of hosts and session tabs (UI 1.1, 2.3, 3.1, 6; docs/data-model.md,
// section 7.6). Generic glyphs per OS family, never distribution or vendor
// logos (trademark policies, UI 2.3): stage 2 plan, decision 12. A host
// without a detection result shows the neutral server icon.
//
// Owner: lead (shared by S6 and S7; add families or glyphs only through the plan).

import { AppWindow, Cpu, HardDrive, Laptop, Server, SquareTerminal, type LucideIcon } from 'lucide-react';
import type { HostOs, OsFamily } from '../../ipc';
import styles from './OsIcon.module.css';

const ICONS: Readonly<Record<OsFamily, LucideIcon>> = {
  linux: SquareTerminal,
  windows: AppWindow,
  macos: Laptop,
  bsd: Cpu,
  other: HardDrive,
};

export interface OsIconProps {
  /** The host's `os`; absent or null = not detected (neutral icon). */
  os?: HostOs | null;
  /** Pixel size. Default 16. */
  size?: number;
  className?: string;
  'data-testid'?: string;
}

/**
 * Decorative icon (`aria-hidden`): the host title next to it carries the
 * meaning. `data-os-family` and `data-os-distro` expose the detection result
 * to tests (absent for the neutral icon).
 */
export function OsIcon({ os, size = 16, className, 'data-testid': testId }: OsIconProps) {
  const Icon = os === undefined || os === null ? Server : ICONS[os.family];
  return (
    <span
      className={className === undefined ? styles.icon : `${styles.icon} ${className}`}
      data-testid={testId}
      data-os-family={os?.family}
      data-os-distro={os?.distro}
      aria-hidden="true"
    >
      <Icon size={size} strokeWidth={1.75} />
    </span>
  );
}

export default OsIcon;
