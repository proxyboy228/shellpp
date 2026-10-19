// Hosts screen (UI 2.1-2.4) with the host and group panels (UI 3) in the
// details panel.
//
// Owner: W5. The props are the W4/W5 contract (stage plan 6.5).

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent, MouseEvent as ReactMouseEvent } from 'react';
import { ContextMenu } from '../../app/ContextMenu/ContextMenu';
import { showToast } from '../../app/toast';
import { HostCard } from '../../components';
import { isIpcError, recordGet, settingsSet } from '../../ipc';
import type { GroupView, HostView, SettingsValues, SortMode, Uuid, VaultView } from '../../ipc';
import { t } from '../../i18n';
import { GroupPanel } from '../panels/GroupPanel';
import { HostPanel } from '../panels/HostPanel';
import { ActionBar, SORT_MODES, type HostsLayout } from './ActionBar';
import {
  copyHostLink,
  createGroup,
  duplicateHost,
  moveRecords,
  removeGroup,
  removeHosts,
  showErrorToast,
} from './actions';
import { Breadcrumbs } from './Breadcrumbs';
import { createDragController, type DragController, type DragOptions, type DragView } from './dnd';
import { FolderEmptyState, NoResults, VaultEmptyState } from './EmptyState';
import { collectTags, hostSubtitle, hostTitle, isFiltering, subtreeIds } from './format';
import { groupMenuEntries, hostMenuEntries } from './menus';
import { GroupRemoveDialog, HostRemoveDialog } from './RemoveDialogs';
import { SearchBar } from './SearchBar';
import { arrowTarget, clickSelection, columnsOf, isArrowKey, type ClickModifiers } from './selection';
import { useHostsData } from './useHostsData';
import {
  dropDraftPanel,
  getHostsViewState,
  newPanel,
  updateHostsViewState,
  useHostsViewState,
  type HostsViewState,
} from './viewState';
import styles from './HostsScreen.module.css';

export interface HostsScreenProps {
  /** The current vault (the personal vault in stage 1). */
  vault: VaultView;
  /** Effective settings; the screen reads `view.hosts.layout`, `view.hosts.sort`, `view.details_panel_width`. */
  settings: SettingsValues;
  /** False while another screen or tab is shown: the screen ignores keyboard shortcuts. */
  active: boolean;
  /**
   * Select a host and open its panel (a `shellpp://host/<id>` link). A new
   * `nonce` repeats the request; a missing host shows the "missing" message.
   */
  selectRequest?: { record_id: Uuid; nonce: number } | null;
}

/** Search typing is applied after 150 ms (stage plan 6.6). */
export const SEARCH_DEBOUNCE_MS = 150;

const LAYOUTS: readonly HostsLayout[] = ['grid', 'list'];

/**
 * A setting value with an optimistic local override: the new value shows at
 * once and the stored value takes over when `settings` reports a change.
 */
function useSettingValue<T extends string>(
  settings: SettingsValues,
  key: string,
  values: readonly T[],
  fallback: T,
): [T, (value: T) => void] {
  const raw = settings[key];
  const stored = values.includes(raw as T) ? (raw as T) : fallback;
  const [pending, setPending] = useState<{ value: T; stored: T } | null>(null);
  // Any change of the stored value (this window's write or another window's)
  // ends the override, so a later change back to the old stored value is not
  // hidden behind it (UI 1.4: changes appear in all windows).
  if (pending !== null && pending.stored !== stored) {
    setPending(null);
  }
  const value = pending !== null && pending.stored === stored ? pending.value : stored;
  const set = useCallback(
    (next: T) => {
      setPending({ value: next, stored });
      settingsSet({ values: { [key]: next } }).catch((reason: unknown) => {
        setPending(null);
        showErrorToast(reason);
      });
    },
    [key, stored],
  );
  return [value, set];
}

type MenuState =
  | { kind: 'host'; ids: Uuid[]; at: { x: number; y: number } }
  | { kind: 'group'; group: GroupView; at: { x: number; y: number } };

const EDITABLE_SELECTOR = 'input, textarea, select, [contenteditable="true"], [role="menu"], [role="dialog"]';

