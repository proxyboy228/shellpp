// Host selection shared by the new tab page, empty split panes (UI 4.7) and
// the Ctrl+K search palette: a search field (focused on mount), Recent
// connections while the field is empty, results (open tabs first, then
// hosts) while typing; arrows and Enter work like clicks (UI 6). Alt with a
// click or Enter opens a host in a new window (UI 2.4).
//
// Test ids: `${testIdPrefix}-search-input`, `-search-hint`, `-recent`,
// `-recent-item`, `-recent-empty`, `-results`, `-result-tab`, `-result-host`,
// `-no-results` (stage 2 plan, section 7.5).
//
// Owner: S7 (skeleton by the lead).

import { History, Search, SquareTerminal } from 'lucide-react';
import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent, type ReactNode } from 'react';
import { modShortcut } from '../../app/platform';
import { useOs } from '../../app/store';
import { OsIcon, TextInput } from '../../components';
import type { SessionTarget } from '../../ipc';
import { t } from '../../i18n';
import { connectionAddress } from '../connect/format';
import { effectiveUsername, hostTitle } from '../hosts/format';
import { useTabSummaries } from '../terminal';
import { pickerItems, type PickerItem } from './pickerModel';
import { usePickerData } from './usePickerData';
import styles from './newtab.module.css';

export interface HostPickerProps {
  /** Prefix of the test ids inside: `newtab`, `pane-host-picker` or `search-palette`. */
  testIdPrefix: string;
  /** Focus the search field on mount (default true). */
  autoFocus?: boolean;
  /** List open tabs in results (new tab page and Ctrl+K: true; split pane: false). */
  includeTabs?: boolean;
  /** A host or a recent connection was chosen; `altKey` = open in a new window. */
  onPickHost(target: SessionTarget, event: { altKey: boolean }): void;
  /** An open tab was chosen. */
  onPickTab?(tabId: string): void;
  /** Escape (closes the palette). */
  onCancel?(): void;
  /** Buttons in the Recent connections header (the new tab page's Restore). */
  recentActions?: ReactNode;
  /**
   * The picker is visible (default true). When it becomes true the search
   * field takes focus (with `autoFocus`) and Recent connections are reloaded.
   */
  active?: boolean;
}

