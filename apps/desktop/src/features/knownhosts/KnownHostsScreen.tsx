// Known Hosts screen (UI 10.1): action bar (Import and Export hidden until
// stage 3; search, view toggle, sort), cards, details panel, context menu,
// Remove. Data: `known_hosts_list`, refreshed on `records:changed` with class
// 8 (known_hosts) and class 2 (hosts: titles and "saved hosts with this
// address").
//
// Owner: S7 (skeleton by the lead).

import { Copy, Fingerprint, LayoutGrid, List, Search, ShieldCheck, Trash2, X } from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent } from 'react';
import { ContextMenu } from '../../app/ContextMenu/ContextMenu';
import { selectRequestStore, updateView } from '../../app/store';
import { showToast } from '../../app/toast';
import { Button, HostCard, IconButton, Modal, SegmentedControl, TextInput } from '../../components';
import {
  clipboardWriteText,
  isIpcError,
  knownHostRemove,
  knownHostsList,
  recordsQuery,
  subscribe,
  type HostView,
  type KnownHostView,
  type SettingsValues,
  type Uuid,
  type VaultView,
} from '../../ipc';
import { t } from '../../i18n';
import { SortMenu } from '../hosts/ActionBar';
import { showErrorToast } from '../hosts/actions';
import { hostTitle } from '../hosts/format';
import { useSettingValue } from '../hosts/useSettingValue';
import { KnownHostPanel } from './KnownHostPanel';
import {
  filterKnownHosts,
  KNOWN_HOSTS_LAYOUTS,
  KNOWN_HOSTS_SORTS,
  knownHostTitle,
  sortKnownHosts,
  type KnownHostsLayout,
  type KnownHostsSort,
} from './model';
import hostsStyles from '../hosts/HostsScreen.module.css';
import styles from './knownHosts.module.css';

export interface KnownHostsScreenProps {
  vault: VaultView;
  settings: SettingsValues;
  /** The screen is visible and no modal covers it (keyboard handling on). */
  active: boolean;
}

/** Record class codes (docs/data-model.md, section 2.2). */
const CLASS_HOSTS = 0x02;
const CLASS_KNOWN_HOSTS = 0x08;
const RELOAD_DEBOUNCE_MS = 50;

interface ScreenData {
  entries: readonly KnownHostView[] | null;
  hostTitles: ReadonlyMap<Uuid, string>;
}

function reportError(reason: unknown): void {
  // A lock unmounts the screen; running queries fail with `locked`.
  if (!isIpcError(reason, 'locked') && import.meta.env.DEV) {
    console.warn('known hosts query failed', isIpcError(reason) ? reason.code : 'unknown');
  }
}

function useKnownHostsData(vaultId: Uuid): ScreenData {
  const [entries, setEntries] = useState<readonly KnownHostView[] | null>(null);
  const [hostTitles, setHostTitles] = useState<ReadonlyMap<Uuid, string>>(new Map());
  const [entriesKey, setEntriesKey] = useState(0);
  const [hostsKey, setHostsKey] = useState(0);

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    let reloadHosts = false;
    void subscribe('records:changed', (event) => {
      if (event.vault_id !== vaultId) {
        return;
      }
      const hosts = event.classes.includes(CLASS_HOSTS);
      if (!hosts && !event.classes.includes(CLASS_KNOWN_HOSTS)) {
        return;
      }
      reloadHosts = reloadHosts || hosts;
      clearTimeout(timer);
      timer = setTimeout(() => {
        setEntriesKey((key) => key + 1);
        if (reloadHosts) {
          reloadHosts = false;
          setHostsKey((key) => key + 1);
        }
      }, RELOAD_DEBOUNCE_MS);
    }).then((stop) => {
      if (disposed) {
        stop();
      } else {
        unlisten = stop;
      }
    });
    return () => {
      disposed = true;
      clearTimeout(timer);
      unlisten?.();
    };
  }, [vaultId]);

  useEffect(() => {
    let cancelled = false;
    knownHostsList().then((list) => {
      if (!cancelled) {
        setEntries(list.filter((entry) => entry.vault_id === vaultId));
      }
    }, reportError);
    return () => {
      cancelled = true;
    };
  }, [vaultId, entriesKey]);

  useEffect(() => {
    let cancelled = false;
    recordsQuery({ vault_id: vaultId, types: ['host'], scope: { kind: 'all' }, sort: 'manual' }).then((records) => {
      if (!cancelled) {
        setHostTitles(
          new Map(
            records.filter((record): record is HostView => record.type === 'host').map((host) => [host.record_id, hostTitle(host)]),
          ),
        );
      }
    }, reportError);
    return () => {
      cancelled = true;
    };
  }, [vaultId, hostsKey]);

  return { entries, hostTitles };
}

