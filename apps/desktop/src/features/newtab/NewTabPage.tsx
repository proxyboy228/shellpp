// The new tab page (UI 6): search with the Ctrl+K hint, Recent connections,
// Restore (setting `session.last`); "Create a workspace" is stage 9.
// Choosing a host connects in this tab (`openSession(target, {inTab})`);
// Alt opens it in a new window; choosing an open tab activates it and the
// new tab page stays open.
//
// Owner: S7 (skeleton by the lead).

import { RotateCcw } from 'lucide-react';
import { showErrorToast } from '../../app/errors';
import { Button } from '../../components';
import type { SessionTarget } from '../../ipc';
import { t } from '../../i18n';
import { activateTab, openSession, restoreLastSession } from '../connect/workspaceBridge';
import { useLastSessionAvailable } from '../terminal';
import { HostPicker } from './HostPicker';
import styles from './newtab.module.css';

export interface NewTabPageProps {
  tabId: string;
  /** The tab is the active one (focus goes to the search field when it becomes active). */
  active: boolean;
}

export function NewTabPage({ tabId, active }: NewTabPageProps) {
  const restoreAvailable = useLastSessionAvailable();

  const pickHost = (target: SessionTarget, { altKey }: { altKey: boolean }) => {
    openSession(target, altKey ? { newWindow: true } : { inTab: tabId }).catch(showErrorToast);
  };

  return (
    <div className={styles.page} data-tab-id={tabId} data-testid="newtab-page">
      <div className={styles.pageInner}>
        <HostPicker
          testIdPrefix="newtab"
          includeTabs
          active={active}
          onPickHost={pickHost}
          onPickTab={(id) => activateTab(id)}
          recentActions={
            <>
              {/* `newtab-create-workspace` and the workspaces block (`newtab-workspaces`, stage 9) are hidden. */}
              <Button
                variant="text"
                icon={<RotateCcw />}
                disabled={!restoreAvailable}
                onClick={() => {
                  restoreLastSession().catch(showErrorToast);
                }}
                data-testid="newtab-restore"
              >
                {t('newtab.restore')}
              </Button>
            </>
          }
        />
      </div>
    </div>
  );
}