export function HostPicker({
  testIdPrefix: P,
  autoFocus = true,
  includeTabs = false,
  onPickHost,
  onPickTab,
  onCancel,
  recentActions,
  active = true,
}: HostPickerProps) {
  const os = useOs();
  const tabs = useTabSummaries();
  const [query, setQuery] = useState('');
  const [cursor, setCursor] = useState(0);
  const [refreshKey, setRefreshKey] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listId = useId();
  const data = usePickerData(refreshKey);

  const items = useMemo(
    () => pickerItems({ query, includeTabs, recent: data.recent, tabs, hosts: data.hosts }),
    [query, includeTabs, data.recent, data.hosts, tabs],
  );
  const current = items.length === 0 ? -1 : Math.min(cursor, items.length - 1);

  // Fresh Recent connections whenever the picker is shown again.
  const [shown, setShown] = useState(active);
  if (shown !== active) {
    setShown(active);
    if (active) {
      setRefreshKey((key) => key + 1);
    }
  }

  // Focus goes to the search field when the picker is shown.
  useEffect(() => {
    if (active && autoFocus) {
      inputRef.current?.focus();
    }
  }, [active, autoFocus]);

  const pick = (item: PickerItem, altKey: boolean) => {
    if (item.kind === 'tab') {
      onPickTab?.(item.tab.id);
    } else {
      onPickHost(item.target, { altKey });
    }
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      if (items.length === 0) {
        return;
      }
      event.preventDefault();
      const step = event.key === 'ArrowDown' ? 1 : -1;
      setCursor((current < 0 ? 0 : current + step + items.length) % items.length);
    } else if (event.key === 'Enter') {
      const item = current < 0 ? undefined : items[current];
      if (item !== undefined) {
        event.preventDefault();
        pick(item, event.altKey);
      }
    } else if (event.key === 'Escape') {
      if (onCancel !== undefined) {
        event.preventDefault();
        onCancel();
      } else if (query !== '') {
        event.preventDefault();
        event.stopPropagation();
        setQuery('');
        setCursor(0);
      }
    }
  };

  const optionId = (index: number) => `${listId}-option-${index}`;
  const itemProps = (item: PickerItem, index: number) => ({
    id: optionId(index),
    role: 'option' as const,
    'aria-selected': index === current,
    className: styles.item,
    onMouseDown: (event: MouseEvent) => event.preventDefault(),
    onMouseMove: () => {
      if (index !== current) {
        setCursor(index);
      }
    },
    onClick: (event: MouseEvent) => pick(item, event.altKey),
  });

  const searching = query.trim() !== '';
  let list: ReactNode;
  if (!searching) {
    list = (
      <section className={styles.block} aria-labelledby={`${listId}-recent-title`} data-testid={`${P}-recent`}>
        <div className={styles.blockHeader}>
          <h2 id={`${listId}-recent-title`} className={styles.blockTitle}>
            {t('recent.title')}
          </h2>
          {recentActions === undefined ? null : <div className={styles.blockActions}>{recentActions}</div>}
        </div>
        {data.ready && items.length === 0 ? (
          <p className={styles.empty} data-testid={`${P}-recent-empty`}>
            {t('recent.empty')}
          </p>
        ) : (
          <ul id={listId} role="listbox" aria-label={t('recent.title')} className={styles.list}>
            {items.map((item, index) =>
              item.kind === 'recent' ? (
                <li
                  key={item.key}
                  {...itemProps(item, index)}
                  data-host-id={item.row.host?.record_id}
                  data-address={item.row.host === undefined ? item.row.address : undefined}
                  data-testid={`${P}-recent-item`}
                >
                  <OsIcon os={item.row.os} size={16} className={styles.itemIcon} data-testid={`${P}-recent-item-os-icon`} />
                  <span className={styles.itemTitle} data-testid={`${P}-recent-item-title`}>
                    {item.row.title}
                  </span>
                  <span className={styles.itemPath} data-testid={`${P}-recent-item-path`}>
                    {item.row.path.join(' / ')}
                  </span>
                </li>
              ) : null,
            )}
          </ul>
        )}
      </section>
    );
  } else if (items.length === 0) {
    list = (
      <p className={styles.empty} data-testid={`${P}-no-results`}>
        {t('newtab.no_results')}
      </p>
    );
  } else {
    list = (
      <ul id={listId} role="listbox" aria-label={t('newtab.results')} className={styles.list} data-testid={`${P}-results`}>
        {items.map((item, index) => {
          if (item.kind === 'tab') {
            return (
              <li key={item.key} {...itemProps(item, index)} data-tab-id={item.tab.id} data-testid={`${P}-result-tab`}>
                <span className={styles.itemIcon} aria-hidden="true">
                  <SquareTerminal size={16} strokeWidth={1.75} />
                </span>
                <span className={styles.itemTitle}>{item.tab.title}</span>
                <span className={styles.itemPath}>{item.tab.address ?? ''}</span>
                <span className={styles.itemKind}>{t('newtab.open_tab')}</span>
              </li>
            );
          }
          if (item.kind === 'host') {
            const port = item.host.data.ssh?.port ?? item.host.inherited.port?.value;
            return (
              <li key={item.key} {...itemProps(item, index)} data-host-id={item.host.record_id} data-testid={`${P}-result-host`}>
                <OsIcon os={item.host.data.os} size={16} className={styles.itemIcon} />
                <span className={styles.itemTitle}>{hostTitle(item.host)}</span>
                <span className={styles.itemPath}>
                  {connectionAddress(item.host.data.address, port, effectiveUsername(item.host))}
                </span>
              </li>
            );
          }
          return null;
        })}
      </ul>
    );
  }

  return (
    <div className={styles.picker} data-testid={`${P}-picker`}>
      <TextInput
        ref={inputRef}
        className={styles.search}
        value={query}
        placeholder={t('newtab.search.placeholder')}
        aria-label={t('newtab.search.label')}
        aria-controls={listId}
        aria-activedescendant={current < 0 ? undefined : optionId(current)}
        autoComplete="off"
        spellCheck={false}
        leadingIcon={searching ? <Search /> : <History />}
        trailing={
          <kbd className={styles.hint} data-testid={`${P}-search-hint`}>
            {modShortcut(os, 'K')}
          </kbd>
        }
        onChange={(event) => {
          setQuery(event.target.value);
          setCursor(0);
        }}
        onKeyDown={handleKeyDown}
        data-testid={`${P}-search-input`}
      />
      {list}
    </div>
  );
}