interface MenuState {
  entry: KnownHostView;
  at: { x: number; y: number };
}

export function KnownHostsScreen({ vault, settings, active }: KnownHostsScreenProps) {
  const vaultId = vault.vault_id;
  const { entries, hostTitles } = useKnownHostsData(vaultId);
  const [search, setSearch] = useState('');
  const [selectedId, setSelectedId] = useState<Uuid | null>(null);
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [toRemove, setToRemove] = useState<KnownHostView | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  const [layout, setLayout] = useSettingValue<KnownHostsLayout>(settings, 'view.known_hosts.layout', KNOWN_HOSTS_LAYOUTS, 'grid');
  const [sort, setSort] = useSettingValue<KnownHostsSort>(settings, 'view.known_hosts.sort', KNOWN_HOSTS_SORTS, 'name');

  const shown = useMemo(() => sortKnownHosts(filterKnownHosts(entries ?? [], search), sort), [entries, search, sort]);
  const selected = selectedId === null ? undefined : entries?.find((entry) => entry.record_id === selectedId);

  const copyFingerprint = useCallback((entry: KnownHostView) => {
    clipboardWriteText(entry.fingerprint).then(
      () => showToast({ message: t('known_hosts.toast.fingerprint_copied'), level: 'success', testId: 'known-hosts-fingerprint-copied-toast' }),
      showErrorToast,
    );
  }, []);

  const confirmRemove = async () => {
    const entry = toRemove;
    setToRemove(null);
    if (entry === null) {
      return;
    }
    try {
      await knownHostRemove({ vault_id: vaultId, record_ids: [entry.record_id] });
      if (selectedId === entry.record_id) {
        setSelectedId(null);
      }
      showToast({ message: t('known_hosts.toast.removed'), level: 'info', testId: 'known-hosts-removed-toast' });
    } catch (reason) {
      showErrorToast(reason);
    }
  };

  const openHost = (recordId: Uuid) => {
    const current = selectRequestStore.get();
    selectRequestStore.set({ record_id: recordId, nonce: (current?.nonce ?? 0) + 1 });
    updateView({ vaultsSection: 'hosts' });
  };

  const openMenu = (event: MouseEvent, entry: KnownHostView) => {
    event.preventDefault();
    setSelectedId(entry.record_id);
    setMenu({ entry, at: { x: event.clientX, y: event.clientY } });
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (!active || event.defaultPrevented) {
      return;
    }
    if (event.key === 'Escape' && selectedId !== null && !(event.target instanceof HTMLInputElement)) {
      event.preventDefault();
      setSelectedId(null);
    } else if (event.key === 'Delete' && selected !== undefined && !(event.target instanceof HTMLInputElement)) {
      event.preventDefault();
      setToRemove(selected);
    }
  };

  let content;
  if (entries === null) {
    content = null;
  } else if (entries.length === 0) {
    content = (
      <div className={hostsStyles.empty} data-testid="known-hosts-empty">
        <span className={hostsStyles.emptyIcon} aria-hidden="true">
          <ShieldCheck size={24} strokeWidth={1.75} />
        </span>
        <h2 className={hostsStyles.emptyTitle}>{t('known_hosts.empty.title')}</h2>
        <p className={hostsStyles.emptyText}>{t('known_hosts.empty.text')}</p>
      </div>
    );
  } else if (shown.length === 0) {
    content = (
      <p className={hostsStyles.emptyLine} data-testid="known-hosts-no-results">
        {t('known_hosts.no_results')}
      </p>
    );
  } else {
    content = (
      <div role="listbox" aria-label={t('known_hosts.list')} className={layout === 'list' ? hostsStyles.list : hostsStyles.grid}>
        {shown.map((entry) => {
          const isSelected = entry.record_id === selectedId;
          const title = knownHostTitle(entry);
          return (
            <HostCard
              key={entry.record_id}
              label={title}
              address={title}
              subtitle={entry.key_type}
              icon={Fingerprint}
              selected={isSelected}
              onSelect={() => setSelectedId(entry.record_id)}
              onOpen={() => setSelectedId(entry.record_id)}
              onContextMenu={(event) => openMenu(event, entry)}
              trailing={
                entry.revoked ? (
                  <span className={styles.revokedBadge} data-testid="known-host-card-revoked-badge">
                    {t('known_hosts.revoked')}
                  </span>
                ) : undefined
              }
              data-record-id={entry.record_id}
              data-hashed={entry.hashed ? 'true' : 'false'}
              data-revoked={entry.revoked ? 'true' : 'false'}
              data-selected={isSelected ? 'true' : 'false'}
              data-testid="known-host-card"
            />
          );
        })}
      </div>
    );
  }

  return (
    <section
      className={hostsStyles.root}
      aria-label={t('known_hosts.screen.label')}
      data-vault-id={vaultId}
      data-ready={entries === null ? 'false' : 'true'}
      data-layout={layout}
      data-sort={sort}
      onKeyDown={handleKeyDown}
      data-testid="known-hosts-screen"
    >
      <div className={hostsStyles.main}>
        <div className={hostsStyles.header}>
          <div className={hostsStyles.actionBar}>
            <div className={hostsStyles.actionGroup}>
              {/* `known-hosts-import` and `known-hosts-export` (stage 3: the import dialog and file dialogs) are hidden. */}
            </div>
            {/* UI 10.1: search, view toggle and sorting on the right of the action bar. */}
            <div className={hostsStyles.actionGroup}>
              <div className={styles.barSearch} role="search">
                <TextInput
                  ref={searchRef}
                  className={styles.barSearchField}
                  value={search}
                  placeholder={t('known_hosts.search.placeholder')}
                  aria-label={t('known_hosts.search.label')}
                  autoComplete="off"
                  spellCheck={false}
                  leadingIcon={<Search />}
                  trailing={
                    search === '' ? undefined : (
                      <IconButton
                        size="sm"
                        icon={<X />}
                        aria-label={t('known_hosts.search.clear')}
                        onClick={() => {
                          setSearch('');
                          searchRef.current?.focus();
                        }}
                        data-testid="known-hosts-search-clear"
                      />
                    )
                  }
                  onChange={(event) => setSearch(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === 'Escape' && search !== '') {
                      event.preventDefault();
                      event.stopPropagation();
                      setSearch('');
                    }
                  }}
                  data-testid="known-hosts-search-input"
                />
              </div>
              <SegmentedControl<KnownHostsLayout>
                aria-label={t('known_hosts.view.label')}
                value={layout}
                onValueChange={setLayout}
                options={[
                  { value: 'grid', icon: <LayoutGrid />, 'aria-label': t('known_hosts.view.grid') },
                  { value: 'list', icon: <List />, 'aria-label': t('known_hosts.view.list') },
                ]}
                data-testid="known-hosts-view-toggle"
              />
              <SortMenu<KnownHostsSort> value={sort} onChange={setSort} modes={KNOWN_HOSTS_SORTS} testIdPrefix="known-hosts-sort" />
            </div>
          </div>
        </div>
        <div className={hostsStyles.content}>{content}</div>
      </div>

      {selected !== undefined ? (
        <KnownHostPanel
          key={selected.record_id}
          entry={selected}
          hostTitles={hostTitles}
          onCopyFingerprint={copyFingerprint}
          onRemove={setToRemove}
          onOpenHost={openHost}
          onClose={() => setSelectedId(null)}
        />
      ) : null}

      {menu !== null ? (
        <ContextMenu
          items={[
            {
              id: 'copy-fingerprint',
              label: t('known_hosts.menu.copy_fingerprint'),
              icon: Copy,
              onSelect: () => copyFingerprint(menu.entry),
              'data-testid': 'known-host-menu-copy-fingerprint',
            },
            { type: 'separator', id: 'separator' },
            {
              id: 'remove',
              label: t('known_hosts.menu.remove'),
              icon: Trash2,
              danger: true,
              onSelect: () => setToRemove(menu.entry),
              'data-testid': 'known-host-menu-remove',
            },
          ]}
          at={menu.at}
          onClose={() => setMenu(null)}
          data-testid="known-host-context-menu"
        />
      ) : null}

      <Modal
        open={toRemove !== null}
        size="small"
        title={t('known_hosts.remove_dialog.title')}
        closeLabel={t('common.close')}
        onClose={() => setToRemove(null)}
        data-testid="known-host-remove-dialog"
        footer={
          <>
            <Button onClick={() => setToRemove(null)} data-testid="known-host-remove-dialog-cancel">
              {t('common.cancel')}
            </Button>
            <Button variant="danger" onClick={() => void confirmRemove()} data-testid="known-host-remove-dialog-confirm">
              {t('known_hosts.remove_dialog.confirm')}
            </Button>
          </>
        }
      >
        <p className={hostsStyles.dialogText}>
          {t('known_hosts.remove_dialog.text', { address: toRemove === null ? '' : knownHostTitle(toRemove) })}
        </p>
        <p className={hostsStyles.dialogHint}>{t('known_hosts.remove_dialog.hint')}</p>
      </Modal>
    </section>
  );
}
