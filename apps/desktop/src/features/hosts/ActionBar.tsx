// Action bar of the Hosts screen (UI 2.2): the New host split button with
// New Group on the left; the view toggle, the tag filter and the sort menu on
// the right. Import (stage 3), cloud integrations (stage 10), the Terminal
// (stage 2) and Serial (stage 9) buttons and the vault members (stage 8) are
// hidden.

import { ArrowDownUp, Check, FolderPlus, LayoutGrid, List, Plus, Tag } from 'lucide-react';
import { useRef, useState, type KeyboardEvent } from 'react';
import { Button, Chip, IconButton, SegmentedControl, SplitButton, type SplitButtonItem } from '../../components';
import type { SortMode } from '../../ipc';
import { t } from '../../i18n';
import { Popover } from '../panels/Popover';
import styles from './HostsScreen.module.css';

export type HostsLayout = 'grid' | 'list';

export const SORT_MODES: readonly SortMode[] = ['manual', 'name', 'created', 'modified'];

/** Up/Down move between the items of a popover menu. */
function handleMenuKeys(event: KeyboardEvent<HTMLElement>) {
  if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp' && event.key !== 'Home' && event.key !== 'End') {
    return;
  }
  const items = Array.from(event.currentTarget.querySelectorAll<HTMLElement>('[role^="menuitem"]'));
  if (items.length === 0) {
    return;
  }
  event.preventDefault();
  const current = items.findIndex((item) => item === document.activeElement);
  let next = 0;
  if (event.key === 'End') {
    next = items.length - 1;
  } else if (event.key === 'ArrowDown') {
    next = current < 0 ? 0 : (current + 1) % items.length;
  } else if (event.key === 'ArrowUp') {
    next = current < 0 ? items.length - 1 : (current - 1 + items.length) % items.length;
  }
  items[next]?.focus();
}

interface TagFilterProps {
  tags: readonly string[];
  selected: readonly string[];
  onChange: (tags: string[]) => void;
}

function TagFilter({ tags, selected, onChange }: TagFilterProps) {
  const [open, setOpen] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const isSelected = (tag: string) => selected.some((item) => item.toLowerCase() === tag.toLowerCase());
  const toggle = (tag: string) => {
    onChange(isSelected(tag) ? selected.filter((item) => item.toLowerCase() !== tag.toLowerCase()) : [...selected, tag]);
  };
  const label = selected.length === 0 ? t('hosts.tag_filter.button') : t('hosts.tag_filter.button_count', { count: selected.length });

  return (
    <div className={styles.anchor}>
      <Button
        ref={buttonRef}
        icon={<Tag />}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        data-testid="hosts-tag-filter-button"
      >
        {label}
      </Button>
      {open ? (
        <Popover
          anchorRef={buttonRef}
          onClose={() => setOpen(false)}
          placement="below-end"
          role="menu"
          aria-label={t('hosts.tag_filter.menu')}
          data-testid="hosts-tag-filter-menu"
        >
          <div className={styles.menuItems} onKeyDown={handleMenuKeys}>
            {tags.length === 0 ? <p className={styles.menuHint}>{t('hosts.tag_filter.empty')}</p> : null}
            {tags.map((tag, index) => {
              const checked = isSelected(tag);
              return (
                <button
                  key={tag}
                  type="button"
                  role="menuitemcheckbox"
                  aria-checked={checked}
                  className={styles.menuItem}
                  // The first item takes focus when the menu opens.
                  autoFocus={index === 0}
                  onClick={() => toggle(tag)}
                  data-tag={tag}
                  data-testid="hosts-tag-filter-option"
                >
                  <span className={styles.menuCheck} aria-hidden="true">
                    {checked ? <Check size={16} strokeWidth={2} /> : null}
                  </span>
                  <span className={styles.menuLabel}>{tag}</span>
                </button>
              );
            })}
          </div>
        </Popover>
      ) : null}
    </div>
  );
}

interface SortMenuProps {
  value: SortMode;
  onChange: (mode: SortMode) => void;
}

