// Path of the open group with a return to any level (UI 2.3).

import { ChevronRight } from 'lucide-react';
import type { GroupView, Uuid } from '../../ipc';
import { t } from '../../i18n';
import styles from './HostsScreen.module.css';

export interface BreadcrumbsProps {
  /** Groups from the top level down to the open folder. */
  path: readonly GroupView[];
  onNavigate: (groupId: Uuid | null) => void;
}

export function Breadcrumbs({ path, onNavigate }: BreadcrumbsProps) {
  return (
    <nav aria-label={t('hosts.breadcrumbs.label')} data-testid="hosts-breadcrumbs">
      <ol className={styles.crumbs}>
        <li className={styles.crumbItem}>
          <button
            type="button"
            className={styles.crumb}
            aria-current={path.length === 0 ? 'page' : undefined}
            onClick={() => onNavigate(null)}
            data-testid="hosts-breadcrumb-root"
          >
            {t('hosts.breadcrumbs.root')}
          </button>
        </li>
        {path.map((group, index) => {
          const last = index === path.length - 1;
          return (
            <li key={group.record_id} className={styles.crumbItem}>
              <ChevronRight className={styles.crumbSeparator} size={16} strokeWidth={1.75} aria-hidden="true" focusable="false" />
              <button
                type="button"
                className={styles.crumb}
                aria-current={last ? 'page' : undefined}
                onClick={() => onNavigate(group.record_id)}
                data-group-id={group.record_id}
                data-testid="hosts-breadcrumb"
              >
                {group.data.name}
              </button>
            </li>
          );
        })}
      </ol>
    </nav>
  );
}

export default Breadcrumbs;
