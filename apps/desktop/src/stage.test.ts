// Stage flags (docs/architecture.md 6.2): later-stage features are hidden.

import { describe, expect, it } from 'vitest';
import stageSource from './stage.ts?raw';
import { BUILD_STAGE, FEATURE_STAGES, LATER, hiddenFeatures, isVisible, type FeatureId } from './stage';

describe('stage flags', () => {
  it('builds stage 1 and hides every listed feature', () => {
    expect(BUILD_STAGE).toBe(1);
    for (const feature of Object.keys(FEATURE_STAGES) as FeatureId[]) {
      expect(isVisible(feature)).toBe(false);
    }
    expect(hiddenFeatures()).toHaveLength(Object.keys(FEATURE_STAGES).length);
  });

  it('lists hidden features by stage, then id, with LATER last', () => {
    const hidden = hiddenFeatures();
    expect(hidden[0]).toEqual({ feature: 'host-connect', stage: 2 });
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

  it('covers the hidden test ids of the plan', () => {
    const expected: FeatureId[] = [
      'topbar-vault-switcher',
      'topbar-sftp-tab',
      'topbar-session-tabs',
      'topbar-new-tab',
      'topbar-update-button',
      'topbar-terminal-sidebar-toggle',
      'sidebar-keychain',
      'sidebar-port-forwarding',
      'sidebar-snippets',
      'sidebar-known-hosts',
      'sidebar-logs',
      'menu-sftp',
      'menu-new-local-terminal',
      'menu-new-serial',
      'menu-window-tabs',
      'menu-check-updates',
      'settings-account-sync',
      'settings-theme',
      'security-password-change-sync-warning',
      'security-biometric-unlock',
    ];
    for (const feature of expected) {
      expect(FEATURE_STAGES[feature]).toBeGreaterThan(BUILD_STAGE);
    }
  });
});
