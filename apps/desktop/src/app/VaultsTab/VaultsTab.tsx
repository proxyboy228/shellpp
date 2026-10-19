// The Vaults tab (UI 1.2): the sidebar with the sections of the vault and the
// main area with the screen of the active section: Hosts, and from stage 2
// Known Hosts (UI 10.1). Other sections are not rendered (src/stage.ts). The
// sidebar can be hidden from View > Show or hide sidebar.
//
// Owner: lead.

import { ArrowLeftRight, Code, KeyRound, ScrollText, Server, ShieldCheck, type LucideIcon } from 'lucide-react';
import { HostsScreen } from '../../features/hosts';
import { KnownHostsScreen } from '../../features/knownhosts';
import { t } from '../../i18n';
import { isVisible, type FeatureId } from '../../stage';
import { updateView, useDialogs, useSelectRequest, useSettings, useVaults, useViewState, type VaultsSection } from '../store';
import styles from './VaultsTab.module.css';

interface Section {
  id: string;
  labelKey: string;
  icon: LucideIcon;
  /** Absent for sections without a screen yet. */
  section?: VaultsSection;
  /** Absent for sections of stage 1. */
  feature?: FeatureId;
}

const SECTIONS: readonly Section[] = [
  { id: 'sidebar-hosts', labelKey: 'sidebar.hosts', icon: Server, section: 'hosts' },
  { id: 'sidebar-keychain', labelKey: 'sidebar.keychain', icon: KeyRound, feature: 'sidebar-keychain' },
  { id: 'sidebar-port-forwarding', labelKey: 'sidebar.port_forwarding', icon: ArrowLeftRight, feature: 'sidebar-port-forwarding' },
  { id: 'sidebar-snippets', labelKey: 'sidebar.snippets', icon: Code, feature: 'sidebar-snippets' },
  { id: 'sidebar-known-hosts', labelKey: 'sidebar.known_hosts', icon: ShieldCheck, section: 'known_hosts', feature: 'sidebar-known-hosts' },
  { id: 'sidebar-logs', labelKey: 'sidebar.logs', icon: ScrollText, feature: 'sidebar-logs' },
];

function Sidebar({ current }: { current: VaultsSection }) {
  const sections = SECTIONS.filter((section) => section.feature === undefined || isVisible(section.feature));
  return (
    <nav className={styles.sidebar} aria-label={t('sidebar.label')} data-testid="sidebar">
      <ul className={styles.sections}>
        {sections.map((section) => {
          const Icon = section.icon;
          const selected = section.section !== undefined && section.section === current;
          return (
            <li key={section.id}>
              <button
                type="button"
                className={styles.section}
                aria-current={selected ? 'page' : undefined}
                onClick={() => {
                  if (section.section !== undefined) {
                    updateView({ vaultsSection: section.section });
                  }
                }}
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

export interface VaultsTabProps {
  /** The Vaults tab is the active title bar tab (screens handle keys only then). */
  active?: boolean;
}

export function VaultsTab({ active = true }: VaultsTabProps) {
  const view = useViewState();
  const dialogs = useDialogs();
  const vaults = useVaults();
  const settings = useSettings();
  const selectRequest = useSelectRequest();
  const vault = vaults.find((candidate) => candidate.kind === 'personal') ?? vaults[0];
  const section: VaultsSection = view.vaultsSection === 'known_hosts' && isVisible('sidebar-known-hosts') ? 'known_hosts' : 'hosts';
  const screenActive = active && !view.settingsOpen && !dialogs.about;

  return (
    <div className={styles.tab} role="tabpanel" aria-label={t('titlebar.vaults')} data-testid="vaults-tab">
      {view.sidebarVisible ? <Sidebar current={section} /> : null}
      <div className={styles.main} data-testid="vaults-tab-main">
        {vault === undefined ? null : section === 'known_hosts' ? (
          <KnownHostsScreen vault={vault} settings={settings} active={screenActive} />
        ) : (
          <HostsScreen vault={vault} settings={settings} active={screenActive} selectRequest={selectRequest} />
        )}
      </div>
    </div>
  );
}

export default VaultsTab;