export function HostsScreen({ vault, settings, active, selectRequest = null }: HostsScreenProps) {
  const vaultId = vault.vault_id;
  const view = useHostsViewState();
  const { folderId, selectedIds, anchorId, panel, showMore } = view;

  // Search and tag filter: component state, dropped on lock (decision 19).
  const [searchInput, setSearchInput] = useState('');
  const [searchText, setSearchText] = useState('');
  const [tagFilter, setTagFilter] = useState<string[]>([]);
  const searchInputRef = useRef<HTMLInputElement>(null);

  const [layout, setLayout] = useSettingValue<HostsLayout>(settings, 'view.hosts.layout', LAYOUTS, 'grid');
  const [sort, setSort] = useSettingValue<SortMode>(settings, 'view.hosts.sort', SORT_MODES, 'manual');

  const [menu, setMenu] = useState<MenuState | null>(null);
  const [hostsToRemove, setHostsToRemove] = useState<Uuid[] | null>(null);
  const [groupToRemove, setGroupToRemove] = useState<GroupView | null>(null);
  const [focusRequest, setFocusRequest] = useState({ key: -1, nonce: 0 });
  const [createdPanelKey, setCreatedPanelKey] = useState<number | null>(null);
  const [extraGroups, setExtraGroups] = useState<readonly GroupView[]>([]);
  const [dragView, setDragView] = useState<DragView | null>(null);
  const [handledRequest, setHandledRequest] = useState<HostsScreenProps['selectRequest']>(null);

  const contentRef = useRef<HTMLDivElement>(null);

  const data = useHostsData({ vaultId, folderId, text: searchText, tags: tagFilter, sort });
  const filtering = isFiltering(searchText, tagFilter);

  // Groups created on the fly are known before the next reload.
  const groupsById = useMemo(() => {
    if (extraGroups.length === 0) {
      return data.groupsById;
    }
    const merged = new Map(data.groupsById);
    for (const group of extraGroups) {
      if (!merged.has(group.record_id)) {
        merged.set(group.record_id, group);
      }
    }
    return merged;
  }, [data.groupsById, extraGroups]);
  const allGroups = useMemo(
    () => [...groupsById.values()].sort((a, b) => a.data.name.localeCompare(b.data.name, undefined, { sensitivity: 'base' })),
    [groupsById],
  );
  const allTags = useMemo(() => collectTags(data.allHosts), [data.allHosts]);

  const hostOrder = useMemo(() => data.hosts.map((host) => host.record_id), [data.hosts]);
  const visibleSelected = useMemo(() => selectedIds.filter((id) => hostOrder.includes(id)), [selectedIds, hostOrder]);

  // ---- search -------------------------------------------------------------

  // The typed text reaches the query after a pause.
  useEffect(() => {
    if (searchInput === searchText) {
      return undefined;
    }
    const timer = setTimeout(() => setSearchText(searchInput), SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [searchInput, searchText]);

  const clearFilters = () => {
    setSearchInput('');
    setSearchText('');
    setTagFilter([]);
  };

  // A draft does not survive a lock (the screen unmounts) or a remount.
  useEffect(() => () => dropDraftPanel(), []);

  // ---- deep link (selectRequest) -------------------------------------------

  if (selectRequest !== null && selectRequest !== handledRequest && data.ready) {
    setHandledRequest(selectRequest);
    if (data.hostsById.has(selectRequest.record_id)) {
      setSearchInput('');
      setSearchText('');
      setTagFilter([]);
    }
  }

  const appliedRequestRef = useRef<HostsScreenProps['selectRequest']>(null);
  useEffect(() => {
    if (handledRequest === null || handledRequest === undefined || appliedRequestRef.current === handledRequest) {
      return;
    }
    appliedRequestRef.current = handledRequest;
    const host = data.hostsById.get(handledRequest.record_id);
    if (host === undefined) {
      showToast({ message: t('hosts.toast.link_missing'), level: 'warning', testId: 'hosts-link-missing-toast' });
      return;
    }
    const groupId = host.path.length > 0 ? (host.path[host.path.length - 1] ?? null) : null;
    updateHostsViewState((state) => ({
      folderId: groupId,
      selectedIds: [host.record_id],
      anchorId: host.record_id,
      panel:
        state.panel?.type === 'host' && state.panel.record_id === host.record_id ? state.panel : newPanel('host', host.record_id),
    }));
  }, [handledRequest, data.hostsById]);

  // ---- records removed elsewhere, a removed open folder ---------------------

  const knownRef = useRef<{ hosts: ReadonlySet<Uuid>; groups: ReadonlySet<Uuid> } | null>(null);
  useEffect(() => {
    if (!data.ready) {
      return;
    }
    const hosts = new Set(data.hostsById.keys());
    const groups = new Set(data.groupsById.keys());
    const previous = knownRef.current;
    knownRef.current = { hosts, groups };
    const exists = (id: Uuid) => hosts.has(id) || groups.has(id);
    const removed = (id: Uuid) =>
      previous === null ? !exists(id) : (previous.hosts.has(id) || previous.groups.has(id)) && !exists(id);
    const state = getHostsViewState();
    const patch: Partial<HostsViewState> = {};
    if (state.panel !== null && state.panel.record_id !== null && removed(state.panel.record_id)) {
      patch.panel = null;
    }
    const kept = state.selectedIds.filter((id) => !removed(id));
    if (kept.length !== state.selectedIds.length) {
      patch.selectedIds = kept;
      patch.anchorId = state.anchorId !== null && kept.includes(state.anchorId) ? state.anchorId : null;
    }
    updateHostsViewState(patch);
  }, [data.ready, data.hostsById, data.groupsById]);

  const lastPathRef = useRef<Uuid[]>([]);
  useEffect(() => {
    if (!data.ready || folderId === null) {
      return undefined;
    }
    const folder = data.groupsById.get(folderId);
    if (folder !== undefined) {
      lastPathRef.current = [...folder.path];
      return undefined;
    }
    // Not in the list: confirm with the core (the list may lag a new group).
    let cancelled = false;
    recordGet({ vault_id: vaultId, record_id: folderId }).catch((reason: unknown) => {
      if (cancelled || !isIpcError(reason, 'not_found')) {
        return;
      }
      const fallback = [...lastPathRef.current].reverse().find((id) => data.groupsById.has(id)) ?? null;
      updateHostsViewState({ folderId: fallback, selectedIds: [], anchorId: null });
    });
    return () => {
      cancelled = true;
    };
  }, [data.ready, data.groupsById, folderId, vaultId]);

  // ---- navigation and selection ---------------------------------------------

  const requestFocus = (key: number) => setFocusRequest((current) => ({ key, nonce: current.nonce + 1 }));

  const openFolder = (groupId: Uuid | null) => {
    clearFilters();
    updateHostsViewState({ folderId: groupId, selectedIds: [], anchorId: null, panel: null });
  };

  const openHostPanel = (state: HostsViewState, recordId: Uuid) =>
    state.panel?.type === 'host' && state.panel.record_id === recordId ? state.panel : newPanel('host', recordId);

  const clickHost = (modifiers: ClickModifiers, recordId: Uuid) => {
    updateHostsViewState((state) => {
      const next = clickSelection({ ids: state.selectedIds, anchor: state.anchorId }, recordId, modifiers, hostOrder);
      const only = next.ids.length === 1 ? next.ids[0] : undefined;
      // The panel edits one object: a multi-selection closes it (decision 18).
      const nextPanel = only === undefined ? null : openHostPanel(state, only);
      return { selectedIds: next.ids, anchorId: next.anchor, panel: nextPanel };
    });
  };

  const editHost = (recordId: Uuid) => {
    const target = openHostPanel(getHostsViewState(), recordId);
    updateHostsViewState({ selectedIds: [recordId], anchorId: recordId, panel: target });
    requestFocus(target.key);
  };

  const editGroup = (groupId: Uuid) => {
    const state = getHostsViewState();
    const target =
      state.panel?.type === 'group' && state.panel.record_id === groupId ? state.panel : newPanel('group', groupId);
    updateHostsViewState({ panel: target, selectedIds: [], anchorId: null });
    requestFocus(target.key);
  };

  const newHost = () => {
    const target = newPanel('host', null, folderId);
    updateHostsViewState({ selectedIds: [], anchorId: null, panel: target });
    requestFocus(target.key);
  };

  const newGroup = () => {
    const target = newPanel('group', null, folderId);
    updateHostsViewState({ selectedIds: [], anchorId: null, panel: target });
    requestFocus(target.key);
  };

  const closePanel = useCallback(() => updateHostsViewState({ panel: null }), []);

  const focusHostCard = (recordId: Uuid) => {
    const card = contentRef.current?.querySelector<HTMLElement>(`[data-testid="host-card"][data-record-id="${recordId}"]`);
    card?.focus();
    card?.scrollIntoView?.({ block: 'nearest' });
  };

  // ---- actions -------------------------------------------------------------

  const requestRemoveHosts = (ids: readonly Uuid[]) => {
    if (ids.length > 0) {
      setHostsToRemove([...ids]);
    }
  };

  const confirmRemoveHosts = async () => {
    const ids = hostsToRemove ?? [];
    setHostsToRemove(null);
    const deleted = await removeHosts(vaultId, ids);
    if (deleted.length > 0) {
      updateHostsViewState((state) => ({
        selectedIds: state.selectedIds.filter((id) => !deleted.includes(id)),
        panel: state.panel !== null && state.panel.record_id !== null && deleted.includes(state.panel.record_id) ? null : state.panel,
      }));
    }
  };

  const requestRemoveGroup = (groupId: Uuid) => {
    const group = groupsById.get(groupId);
    if (group !== undefined) {
      setGroupToRemove(group);
    }
  };

  const confirmRemoveGroup = async () => {
    const group = groupToRemove;
    setGroupToRemove(null);
    if (group === null) {
      return;
    }
    if (await removeGroup(vaultId, group.record_id)) {
      updateHostsViewState((state) => ({
        panel: state.panel?.record_id === group.record_id ? null : state.panel,
      }));
    }
  };

  const duplicate = async (recordId: Uuid) => {
    const copy = await duplicateHost(vaultId, recordId);
    if (copy !== null) {
      updateHostsViewState({ selectedIds: [copy.record_id], anchorId: copy.record_id, panel: newPanel('host', copy.record_id) });
    }
  };

  const move = (ids: readonly Uuid[], groupId: Uuid | null) => {
    void moveRecords(vaultId, ids, groupId);
  };

  const createGroupOnTheFly = async (name: string): Promise<Uuid | null> => {
    const group = await createGroup(vaultId, name, folderId);
    if (group === null) {
      return null;
    }
    setExtraGroups((current) => [...current, group]);
    return group.record_id;
  };

  const groupActions = {
    open: (groupId: Uuid) => openFolder(groupId),
    edit: editGroup,
    move: (groupId: Uuid, parentId: Uuid | null) => move([groupId], parentId),
    remove: requestRemoveGroup,
  };

  // ---- context menus -------------------------------------------------------

  const openHostMenu = (event: ReactMouseEvent, recordId: Uuid) => {
    event.preventDefault();
    const at = { x: event.clientX, y: event.clientY };
    const state = getHostsViewState();
    if (state.selectedIds.includes(recordId) && hostOrder.includes(recordId)) {
      setMenu({ kind: 'host', ids: visibleSelected, at });
      return;
    }
    updateHostsViewState({
      selectedIds: [recordId],
      anchorId: recordId,
      panel: state.panel?.type === 'host' && state.panel.record_id !== null ? openHostPanel(state, recordId) : state.panel,
    });
    setMenu({ kind: 'host', ids: [recordId], at });
  };

  const openGroupMenu = (event: ReactMouseEvent, group: GroupView) => {
    event.preventDefault();
    setMenu({ kind: 'group', group, at: { x: event.clientX, y: event.clientY } });
  };

  // ---- keyboard --------------------------------------------------------------

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!active || event.defaultPrevented || event.altKey) {
      return;
    }
    const target = event.target;
    if (target instanceof Element && target.closest(EDITABLE_SELECTOR) !== null) {
      return;
    }
    if (isArrowKey(event.key) && !event.ctrlKey && !event.metaKey) {
      const current = anchorId ?? visibleSelected[0] ?? null;
      const index = current === null ? -1 : hostOrder.indexOf(current);
      const cards = Array.from(contentRef.current?.querySelectorAll('[data-testid="host-card"]') ?? []);
      const next = arrowTarget(index, event.key, hostOrder.length, columnsOf(cards), layout);
      const nextId = next < 0 ? undefined : hostOrder[next];
      if (nextId === undefined) {
        return;
      }
      event.preventDefault();
      updateHostsViewState((state) => ({ selectedIds: [nextId], anchorId: nextId, panel: openHostPanel(state, nextId) }));
      focusHostCard(nextId);
    } else if ((event.key === 'e' || event.key === 'E') && !event.ctrlKey && !event.metaKey) {
      const only = visibleSelected.length === 1 ? visibleSelected[0] : undefined;
      if (only !== undefined) {
        event.preventDefault();
        editHost(only);
      }
    } else if (event.key === 'Delete') {
      if (visibleSelected.length > 0) {
        event.preventDefault();
        requestRemoveHosts(visibleSelected);
      }
    } else if (event.key === 'Escape' && panel !== null) {
      event.preventDefault();
      closePanel();
    }
    // Enter (quick connect) is stage 2 (`host-connect`).
  };

  // ---- drag and drop -------------------------------------------------------

  const dragOptions: DragOptions = {
    container: null,
    canReorder: sort === 'manual' && !filtering,
    layout,
    draggedIds: (item) =>
      item.type === 'host' && visibleSelected.includes(item.id) ? [...visibleSelected] : [item.id],
    canDropInto: (type, ids, groupId) =>
      type === 'host' || !ids.some((id) => subtreeIds(id, data.allGroups).has(groupId)),
    onDrop: (_item, ids, intent) => {
      if (intent.kind === 'into') {
        void moveRecords(vaultId, ids, intent.groupId);
      } else {
        void moveRecords(vaultId, ids, folderId, { kind: intent.position, record_id: intent.targetId });
      }
    },
  };
  const dragOptionsRef = useRef<DragOptions>(dragOptions);
  useLayoutEffect(() => {
    dragOptionsRef.current = { ...dragOptions, container: contentRef.current };
  });
  const dragRef = useRef<DragController | null>(null);
  useEffect(() => {
    const controller = createDragController({ options: () => dragOptionsRef.current, render: setDragView });
    dragRef.current = controller;
    return () => {
      controller.dispose();
      dragRef.current = null;
    };
  }, []);
  const suppressClickAfterDrag = (event: ReactMouseEvent) => {
    if (dragRef.current?.consumeClick() === true) {
      event.preventDefault();
    }
  };

  // ---- render ----------------------------------------------------------------

  const folder = folderId === null ? undefined : groupsById.get(folderId);
  const breadcrumbPath: GroupView[] =
    folder === undefined
      ? []
      : [...folder.path.map((id) => groupsById.get(id)).filter((group): group is GroupView => group !== undefined), folder];

  const nothingShown = data.groups.length === 0 && data.hosts.length === 0;
  const dropTargetId = dragView?.intent?.kind === 'into' ? dragView.intent.groupId : null;
  const dragging = new Set(dragView?.ids ?? []);
  const formatCount = (count: number) =>
    count === 1 ? t('hosts.group_card.count_one') : t('hosts.group_card.count_other', { count });

  const panelHost: HostView | undefined =
    panel?.type === 'host' && panel.record_id !== null ? data.hostsById.get(panel.record_id) : undefined;
  const panelGroup: GroupView | undefined =
    panel?.type === 'group' && panel.record_id !== null ? groupsById.get(panel.record_id) : undefined;
  const panelReady = (found: boolean) =>
    panel !== null && (panel.record_id === null || found || createdPanelKey === panel.key);

  let contentBody;
  if (!data.ready) {
    contentBody = null;
  } else if (nothingShown && filtering) {
    contentBody = <NoResults />;
  } else if (nothingShown && folderId === null) {
    contentBody = <VaultEmptyState onCreate={newHost} />;
  } else if (nothingShown) {
    contentBody = <FolderEmptyState />;
  } else {
    contentBody = (
      <>
        {data.groups.length > 0 ? (
          <section className={styles.section} aria-labelledby="hosts-groups-title" data-testid="hosts-groups-section">
            <h2 id="hosts-groups-title" className={styles.sectionTitle}>
              {t('hosts.section.groups')}
            </h2>
            <div role="listbox" aria-labelledby="hosts-groups-title" className={layout === 'list' ? styles.list : styles.grid}>
              {data.groups.map((group) => {
                const selected = panel?.type === 'group' && panel.record_id === group.record_id;
                return (
                  <HostCard
                    key={group.record_id}
                    variant="group"
                    className={styles.card}
                    label={group.data.name}
                    hostCount={group.host_count}
                    formatHostCount={formatCount}
                    color={group.data.color ?? 'graphite'}
                    selected={selected}
                    onClick={suppressClickAfterDrag}
                    onSelect={() => openFolder(group.record_id)}
                    onOpen={() => openFolder(group.record_id)}
                    onContextMenu={(event) => openGroupMenu(event, group)}
                    onPointerDown={(event) => dragRef.current?.press(event.nativeEvent, { type: 'group', id: group.record_id })}
                    data-record-id={group.record_id}
                    data-selected={selected ? 'true' : 'false'}
                    data-drop-target={dropTargetId === group.record_id ? 'true' : undefined}
                    data-dragging={dragging.has(group.record_id) ? 'true' : undefined}
                    data-testid="group-card"
                  />
                );
              })}
            </div>
          </section>
        ) : null}
        {data.hosts.length > 0 ? (
          <section className={styles.section} aria-labelledby="hosts-hosts-title" data-testid="hosts-hosts-section">
            <h2 id="hosts-hosts-title" className={styles.sectionTitle}>
              {t('hosts.section.hosts')}
            </h2>
            <div
              role="listbox"
              aria-multiselectable="true"
              aria-labelledby="hosts-hosts-title"
              className={layout === 'list' ? styles.list : styles.grid}
            >
              {data.hosts.map((host) => {
                const selected = visibleSelected.includes(host.record_id);
                return (
                  <HostCard
                    key={host.record_id}
                    className={styles.card}
                    label={host.data.label}
                    address={host.data.address}
                    subtitle={hostSubtitle(host, filtering, groupsById)}
                    color={host.data.color ?? 'graphite'}
                    selected={selected}
                    onClick={suppressClickAfterDrag}
                    onSelect={(event) =>
                      clickHost({ toggle: event.ctrlKey || event.metaKey, range: event.shiftKey }, host.record_id)
                    }
                    onOpen={(event) => {
                      // Double click connects from stage 2 (`host-connect`); in stage 1 it
                      // acts as a plain click. Enter does nothing.
                      if (event.type === 'dblclick') {
                        clickHost({ toggle: false, range: false }, host.record_id);
                      }
                    }}
                    onContextMenu={(event) => openHostMenu(event, host.record_id)}
                    onPointerDown={(event) => dragRef.current?.press(event.nativeEvent, { type: 'host', id: host.record_id })}
                    data-record-id={host.record_id}
                    data-selected={selected ? 'true' : 'false'}
                    data-dragging={dragging.has(host.record_id) ? 'true' : undefined}
                    data-testid="host-card"
                  />
                );
              })}
            </div>
          </section>
        ) : null}
      </>
    );
  }

  return (
    <section
      className={styles.root}
      aria-label={t('hosts.screen.label')}
      data-vault-id={vaultId}
      data-ready={data.ready ? 'true' : 'false'}
      data-layout={layout}
      data-sort={sort}
      data-testid="hosts-screen"
    >
      <div className={styles.main}>
        <div className={styles.header}>
          <SearchBar
            value={searchInput}
            onChange={setSearchInput}
            onClear={clearFilters}
            inputRef={searchInputRef}
          />
          <ActionBar
            onNewHost={newHost}
            onNewGroup={newGroup}
            layout={layout}
            onLayoutChange={setLayout}
            sort={sort}
            onSortChange={setSort}
            tags={allTags}
            selectedTags={tagFilter}
            onSelectedTagsChange={setTagFilter}
          />
        </div>
        <div
          ref={contentRef}
          className={styles.content}
          tabIndex={-1}
          onKeyDown={handleKeyDown}
          data-dragging={dragView === null ? undefined : 'true'}
        >
          <Breadcrumbs path={breadcrumbPath} onNavigate={openFolder} />
          {contentBody}
          {dragView?.indicator ? (
            <div
              className={styles.dropIndicator}
              data-orientation={dragView.indicator.orientation}
              style={{
                left: dragView.indicator.left,
                top: dragView.indicator.top,
                width: dragView.indicator.orientation === 'horizontal' ? dragView.indicator.width : undefined,
                height: dragView.indicator.orientation === 'vertical' ? dragView.indicator.height : undefined,
              }}
              data-testid="hosts-drop-indicator"
            />
          ) : null}
        </div>
      </div>

      {panel?.type === 'host' && panelReady(panelHost !== undefined) ? (
        <HostPanel
          key={panel.key}
          vaultId={vaultId}
          recordId={panel.record_id}
          view={panelHost}
          draftGroupId={panel.draft_parent_id}
          groups={allGroups}
          groupsById={groupsById}
          allTags={allTags}
          settings={settings}
          showMore={showMore}
          onShowMoreChange={(expanded) => updateHostsViewState({ showMore: expanded })}
          focusNonce={focusRequest.key === panel.key ? focusRequest.nonce : 0}
          onCreated={(created) => {
            setCreatedPanelKey(panel.key);
            updateHostsViewState((state) => ({
              panel: state.panel?.key === panel.key ? { ...state.panel, record_id: created.record_id } : state.panel,
              selectedIds: [created.record_id],
              anchorId: created.record_id,
            }));
          }}
          onClose={closePanel}
          onDuplicate={(recordId) => void duplicate(recordId)}
          onRemove={(recordId) => requestRemoveHosts([recordId])}
          onCreateGroup={createGroupOnTheFly}
          onMissing={closePanel}
        />
      ) : null}
      {panel?.type === 'group' && panelReady(panelGroup !== undefined) ? (
        <GroupPanel
          key={panel.key}
          vaultId={vaultId}
          recordId={panel.record_id}
          view={panelGroup}
          draftParentId={panel.draft_parent_id}
          groups={allGroups}
          groupsById={groupsById}
          focusNonce={focusRequest.key === panel.key ? focusRequest.nonce : 0}
          onCreated={(created) => {
            setCreatedPanelKey(panel.key);
            setExtraGroups((current) => [...current, created]);
            updateHostsViewState((state) => ({
              panel: state.panel?.key === panel.key ? { ...state.panel, record_id: created.record_id } : state.panel,
            }));
          }}
          onClose={closePanel}
          actions={{ open: groupActions.open, move: groupActions.move, remove: groupActions.remove }}
          onMissing={closePanel}
        />
      ) : null}

      {menu?.kind === 'host' ? (
        <ContextMenu
          items={hostMenuEntries(menu.ids, allGroups, groupsById, {
            edit: editHost,
            move,
            duplicate: (recordId) => void duplicate(recordId),
            copyLink: (recordId) => void copyHostLink(recordId),
            remove: requestRemoveHosts,
          })}
          at={menu.at}
          onClose={() => setMenu(null)}
          data-testid="host-context-menu"
        />
      ) : null}
      {menu?.kind === 'group' ? (
        <ContextMenu
          items={groupMenuEntries(
            'group-menu',
            menu.group,
            allGroups,
            groupsById,
            subtreeIds(menu.group.record_id, allGroups),
            groupActions,
          )}
          at={menu.at}
          onClose={() => setMenu(null)}
          data-testid="group-context-menu"
        />
      ) : null}

      {hostsToRemove !== null ? (
        <HostRemoveDialog
          titles={hostsToRemove.map((id) => {
            const host = data.hostsById.get(id);
            return host === undefined ? id : hostTitle(host);
          })}
          onConfirm={() => void confirmRemoveHosts()}
          onCancel={() => setHostsToRemove(null)}
        />
      ) : null}
      {groupToRemove !== null ? (
        <GroupRemoveDialog group={groupToRemove} onConfirm={() => void confirmRemoveGroup()} onCancel={() => setGroupToRemove(null)} />
      ) : null}
    </section>
  );
}
