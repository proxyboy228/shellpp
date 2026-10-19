// Keeps a rendering error in one screen from blanking the whole window: the
// window content shows a short message with a Reload button instead. The
// error itself is not logged (it may contain record data).
//
// Owner: W4.

import { Component, type ReactNode } from 'react';
import { Button } from '../components';
import { t } from '../i18n';
import styles from './AppRoot.module.css';

interface Props {
  children: ReactNode;
}

interface State {
  failed: boolean;
}

export class RenderErrorBoundary extends Component<Props, State> {
  override state: State = { failed: false };

  static getDerivedStateFromError(): State {
    return { failed: true };
  }

  override render() {
    if (!this.state.failed) {
      return this.props.children;
    }
    return (
      <div className={styles.fatal} role="alert" data-testid="app-render-error">
        <h1 className={styles.fatalTitle}>{t('render_error.title')}</h1>
        <p className={styles.fatalHint}>{t('render_error.text')}</p>
        <Button variant="primary" onClick={() => window.location.reload()} data-testid="app-render-error-reload">
          {t('render_error.reload')}
        </Button>
      </div>
    );
  }
}
