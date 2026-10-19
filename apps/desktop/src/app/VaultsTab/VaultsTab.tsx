// The Vaults tab (UI 1.2): the sidebar with the sections of the vault and the
// main area with the screen of the active section. Stage 1 has only the Hosts
// section; the others are not rendered (src/stage.ts). The sidebar can be
// hidden from View > Show or hide sidebar.
//
// Owner: W4.

import { ArrowLeftRight, Code, KeyRound, ScrollText, Server, ShieldCheck, type LucideIcon } from 'lucide-react';
import { HostsScreen } from '../../features/hosts';
import { t } from '../../i18n';
import { isVisible, type FeatureId } from '../../stage';
import { useDialogs, useSelectRequest, useSettings, useVaults, useViewState } from '../store';
import styles from './VaultsTab.module.css';

interface Section {
  id: string;
  labelKey: string;
  icon: LucideIcon;
  /** Absent for sections of this stage. */
  feature?: FeatureId;
}

const SECTIONS: readonly Section[] = [
  { id: 'sidebar-hosts', labelKey: 'sidebar.hosts', icon: Server },
  { id: 'sidebar-keychain', labelKey: 'sidebar.keychain', icon: KeyRound, feature: 'sidebar-keychain' },
  { id: 'sidebar-port-forwarding', labelKey: 'sidebar.port_forwarding', icon: ArrowLeftRight, feature: 'sidebar-port-forwarding' },
  { id: 'sidebar-snippets', labelKey: 'sidebar.snippets', icon: Code, feature: 'sidebar-snippets' },
  { id: 'sidebar-known-hosts', labelKey: 'sidebar.known_hosts', icon: ShieldCheck, feature: 'sidebar-known-hosts' },
  { id: 'sidebar-logs', labelKey: 'sidebar.logs', icon: ScrollText, feature: 'sidebar-logs' },
];

function Sidebar() {
  const sections = SECTIONS.filter((section) => section.feature === undefined || isVisible(section.feature));
  return (
    <nav className={styles.sidebar} aria-label={t('sidebar.label')} data-testid="sidebar">
      <ul className={styles.sections}>
        {sections.map((section) => {
          const Icon = section.icon;
          // Hosts is the only (and so the active) section in stage 1.
          const current = section.id === 'sidebar-hosts';
          return (
            <li key={section.id}>
              <button
                type="button"
                className={styles.section}
                aria-current={current ? 'page' : undefined}
                data-testid={section.id}
              >
                <Icon size={16} strokeWidth={1.75} aria-hidden="true" />
                <span>{t(section.labelKey)}</span>
              </button>
            </li>
          );
        })}
      </ul>
    </nav>
  );
}

export function VaultsTab() {
  const view = useViewState();
  const dialogs = useDialogs();
  const vaults = useVaults();
  const settings = useSettings();
  const selectRequest = useSelectRequest();
  const vault = vaults.find((candidate) => candidate.kind === 'personal') ?? vaults[0];

  return (
    <div className={styles.tab} role="tabpanel" aria-label={t('titlebar.vaults')} data-testid="vaults-tab">
      {view.sidebarVisible ? <Sidebar /> : null}
      <div className={styles.main} data-testid="vaults-tab-main">
        {vault === undefined ? null : (
          <HostsScreen
            vault={vault}
            settings={settings}
            active={!view.settingsOpen && !dialogs.about}
            selectRequest={selectRequest}
          />
        )}
      </div>
    </div>
  );
}

export default VaultsTab;
