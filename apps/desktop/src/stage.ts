// Stage flags (docs/architecture.md, section 6.2; docs/decisions.md, decision 3).
//
// Elements of later stages are hidden, not disabled: components check
// `isVisible(feature)` and render nothing when it is false, so their
// `data-testid` values are absent from the DOM. The flag is a build constant:
// never a setting, an environment variable or a remote switch.
//
// `scripts/hidden-features.mjs` reads this file and prints the features whose
// stage is greater than BUILD_STAGE for the stage report. Keep one
// `'feature-id': <stage>,` entry per line; `LATER` marks features without a
// stage yet.
//
// Owner: W4. New ids are added only through the stage plan.

/** The stage this build implements. */
export const BUILD_STAGE = 1;

/** A feature planned after the numbered stages (biometric unlock after the stage 6 decision, D5). */
export const LATER = 100;

/** Feature id -> the stage that implements it. Features of stage 1 and earlier are not listed. */
export const FEATURE_STAGES = {
  // UI 1.1 Top bar
  'topbar-vault-switcher': 8,
  'topbar-sftp-tab': 4,
  'topbar-session-tabs': 2,
  'topbar-new-tab': 2,
  'topbar-update-button': 9,
  'topbar-terminal-sidebar-toggle': 2,
  // UI 1.2 Sidebar sections
  'sidebar-keychain': 3,
  'sidebar-port-forwarding': 5,
  'sidebar-snippets': 5,
  'sidebar-known-hosts': 2,
  'sidebar-logs': 6,
  // UI 1.4 App menu and shortcuts
  'menu-sftp': 4,
  'menu-new-local-terminal': 2,
  'menu-new-serial': 9,
  'menu-window-tabs': 2,
  'menu-check-updates': 9,
  'shortcut-search-hosts-and-tabs': 2,
  'shortcut-snippets': 5,
  // UI 2.1-2.4 Hosts screen
  'hosts-quick-connect': 2,
  'hosts-import': 3,
  'hosts-cloud-integrations': 10,
  'hosts-local-terminal-button': 2,
  'hosts-serial-button': 9,
  'hosts-vault-members': 8,
  'hosts-conflict-badge': 7,
  'host-connect': 2,
  'host-connect-sftp': 4,
  'host-add-to-workspace': 9,
  'host-collaborate': 8,
  'host-vault-transfer': 8,
  // UI 3.1-3.4 Host and group panels
  'panel-vault-field': 8,
  'host-share': 8,
  'host-credentials-key': 2,
  'host-credentials-identity': 3,
  'group-credentials': 3,
  'host-startup-snippet': 5,
  'host-chaining': 5,
  'host-proxy': 5,
  'host-telnet': 9,
  'host-mosh': 10,
  // UI 12.1 Settings window sections
  'settings-account-sync': 3,
  'settings-ssh-id': 8,
  'settings-team': 8,
  'settings-terminal': 6,
  'settings-sftp': 6,
  'settings-logs': 6,
  'settings-shortcuts': 6,
  'settings-updates': 9,
  'settings-theme': 6,
  // UI 13 Lock and account
  'security-password-change-sync-warning': 7,
  'sync-connect-wizard': 7,
  'security-biometric-unlock': LATER,
} as const satisfies Record<string, number>;

export type FeatureId = keyof typeof FEATURE_STAGES;

/** True when the feature belongs to this stage or an earlier one. */
export function isVisible(feature: FeatureId): boolean {
  return FEATURE_STAGES[feature] <= BUILD_STAGE;
}

/** Features hidden in this build, sorted by stage then id (for reports and tests). */
export function hiddenFeatures(): { feature: FeatureId; stage: number }[] {
  return (Object.entries(FEATURE_STAGES) as [FeatureId, number][])
    .filter(([, stage]) => stage > BUILD_STAGE)
    .map(([feature, stage]) => ({ feature, stage }))
    .sort((a, b) => a.stage - b.stage || a.feature.localeCompare(b.feature));
}
