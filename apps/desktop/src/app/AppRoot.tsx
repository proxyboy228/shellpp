// Root of an app window (plan, "Routing"):
//
//   boot -> initBackend -> createEventBuffer -> state_snapshot -> release(v)
//     startup error      -> FatalErrorScreen (errors.<code>)
//     first_run          -> TitleBar(minimal) + FirstRunScreen
//     locked             -> TitleBar(minimal) + UnlockScreen | RecoveryUnlockScreen
//     unlocked           -> TitleBar(full) + MainWindow (Vaults tab, Settings, dialogs)
//
// The lock screens cover the whole window content; only the window buttons
// stay (UI 13.2). `data-lock-state` follows what the window shows: during the
// recovery key step of first run it stays `first_run`.
//
// Stage 2: the terminal area (features/terminal `TerminalHost`) is mounted
// from the first unlock on and stays mounted while locked, hidden and inert
// under the unlock screen, so sessions keep their scrollback and output
// (docs/security.md, section 8). The close confirmation dialogs work in every
// lock state.
//
// Owner: lead.

import { CircleAlert } from 'lucide-react';
import { useEffect, useState } from 'react';
import { FirstRunScreen, RecoveryUnlockScreen, UnlockScreen } from '../features/lock';
import { CloseGuardDialogs, startSessionsSync, TerminalHost, useWorkspace } from '../features/terminal';
import { t } from '../i18n';
import { installActivityReporter } from './activity';
import { applyAppearance } from './appearance';
import { MainWindow } from './MainWindow';
import { RenderErrorBoundary } from './RenderErrorBoundary';
import { startSession } from './session';
import { useLockStatus, useOs, useSettings, useViewState, useWindowState } from './store';
import { TitleBar } from './TitleBar/TitleBar';
import { ToastRegion } from './ToastRegion/ToastRegion';
import styles from './AppRoot.module.css';

export type DisplayedLockState = 'booting' | 'first_run' | 'locked' | 'unlocked' | 'error';

function FatalErrorScreen({ code, found }: { code: string; found: number | null }) {
  return (
    <div className={styles.fatal} role="alert" data-testid="app-fatal-error">
      <CircleAlert className={styles.fatalIcon} size={24} strokeWidth={1.75} aria-hidden="true" />
      <h1 className={styles.fatalTitle}>{t('fatal.title')}</h1>
      <p className={styles.fatalText}>{t(`errors.${code}`)}</p>
      {found === null ? null : <p className={styles.fatalDetail}>{t('fatal.schema_version', { found })}</p>}
      <p className={styles.fatalHint}>{t('fatal.hint')}</p>
    </div>
  );
}

export function AppRoot() {
  const windowState = useWindowState();
  const lock = useLockStatus();
  const view = useViewState();
  const settings = useSettings();
  const os = useOs();

  const workspace = useWorkspace();
  // The terminal area mounts at the first unlock and then stays.
  const [terminalsMounted, setTerminalsMounted] = useState(false);

  useEffect(() => startSession(), []);
  useEffect(() => startSessionsSync(), []);
  useEffect(() => applyAppearance(settings), [settings]);
  // Input counts for auto-lock whenever the core is unlocked, including the
  // recovery key step of first run, which runs unlocked (UI 13.1, 13.3).
  const lockState = lock?.state;
  useEffect(() => (lockState === 'unlocked' ? installActivityReporter() : undefined), [lockState]);

  let displayed: DisplayedLockState;
  if (windowState.phase === 'error') {
    displayed = 'error';
  } else if (windowState.phase === 'booting' || lock === null) {
    displayed = 'booting';
  } else if (lock.state === 'first_run' || (lock.state === 'unlocked' && view.firstRunInProgress)) {
    displayed = 'first_run';
  } else {
    displayed = lock.state;
  }
  if (displayed === 'unlocked' && !terminalsMounted) {
    setTerminalsMounted(true);
  }
  const terminalsVisible = displayed === 'unlocked' && workspace.active !== 'vaults';

  return (
    <div
      className={styles.root}
      data-testid="app-root"
      data-window-label={windowState.label === '' ? undefined : windowState.label}
      data-lock-state={displayed}
      data-os={os ?? undefined}
    >
      <TitleBar variant={displayed === 'unlocked' ? 'full' : 'minimal'} />
      <div className={styles.content}>
        {displayed === 'booting' ? <div className={styles.booting} aria-busy="true" /> : null}
        {displayed === 'error' ? (
          <FatalErrorScreen code={windowState.errorCode ?? 'internal'} found={windowState.errorFound} />
        ) : null}
        {displayed === 'first_run' ? <FirstRunScreen /> : null}
        {displayed === 'locked' ? view.unlockMode === 'recovery' ? <RecoveryUnlockScreen /> : <UnlockScreen /> : null}
        {terminalsMounted ? (
          <RenderErrorBoundary>
            <TerminalHost visible={terminalsVisible} />
          </RenderErrorBoundary>
        ) : null}
        {displayed === 'unlocked' ? (
          <RenderErrorBoundary>
            <MainWindow />
          </RenderErrorBoundary>
        ) : null}
      </div>
      <CloseGuardDialogs />
      <ToastRegion />
    </div>
  );
}

export default AppRoot;
