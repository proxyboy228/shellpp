// Empty states of the Hosts screen (UI 2.3): an empty vault, an empty group
// folder, and a search without matches.

import { Plus, SearchX, Server } from 'lucide-react';
import { Button } from '../../components';
import { t } from '../../i18n';
import styles from './HostsScreen.module.css';

export function VaultEmptyState({ onCreate }: { onCreate: () => void }) {
  return (
    <div className={styles.empty} data-testid="hosts-empty-state">
      <span className={styles.emptyIcon} aria-hidden="true">
        <Server size={28} strokeWidth={1.5} focusable="false" />
      </span>
      <h3 className={styles.emptyTitle}>{t('hosts.empty.title')}</h3>
      <p className={styles.emptyText}>{t('hosts.empty.text')}</p>
      <div className={styles.emptyActions}>
        <Button variant="primary" icon={<Plus />} onClick={onCreate} data-testid="hosts-empty-create">
          {t('hosts.empty.create')}
        </Button>
        {/* `hosts-import` (stage 3): "Import from ~/.ssh/config" is hidden. */}
      </div>
    </div>
  );
}

export function FolderEmptyState() {
  return (
    <p className={styles.emptyLine} data-testid="hosts-folder-empty">
      {t('hosts.empty.folder')}
    </p>
  );
}

export function NoResults() {
  return (
    <div className={styles.empty} data-testid="hosts-no-results">
      <span className={styles.emptyIcon} aria-hidden="true">
        <SearchX size={28} strokeWidth={1.5} focusable="false" />
      </span>
      <p className={styles.emptyText}>{t('hosts.empty.no_results')}</p>
    </div>
  );
}