function SortMenu({ value, onChange }: SortMenuProps) {
  const [open, setOpen] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const choose = (mode: SortMode) => {
    onChange(mode);
    setOpen(false);
    buttonRef.current?.focus();
  };

  return (
    <div className={styles.anchor}>
      <IconButton
        ref={buttonRef}
        variant="secondary"
        icon={<ArrowDownUp />}
        aria-label={t('hosts.sort.button', { mode: t(`hosts.sort.${value}`) })}
        title={t('hosts.sort.button', { mode: t(`hosts.sort.${value}`) })}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((current) => !current)}
        data-sort={value}
        data-testid="hosts-sort-button"
      />
      {open ? (
        <Popover
          anchorRef={buttonRef}
          onClose={() => setOpen(false)}
          placement="below-end"
          role="menu"
          aria-label={t('hosts.sort.menu')}
          data-testid="hosts-sort-menu"
        >
          <div className={styles.menuItems} onKeyDown={handleMenuKeys}>
            {SORT_MODES.map((mode) => (
              <button
                key={mode}
                type="button"
                role="menuitemradio"
                aria-checked={mode === value}
                className={styles.menuItem}
                autoFocus={mode === value}
                onClick={() => choose(mode)}
                data-testid={`hosts-sort-${mode}`}
              >
                <span className={styles.menuCheck} aria-hidden="true">
                  {mode === value ? <Check size={16} strokeWidth={2} /> : null}
                </span>
                <span className={styles.menuLabel}>{t(`hosts.sort.${mode}`)}</span>
              </button>
            ))}
          </div>
        </Popover>
      ) : null}
    </div>
  );
}

export interface ActionBarProps {
  onNewHost: () => void;
  onNewGroup: () => void;
  layout: HostsLayout;
  onLayoutChange: (layout: HostsLayout) => void;
  sort: SortMode;
  onSortChange: (mode: SortMode) => void;
  /** Every tag in the vault. */
  tags: readonly string[];
  selectedTags: readonly string[];
  onSelectedTagsChange: (tags: string[]) => void;
}

export function ActionBar({
  onNewHost,
  onNewGroup,
  layout,
  onLayoutChange,
  sort,
  onSortChange,
  tags,
  selectedTags,
  onSelectedTagsChange,
}: ActionBarProps) {
  const newHostItems: SplitButtonItem[] = [
    { id: 'new-group', label: t('hosts.actions.new_group'), icon: <FolderPlus />, onSelect: onNewGroup },
    // `hosts-import` (stage 3) and `hosts-cloud-integrations` (stage 10) items are hidden.
  ];

  return (
    <>
      <div className={styles.actionBar}>
        <div className={styles.actionGroup}>
          <SplitButton
            icon={<Plus />}
            items={newHostItems}
            menuLabel={t('hosts.actions.new_menu')}
            onClick={onNewHost}
            data-testid="hosts-new-host"
          >
            {t('hosts.actions.new_host')}
          </SplitButton>
          {/* `hosts-local-terminal-button` (stage 2) and `hosts-serial-button` (stage 9) are hidden. */}
        </div>
        <div className={styles.actionGroup}>
          <SegmentedControl<HostsLayout>
            aria-label={t('hosts.view.label')}
            value={layout}
            onValueChange={onLayoutChange}
            options={[
              { value: 'grid', icon: <LayoutGrid />, 'aria-label': t('hosts.view.grid') },
              { value: 'list', icon: <List />, 'aria-label': t('hosts.view.list') },
            ]}
            data-testid="hosts-view-toggle"
          />
          <TagFilter tags={tags} selected={selectedTags} onChange={onSelectedTagsChange} />
          <SortMenu value={sort} onChange={onSortChange} />
          {/* `hosts-vault-members` (stage 8) is hidden. */}
        </div>
      </div>
      {selectedTags.length > 0 ? (
        <div className={styles.chipsRow} aria-label={t('hosts.tag_filter.active')}>
          {selectedTags.map((tag) => (
            <Chip
              key={tag}
              label={tag}
              active
              onRemove={() => onSelectedTagsChange(selectedTags.filter((item) => item !== tag))}
              removeLabel={t('hosts.tag_filter.remove', { tag })}
              data-tag={tag}
              data-testid="hosts-tag-filter-chip"
            />
          ))}
        </div>
      ) : null}
    </>
  );
}

export default ActionBar;
