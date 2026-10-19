// Parent Group picker (UI 3.1, 3.4): a button with the current group, and a
// list with search. The host panel can create a group on the fly; the group
// panel excludes the group itself and its subtree.

import { ChevronDown, Folder, Plus } from 'lucide-react';
import { useId, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { TextInput } from '../../../components';
import type { GroupView, Uuid } from '../../../ipc';
import { t } from '../../../i18n';
import { groupPathLabel, sortedByPath } from '../../hosts/format';
import { Popover } from '../Popover';
import styles from '../panels.module.css';

export interface GroupPickerProps {
  label: string;
  /** Selected group id; `''` = top level. */
  value: Uuid | '';
  onChange: (groupId: Uuid | '') => void;
  /** Every group of the vault. */
  groups: readonly GroupView[];
  groupsById: ReadonlyMap<Uuid, GroupView>;
  /** Groups that cannot be chosen (the group itself and its subtree). */
  exclude?: ReadonlySet<Uuid>;
  /** Creates a group with this name; resolves with its id (null when it failed). */
  onCreate?: (name: string) => Promise<Uuid | null>;
  /**
   * Test id of the button. Parts: `<id>-popover`, `<id>-search`,
   * `<id>-option` (`data-group-id`, `""` = top level); `createTestId` for
   * the create item.
   */
  'data-testid': string;
  createTestId?: string;
}

interface Option {
  id: Uuid | '';
  label: string;
}

export function GroupPicker({
  label,
  value,
  onChange,
  groups,
  groupsById,
  exclude,
  onCreate,
  'data-testid': testId,
  createTestId,
}: GroupPickerProps) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState('');
  const [activeIndex, setActiveIndex] = useState(0);
  const [creating, setCreating] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const labelId = useId();
  const listId = useId();

  const selected = value === '' ? undefined : groupsById.get(value);
  let buttonText = t('panels.group_picker.top_level');
  if (value !== '') {
    buttonText = selected === undefined ? t('panels.group_picker.missing') : groupPathLabel(selected, groupsById);
  }

  const query = search.trim().toLowerCase();
  const options = useMemo<Option[]>(() => {
    const result: Option[] = [];
    if (query === '') {
      result.push({ id: '', label: t('panels.group_picker.top_level') });
    }
    for (const { group, label: pathLabel } of sortedByPath(groups, groupsById)) {
      if (exclude?.has(group.record_id)) {
        continue;
      }
      if (query !== '' && !group.data.name.toLowerCase().includes(query)) {
        continue;
      }
      result.push({ id: group.record_id, label: pathLabel });
    }
    return result;
  }, [groups, groupsById, exclude, query]);

  const exactMatch = groups.some((group) => group.data.name.trim().toLowerCase() === query);
  const canCreate = onCreate !== undefined && query !== '' && !exactMatch && !creating;
  const itemCount = options.length + (canCreate ? 1 : 0);

  const close = () => {
    setOpen(false);
    setSearch('');
    setActiveIndex(0);
  };

  const choose = (id: Uuid | '') => {
    onChange(id);
    close();
    buttonRef.current?.focus();
  };

  const create = async () => {
    if (onCreate === undefined) {
      return;
    }
    const name = search.trim();
    setCreating(true);
    const id = await onCreate(name);
    setCreating(false);
    if (id !== null) {
      choose(id);
    }
  };

  const activate = (index: number) => {
    const option = options[index];
    if (option !== undefined) {
      choose(option.id);
    } else if (canCreate && index === options.length) {
      void create();
    }
  };

  const handleSearchKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      if (itemCount > 0) {
        const step = event.key === 'ArrowDown' ? 1 : -1;
        setActiveIndex((index) => (index + step + itemCount) % itemCount);
      }
    } else if (event.key === 'Enter') {
      event.preventDefault();
      activate(Math.min(activeIndex, itemCount - 1));
    }
  };

  return (
    <div className={styles.field}>
      <span id={labelId} className={styles.label}>
        {label}
      </span>
      <button
        ref={buttonRef}
        type="button"
        className={styles.pickerButton}
        aria-labelledby={`${labelId} ${testId}-value`}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => (open ? close() : setOpen(true))}
        data-testid={testId}
        data-group-id={value}
      >
        <Folder size={16} strokeWidth={1.75} aria-hidden="true" focusable="false" />
        <span id={`${testId}-value`} className={styles.pickerValue}>
          {buttonText}
        </span>
        <ChevronDown size={16} strokeWidth={1.75} aria-hidden="true" focusable="false" />
      </button>
      {open ? (
        <Popover anchorRef={buttonRef} onClose={close} placement="inline" data-testid={`${testId}-popover`}>
          <TextInput
            aria-label={t('panels.group_picker.search')}
            placeholder={t('panels.group_picker.search')}
            value={search}
            autoFocus
            role="combobox"
            aria-expanded="true"
            aria-controls={listId}
            onChange={(event) => {
              setSearch(event.target.value);
              setActiveIndex(0);
            }}
            onKeyDown={handleSearchKeyDown}
            data-testid={`${testId}-search`}
          />
          <div id={listId} role="listbox" aria-label={label} className={styles.optionList}>
            {options.map((option, index) => (
              <div
                key={option.id === '' ? 'top-level' : option.id}
                role="option"
                aria-selected={option.id === value}
                className={styles.option}
                data-active={index === activeIndex || undefined}
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => choose(option.id)}
                data-testid={`${testId}-option`}
                data-group-id={option.id}
              >
                <span className={styles.optionLabel}>{option.label}</span>
              </div>
            ))}
            {options.length === 0 && !canCreate ? <p className={styles.hint}>{t('panels.group_picker.no_matches')}</p> : null}
          </div>
          {canCreate ? (
            <button
              type="button"
              className={styles.option}
              data-active={activeIndex === options.length || undefined}
              onClick={() => void create()}
              data-testid={createTestId}
            >
              <Plus size={16} strokeWidth={1.75} aria-hidden="true" focusable="false" />
              <span className={styles.optionLabel}>{t('panels.group_picker.create', { name: search.trim() })}</span>
            </button>
          ) : null}
        </Popover>
      ) : null}
    </div>
  );
}

export default GroupPicker;
