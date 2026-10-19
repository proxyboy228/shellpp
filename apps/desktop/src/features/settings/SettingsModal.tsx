// Settings window (UI 12.1): a modal over the app window, closed with X or
// Esc. Navigation on the left, the section on the right, scrolling
// independently; changes apply immediately. Stage 1 shows only the Security
// section (UI 13.4); the other sections and the Theme dropdown are not
// rendered (src/stage.ts). Below the navigation: the external links and the
// app version.
//
// Owner: W4. Plan, decision 1: an in-window modal, one per app window.

import { ExternalLink } from 'lucide-react';
import { showErrorToast } from '../../app/errors';
import { CHANGELOG_URL, DOCUMENTATION_URL, ISSUES_URL } from '../../app/links';
import { settingsStore, useAppInfo, useSettings, enumSetting } from '../../app/store';
import { Modal, Select } from '../../components';
import { t } from '../../i18n';
import { externalOpenUrl, settingsSet } from '../../ipc';
import { isVisible, type FeatureId } from '../../stage';
import { SecuritySection } from './SecuritySection';
import styles from './Settings.module.css';

export interface SettingsModalProps {
  open: boolean;
  onClose: () => void;
}

interface NavEntry {
  id: string;
  labelKey: string;
  feature?: FeatureId;
}

/** Navigation of UI 12.1 in order; only entries visible in this stage are rendered. */
const NAVIGATION: readonly NavEntry[] = [
  { id: 'settings-nav-account-sync', labelKey: 'settings.account_sync', feature: 'settings-account-sync' },
  { id: 'settings-nav-security', labelKey: 'settings.security' },
  { id: 'settings-nav-ssh-id', labelKey: 'settings.ssh_id', feature: 'settings-ssh-id' },
  { id: 'settings-nav-team', labelKey: 'settings.team', feature: 'settings-team' },
  { id: 'settings-nav-terminal', labelKey: 'settings.terminal', feature: 'settings-terminal' },
  { id: 'settings-nav-sftp', labelKey: 'settings.sftp', feature: 'settings-sftp' },
  { id: 'settings-nav-logs', labelKey: 'settings.logs', feature: 'settings-logs' },
  { id: 'settings-nav-shortcuts', labelKey: 'settings.shortcuts', feature: 'settings-shortcuts' },
  { id: 'settings-nav-updates', labelKey: 'settings.updates', feature: 'settings-updates' },
];

const LINKS = [
  { id: 'settings-link-documentation', labelKey: 'settings.link.documentation', url: DOCUMENTATION_URL },
  { id: 'settings-link-report-bug', labelKey: 'settings.link.report_bug', url: ISSUES_URL },
  { id: 'settings-link-changelog', labelKey: 'settings.link.changelog', url: CHANGELOG_URL },
] as const;

type ThemeChoice = 'auto' | 'light' | 'dark';

function ThemeSelect() {
  const settings = useSettings();
  const value = enumSetting<ThemeChoice>(settings, 'appearance.ui_theme', ['auto', 'light', 'dark'], 'auto');
  return (
    <Select<ThemeChoice>
      label={t('settings.theme')}
      value={value}
      options={[
        { value: 'auto', label: t('settings.theme.auto') },
        { value: 'light', label: t('settings.theme.light') },
        { value: 'dark', label: t('settings.theme.dark') },
      ]}
      onValueChange={(next) => {
        settingsSet({ values: { 'appearance.ui_theme': next } })
          .then((values) => settingsStore.set(values))
          .catch(showErrorToast);
      }}
      data-testid="settings-theme-select"
    />
  );
}

export function SettingsModal({ open, onClose }: SettingsModalProps) {
  const info = useAppInfo();
  const navigation = NAVIGATION.filter((entry) => entry.feature === undefined || isVisible(entry.feature));
  return (
    <Modal
      open={open}
      onClose={onClose}
      title={t('settings.title')}
      closeLabel={t('common.close')}
      size="large"
      className={styles.modal}
      data-testid="settings-modal"
    >
      <div className={styles.layout}>
        <nav className={styles.nav} aria-label={t('settings.nav')} data-testid="settings-nav">
          <ul className={styles.navList}>
            {navigation.map((entry) => {
              // Security is the only section in stage 1, so it is always selected.
              const current = entry.id === 'settings-nav-security';
              return (
                <li key={entry.id}>
                  <button
                    type="button"
                    className={styles.navItem}
                    aria-current={current ? 'page' : undefined}
                    data-testid={entry.id}
                  >
                    {t(entry.labelKey)}
                  </button>
                </li>
              );
            })}
          </ul>
          {isVisible('settings-theme') ? <ThemeSelect /> : null}
          <ul className={styles.links}>
            {LINKS.map((link) => (
              <li key={link.id}>
                <button
                  type="button"
                  className={styles.linkItem}
                  onClick={() => externalOpenUrl(link.url).catch(showErrorToast)}
                  data-testid={link.id}
                >
                  <span>{t(link.labelKey)}</span>
                  <ExternalLink size={14} strokeWidth={1.75} aria-label={t('settings.opens_in_browser')} role="img" />
                </button>
              </li>
            ))}
          </ul>
          <p className={styles.version} data-testid="settings-version">
            {t('settings.version', { version: info?.version ?? '' })}
          </p>
        </nav>
        <div className={styles.content}>
          <SecuritySection />
        </div>
      </div>
    </Modal>
  );
}

export default SettingsModal;
