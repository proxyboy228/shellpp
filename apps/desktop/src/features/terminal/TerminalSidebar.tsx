// Terminal sidebar (UI 4.2): on the right of the terminal area, which
// narrows. Icon tabs Tools, History and Appearance (Snippets is stage 5).
// Everything acts on the active pane of the active tab.
//
// - Tools: search in the terminal output with previous/next and "i of n"
//   (Ctrl+Shift+F focuses it); Autocomplete is a later stage.
// - History: the host's commands (local terminals and quick connects share
//   the "no host" list), search, newest first; a click inserts the command
//   without Enter (injected input, never recorded again); delete an entry;
//   clear the host's history with a confirmation. The "save as snippets"
//   card is stage 5.
// - Appearance: the Font row opens family, size and line spacing (global
//   settings); the theme list applies a theme to the session at once and
//   saves it on a saved host of the personal vault (decision 15).
//
// Owner: S6.

import { ChevronLeft, ChevronRight, Code, History, Minus, Palette, Plus, Search, Trash2, Wrench, X } from 'lucide-react';
import { useEffect, useMemo, useState, type FormEvent, type KeyboardEvent } from 'react';
import { showErrorToast } from '../../app/errors';
import { settingsStore, useStore, useVaults } from '../../app/store';
import { Button, IconButton, Modal, TextInput } from '../../components';
import { t } from '../../i18n';
import {
  historyClear,
  historyDelete,
  historyList,
  recordGet,
  recordSave,
  settingsSet,
  subscribe,
  toIpcError,
  type HistoryEntryView,
  type JsonValue,
  type SessionInfo,
} from '../../ipc';
import { isVisible } from '../../stage';
import { paneController, searchResultsStore } from './controllers';
import { frequentCommands, historyHost } from './historyView';
import {
  DEFAULT_FONT_FAMILY,
  FONT_SIZE_MAX,
  FONT_SIZE_MIN,
  LINE_HEIGHT_MAX,
  LINE_HEIGHT_MIN,
  useTerminalSettings,
} from './terminalSettings';
import { BUILTIN_THEMES, setSessionTheme, useSessionTheme, type BuiltinTheme } from './themes';
import { workspaceStore, type PaneId, type SidebarTab } from './workspace';
import styles from './TerminalSidebar.module.css';

const CLASS_HISTORY = 6;

interface TabProps {
  paneId: PaneId | null;
  session: SessionInfo | undefined;
}

// --- Tools -----------------------------------------------------------------------

function ToolsTab({ paneId }: TabProps) {
  const [term, setTerm] = useState('');
  const results = useStore(searchResultsStore);
  const current = paneId === null ? undefined : results.get(paneId);
  const controller = () => (paneId === null ? undefined : paneController(paneId));

  const search = (value: string, direction: 'next' | 'previous') => {
    const target = controller();
    if (target === undefined) {
      return;
    }
    if (value === '') {
      target.clearSearch();
    } else if (direction === 'next') {
      target.findNext(value);
    } else {
      target.findPrevious(value);
    }
  };

  // Leaving the Tools tab (or closing the sidebar) removes the match highlights.
  useEffect(
    () => () => {
      if (paneId !== null) {
        paneController(paneId)?.clearSearch();
      }
    },
    [paneId],
  );

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      search(term, event.shiftKey ? 'previous' : 'next');
    } else if (event.key === 'Escape' && term !== '') {
      event.preventDefault();
      setTerm('');
      search('', 'next');
    }
  };

  const count = current?.count ?? 0;
  const index = current === undefined || current.index < 0 ? 0 : current.index + 1;

  return (
    <div className={styles.section}>
      <h3 className={styles.heading}>{t('term_search.title')}</h3>
      <TextInput
        type="search"
        value={term}
        placeholder={t('term_search.placeholder')}
        aria-label={t('term_search.label')}
        leadingIcon={<Search size={16} strokeWidth={1.75} />}
        autoFocus
        onChange={(event) => {
          setTerm(event.target.value);
          search(event.target.value, 'next');
        }}
        onKeyDown={onKeyDown}
        data-testid="terminal-search-input"
      />
      <div className={styles.searchRow}>
        <span
          className={styles.muted}
          role="status"
          data-testid="terminal-search-results"
          data-index={index}
          data-count={count}
        >
          {term === '' ? '' : count === 0 ? t('term_search.no_results') : t('term_search.results', { index, count })}
        </span>
        <IconButton
          size="sm"
          aria-label={t('term_search.previous')}
          icon={<ChevronLeft size={16} strokeWidth={1.75} />}
          disabled={term === ''}
          onClick={() => search(term, 'previous')}
          data-testid="terminal-search-prev"
        />
        <IconButton
          size="sm"
          aria-label={t('term_search.next')}
          icon={<ChevronRight size={16} strokeWidth={1.75} />}
          disabled={term === ''}
          onClick={() => search(term, 'next')}
          data-testid="terminal-search-next"
        />
      </div>
      {isVisible('terminal-autocomplete') ? (
        <div className={styles.row} data-testid="terminal-autocomplete">
          {t('term_sidebar.autocomplete')}
        </div>
      ) : null}
    </div>
  );
}

