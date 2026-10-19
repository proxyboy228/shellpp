// Stage flags (docs/architecture.md 6.2): later-stage features are hidden.

import { describe, expect, it } from 'vitest';
import stageSource from './stage.ts?raw';
import { BUILD_STAGE, FEATURE_STAGES, LATER, hiddenFeatures, isVisible, type FeatureId } from './stage';

/** Features of stage 2: visible in this build. */
const STAGE_2: FeatureId[] = [
  'topbar-session-tabs',
  'topbar-new-tab',
  'topbar-terminal-sidebar-toggle',
  'sidebar-known-hosts',
  'menu-new-local-terminal',
  'menu-window-tabs',
  'shortcut-search-hosts-and-tabs',
  'hosts-quick-connect',
  'hosts-local-terminal-button',
  'host-connect',
  'host-credentials-key',
];

describe('stage flags', () => {
  it('builds stage 2: stage 2 features are visible, later ones hidden', () => {
    expect(BUILD_STAGE).toBe(2);
    for (const feature of Object.keys(FEATURE_STAGES) as FeatureId[]) {
      expect(isVisible(feature), feature).toBe(FEATURE_STAGES[feature] <= 2);
    }
    for (const feature of STAGE_2) {
      expect(FEATURE_STAGES[feature], feature).toBe(2);
      expect(isVisible(feature), feature).toBe(true);
    }
    expect(hiddenFeatures()).toHaveLength(Object.keys(FEATURE_STAGES).length - STAGE_2.length);
  });

  it('lists hidden features by stage, then id, with LATER last', () => {
    const hidden = hiddenFeatures();
    expect(hidden[0]).toEqual({ feature: 'group-credentials', stage: 3 });
    expect(hidden.at(-1)).toEqual({ feature: 'security-biometric-unlock', stage: LATER });
    for (let index = 1; index < hidden.length; index += 1) {
      const previous = hidden[index - 1];
      const current = hidden[index];
      expect(previous.stage < current.stage || (previous.stage === current.stage && previous.feature < current.feature)).toBe(true);
    }
  });

  it('keeps one entry per line for scripts/hidden-features.mjs', () => {
    const entries = stageSource.match(/^ {2}'[a-z0-9-]+': (\d+|LATER),$/gm) ?? [];
    expect(entries).toHaveLength(Object.keys(FEATURE_STAGES).length);
  });

  it('covers the hidden test ids of the stage 2 plan', () => {
    const expected: FeatureId[] = [
      'topbar-vault-switcher',
      'topbar-sftp-tab',
      'topbar-update-button',
      'sidebar-keychain',
      'sidebar-port-forwarding',
      'sidebar-snippets',
      'sidebar-logs',
      'menu-sftp',
      'menu-new-serial',
      'menu-check-updates',
      'host-connect-sftp',
      'host-add-to-workspace',
      'host-credentials-identity',
      'terminal-sidebar-snippets',
      'terminal-autocomplete',
      'history-snippet-hint',
      'session-multiplayer',
      'session-broadcast',
      'terminal-keyword-highlighting',
      'newtab-workspaces',
      'known-hosts-import',
      'known-hosts-export',
      'settings-account-sync',
      'settings-terminal',
      'settings-theme',
      'security-password-change-sync-warning',
      'security-biometric-unlock',
    ];
    for (const feature of expected) {
      expect(FEATURE_STAGES[feature], feature).toBeGreaterThan(BUILD_STAGE);
    }
  });
});