// --- History ---------------------------------------------------------------------

function HistoryTab({ paneId, session }: TabProps) {
  const host = historyHost(session);
  const hostKey = host === null ? '' : `${host.vault_id}/${host.record_id}`;
  const [entries, setEntries] = useState<readonly HistoryEntryView[] | null>(null);
  const [query, setQuery] = useState('');
  const [nonce, setNonce] = useState(0);
  const [clearing, setClearing] = useState(false);
  const hasSession = session !== undefined;

  useEffect(() => {
    if (!hasSession) {
      return undefined;
    }
    let active = true;
    const [vault_id, record_id] = hostKey === '' ? [] : hostKey.split('/');
    historyList({ host: vault_id === undefined || record_id === undefined ? null : { vault_id, record_id }, limit: 500 })
      .then((list) => {
        if (active) {
          setEntries([...list].sort((a, b) => b.executed_at.localeCompare(a.executed_at)));
        }
      })
      .catch(() => {
        if (active) {
          setEntries([]);
        }
      });
    return () => {
      active = false;
    };
  }, [hostKey, nonce, hasSession]);

  useEffect(() => {
    let stop: (() => void) | null = null;
    let disposed = false;
    subscribe('records:changed', (event) => {
      if (event.classes.includes(CLASS_HISTORY)) {
        setNonce((value) => value + 1);
      }
    })
      .then((unlisten) => {
        if (disposed) {
          unlisten();
        } else {
          stop = unlisten;
        }
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      stop?.();
    };
  }, []);

  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return (entries ?? []).filter((entry) => needle === '' || entry.command.toLowerCase().includes(needle));
  }, [entries, query]);
  const frequent = useMemo(() => frequentCommands(entries ?? []), [entries]);

  const insert = (command: string) => {
    if (paneId !== null) {
      paneController(paneId)?.insert(command);
    }
  };

  const remove = (entry: HistoryEntryView) => {
    historyDelete({ vault_id: entry.vault_id, record_ids: [entry.record_id] })
      .then(() => setNonce((value) => value + 1))
      .catch(showErrorToast);
  };

  const clear = () => {
    setClearing(false);
    historyClear({ host })
      .then(() => setNonce((value) => value + 1))
      .catch(showErrorToast);
  };

  if (!hasSession) {
    return (
      <div className={styles.section}>
        <p className={styles.muted} data-testid="history-empty">
          {t('term_history.no_session')}
        </p>
      </div>
    );
  }

  return (
    <div className={styles.section}>
      <TextInput
        type="search"
        value={query}
        placeholder={t('term_history.search_placeholder')}
        aria-label={t('term_history.search_label')}
        leadingIcon={<Search size={16} strokeWidth={1.75} />}
        onChange={(event) => setQuery(event.target.value)}
        data-testid="history-search-input"
      />
      {isVisible('history-snippet-hint') && frequent.length > 0 ? (
        <div className={styles.card} data-testid="history-snippet-hint">
          <p className={styles.cardTitle}>{t('term_history.hint_title')}</p>
          <ul className={styles.hintList}>
            {frequent.map((command) => (
              <li key={command} className={styles.mono}>
                {command}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      {entries !== null && visible.length === 0 ? (
        <p className={styles.muted} data-testid="history-empty">
          {entries.length === 0 ? t('term_history.empty') : t('term_history.no_results')}
        </p>
      ) : null}
      {visible.length > 0 ? (
        <ul className={styles.list} aria-label={t('term_history.list_label')} data-testid="history-list">
          {visible.map((entry) => (
            <li key={entry.record_id} className={styles.historyItem} data-testid="history-item" data-record-id={entry.record_id}>
              <button
                type="button"
                className={styles.historyCommand}
                title={t('term_history.insert_hint')}
                onClick={() => insert(entry.command)}
                data-testid="history-item-command"
              >
                {entry.command}
              </button>
              <IconButton
                size="sm"
                aria-label={t('term_history.delete')}
                icon={<X size={14} strokeWidth={1.75} />}
                onClick={() => remove(entry)}
                data-testid="history-item-delete"
              />
            </li>
          ))}
        </ul>
      ) : null}
      <Button
        variant="text"
        icon={<Trash2 size={16} strokeWidth={1.75} />}
        disabled={entries === null || entries.length === 0}
        onClick={() => setClearing(true)}
        data-testid="history-clear"
      >
        {t('term_history.clear')}
      </Button>
      <Modal
        open={clearing}
        title={t('term_history.clear_title')}
        onClose={() => setClearing(false)}
        closeLabel={t('common.close')}
        size="small"
        data-testid="history-clear-dialog"
        footer={
          <>
            <Button onClick={() => setClearing(false)} data-testid="history-clear-dialog-cancel">
              {t('term_history.cancel')}
            </Button>
            <Button variant="danger" onClick={clear} data-testid="history-clear-dialog-confirm">
              {t('term_history.clear_confirm')}
            </Button>
          </>
        }
      >
        <p className={styles.dialogText}>
          {host === null ? t('term_history.clear_text_none') : t('term_history.clear_text_host', { host: session.title })}
        </p>
      </Modal>
    </div>
  );
}

// --- Appearance ------------------------------------------------------------------

async function saveSettings(values: Record<string, JsonValue>): Promise<void> {
  const next = await settingsSet({ values });
  settingsStore.set(next);
}

function ThemePreview({ theme }: { theme: BuiltinTheme }) {
  const colors = theme.theme;
  return (
    <span
      className={styles.preview}
      style={{ background: colors.background, color: colors.foreground, borderColor: colors.ansi.brightBlack }}
      aria-hidden="true"
      data-testid="appearance-theme-option-preview"
    >
      <span className={styles.previewText}>$ ls</span>
      <span className={styles.previewSwatches}>
        {[colors.ansi.red, colors.ansi.green, colors.ansi.yellow, colors.ansi.blue, colors.ansi.magenta, colors.ansi.cyan].map(
          (color, index) => (
            <span key={index} className={styles.previewSwatch} style={{ background: color }} />
          ),
        )}
      </span>
    </span>
  );
}

function FontPanel({ onBack }: { onBack(): void }) {
  const settings = useTerminalSettings();
  const [family, setFamily] = useState(settings.fontFamily);
  const [familyError, setFamilyError] = useState<string | null>(null);
  const [sizeError, setSizeError] = useState<string | null>(null);
  const [lineError, setLineError] = useState<string | null>(null);
  const [sizeText, setSizeText] = useState(String(settings.fontSize));
  const [lineText, setLineText] = useState(settings.lineHeight.toFixed(1));

  const fieldError = (reason: unknown) => {
    const error = toIpcError(reason);
    return error.code === 'validation_failed' ? t('term_appearance.invalid') : t(`errors.${error.code}`);
  };

  const saveFamily = (event?: FormEvent) => {
    event?.preventDefault();
    const value = family.trim();
    if (value === '' || value.length > 256) {
      setFamilyError(t('term_appearance.family_invalid'));
      return;
    }
    setFamilyError(null);
    if (value !== settings.fontFamily) {
      saveSettings({ 'terminal.font_family': value }).catch((reason) => setFamilyError(fieldError(reason)));
    }
  };

  const saveSize = (value: number) => {
    if (!Number.isInteger(value) || value < FONT_SIZE_MIN || value > FONT_SIZE_MAX) {
      setSizeError(t('term_appearance.size_range', { min: FONT_SIZE_MIN, max: FONT_SIZE_MAX }));
      return;
    }
    setSizeError(null);
    setSizeText(String(value));
    saveSettings({ 'terminal.font_size': value }).catch((reason) => setSizeError(fieldError(reason)));
  };

  const saveLineHeight = () => {
    const value = Math.round(Number(lineText) * 10) / 10;
    if (!Number.isFinite(value) || value < LINE_HEIGHT_MIN || value > LINE_HEIGHT_MAX) {
      setLineError(t('term_appearance.line_height_range', { min: LINE_HEIGHT_MIN.toFixed(1), max: LINE_HEIGHT_MAX.toFixed(1) }));
      return;
    }
    setLineError(null);
    setLineText(value.toFixed(1));
    saveSettings({ 'terminal.line_height': value }).catch((reason) => setLineError(fieldError(reason)));
  };

  return (
    <div className={styles.section} data-testid="appearance-font-panel">
      <button type="button" className={styles.back} onClick={onBack} data-testid="appearance-font-back">
        <ChevronLeft size={16} strokeWidth={1.75} aria-hidden="true" />
        {t('term_appearance.font')}
      </button>
      <form onSubmit={saveFamily}>
        <TextInput
          label={t('term_appearance.font_family')}
          value={family}
          placeholder={DEFAULT_FONT_FAMILY}
          error={familyError ?? undefined}
          onChange={(event) => setFamily(event.target.value)}
          onBlur={() => saveFamily()}
          data-testid="appearance-font-family"
        />
      </form>
      <div className={styles.sizeRow}>
        <TextInput
          type="number"
          label={t('term_appearance.font_size')}
          value={sizeText}
          min={FONT_SIZE_MIN}
          max={FONT_SIZE_MAX}
          step={1}
          error={sizeError ?? undefined}
          onChange={(event) => setSizeText(event.target.value)}
          onBlur={() => saveSize(Number(sizeText))}
          onKeyDown={(event) => {
            if (event.key === 'Enter') {
              saveSize(Number(sizeText));
            }
          }}
          data-testid="appearance-font-size"
        />
        <IconButton
          variant="secondary"
          aria-label={t('term_appearance.decrease')}
          icon={<Minus size={16} strokeWidth={1.75} />}
          disabled={settings.fontSize <= FONT_SIZE_MIN}
          onClick={() => saveSize(settings.fontSize - 1)}
          data-testid="appearance-font-size-decrease"
        />
        <IconButton
          variant="secondary"
          aria-label={t('term_appearance.increase')}
          icon={<Plus size={16} strokeWidth={1.75} />}
          disabled={settings.fontSize >= FONT_SIZE_MAX}
          onClick={() => saveSize(settings.fontSize + 1)}
          data-testid="appearance-font-size-increase"
        />
      </div>
      <TextInput
        type="number"
        label={t('term_appearance.line_height')}
        value={lineText}
        min={LINE_HEIGHT_MIN}
        max={LINE_HEIGHT_MAX}
        step={0.1}
        error={lineError ?? undefined}
        onChange={(event) => setLineText(event.target.value)}
        onBlur={saveLineHeight}
        onKeyDown={(event) => {
          if (event.key === 'Enter') {
            saveLineHeight();
          }
        }}
        data-testid="appearance-line-height"
      />
    </div>
  );
}

function ThemeList({ session }: { session: SessionInfo }) {
  const current = useSessionTheme(session.session_id, session.terminal_theme);
  const vaults = useVaults();

  const choose = (theme: BuiltinTheme) => {
    const ref = { builtin: theme.slug };
    setSessionTheme(session.session_id, ref);
    const target = session.target;
    if (target?.kind !== 'host') {
      return;
    }
    const vault = vaults.find((candidate) => candidate.vault_id === target.vault_id);
    if (vault !== undefined && vault.kind !== 'personal') {
      return;
    }
    recordGet({ vault_id: target.vault_id, record_id: target.record_id })
      .then((view) => {
        if (view.type !== 'host') {
          return undefined;
        }
        return recordSave({
          vault_id: view.vault_id,
          record_id: view.record_id,
          type: 'host',
          data: { ...view.data, terminal_theme: ref },
        });
      })
      .catch(showErrorToast);
  };

  return (
    <div className={styles.themes} role="listbox" aria-label={t('term_appearance.themes')} data-testid="appearance-theme-list">
      {BUILTIN_THEMES.map((theme) => {
        const selected = theme.slug === current.slug;
        return (
          <button
            key={theme.slug}
            type="button"
            role="option"
            aria-selected={selected}
            className={styles.themeOption}
            onClick={() => choose(theme)}
            data-testid="appearance-theme-option"
            data-theme={theme.slug}
          >
            <ThemePreview theme={theme} />
            <span className={styles.themeName}>{theme.theme.name}</span>
          </button>
        );
      })}
    </div>
  );
}

function AppearanceTab({ session }: TabProps) {
  const [fontPanel, setFontPanel] = useState(false);
  const settings = useTerminalSettings();
  if (fontPanel) {
    return <FontPanel onBack={() => setFontPanel(false)} />;
  }
  return (
    <div className={styles.section}>
      <button type="button" className={styles.row} onClick={() => setFontPanel(true)} data-testid="appearance-font">
        <span>{t('term_appearance.font')}</span>
        <span className={styles.rowValue}>
          {t('term_appearance.font_value', { family: settings.fontFamily, size: settings.fontSize })}
        </span>
        <ChevronRight size={16} strokeWidth={1.75} aria-hidden="true" />
      </button>
      <h3 className={styles.heading}>{t('term_appearance.themes')}</h3>
      {session === undefined ? (
        <p className={styles.muted}>{t('term_history.no_session')}</p>
      ) : (
        <ThemeList session={session} />
      )}
    </div>
  );
}

// --- Sidebar ---------------------------------------------------------------------

/** UI 4.2 order: Tools, Snippets (stage 5), History, Appearance. */
const TABS: readonly { id: SidebarTab; icon: typeof Wrench; label: string; feature?: 'terminal-sidebar-snippets' }[] = [
  { id: 'tools', icon: Wrench, label: 'term_sidebar.tools' },
  { id: 'snippets', icon: Code, label: 'term_sidebar.snippets', feature: 'terminal-sidebar-snippets' },
  { id: 'history', icon: History, label: 'term_sidebar.history' },
  { id: 'appearance', icon: Palette, label: 'term_sidebar.appearance' },
];

export interface TerminalSidebarProps {
  tab: SidebarTab;
  paneId: PaneId | null;
  session: SessionInfo | undefined;
}

export function TerminalSidebar({ tab, paneId, session }: TerminalSidebarProps) {
  const select = (id: SidebarTab) => workspaceStore.set((current) => (current.sidebarTab === id ? current : { ...current, sidebarTab: id }));
  return (
    <aside className={styles.sidebar} aria-label={t('term_sidebar.label')} data-testid="terminal-sidebar" data-tab={tab}>
      <div className={styles.tabs} role="tablist" aria-label={t('term_sidebar.label')}>
        {TABS.filter(({ feature }) => feature === undefined || isVisible(feature)).map(({ id, icon: Icon, label }) => (
          <button
            key={id}
            type="button"
            role="tab"
            aria-selected={tab === id}
            aria-label={t(label)}
            title={t(label)}
            className={styles.tab}
            onClick={() => select(id)}
            data-testid={`terminal-sidebar-tab-${id}`}
          >
            <Icon size={16} strokeWidth={1.75} aria-hidden="true" />
          </button>
        ))}
      </div>
      <div className={styles.body} role="tabpanel">
        {tab === 'tools' ? <ToolsTab key={paneId ?? 'none'} paneId={paneId} session={session} /> : null}
        {tab === 'history' ? <HistoryTab paneId={paneId} session={session} /> : null}
        {tab === 'appearance' ? <AppearanceTab paneId={paneId} session={session} /> : null}
      </div>
    </aside>
  );
}
