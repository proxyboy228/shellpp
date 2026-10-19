// Tags of a host (UI 3.1): chips with a delete button and an input with
// autocomplete from the tags of other hosts; Enter or a comma adds a tag.

import { useId, useMemo, useState, type KeyboardEvent } from 'react';
import { Chip, TextInput } from '../../../components';
import type { ValidationReason } from '../../../ipc';
import { t } from '../../../i18n';
import { MAX_TAGS, parseTag } from '../validators';
import styles from '../panels.module.css';

export interface TagsFieldProps {
  tags: readonly string[];
  onChange: (tags: string[]) => void;
  /** Every tag in the vault, for autocomplete. */
  suggestions: readonly string[];
  /** A refusal of the whole list (for example `too_many`). */
  error?: ValidationReason;
  /** Prefix: `<id>-tags-input`, `<id>-tag`, `<id>-tags-suggestion`. */
  testIdPrefix: string;
}

const MAX_SUGGESTIONS = 8;

export function TagsField({ tags, onChange, suggestions, error, testIdPrefix }: TagsFieldProps) {
  const [input, setInput] = useState('');
  const [inputError, setInputError] = useState<ValidationReason | null>(null);
  const [focused, setFocused] = useState(false);
  const listId = useId();
  const inputId = useId();

  const present = useMemo(() => new Set(tags.map((tag) => tag.toLowerCase())), [tags]);
  const query = input.trim().toLowerCase();
  const matches = useMemo(
    () =>
      query === ''
        ? []
        : suggestions
            .filter((tag) => !present.has(tag.toLowerCase()) && tag.toLowerCase().includes(query))
            .slice(0, MAX_SUGGESTIONS),
    [suggestions, present, query],
  );

  /** Adds the given texts as tags; returns false when one was refused. */
  const addTags = (texts: readonly string[]): boolean => {
    const next = [...tags];
    const seen = new Set(present);
    for (const text of texts) {
      const result = parseTag(text);
      if (!result.ok) {
        setInputError(result.reason);
        return false;
      }
      if (result.value === undefined || seen.has(result.value.toLowerCase())) {
        continue;
      }
      if (next.length >= MAX_TAGS) {
        setInputError('too_many');
        return false;
      }
      seen.add(result.value.toLowerCase());
      next.push(result.value);
    }
    setInputError(null);
    if (next.length !== tags.length) {
      onChange(next);
    }
    return true;
  };

  const handleChange = (value: string) => {
    if (value.includes(',')) {
      const parts = value.split(',');
      const last = parts.pop() ?? '';
      if (addTags(parts)) {
        setInput(last);
      } else {
        setInput(value.replaceAll(',', ''));
      }
      return;
    }
    setInputError(null);
    setInput(value);
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      if (input.trim() !== '' && addTags([input])) {
        setInput('');
      }
    } else if (event.key === 'Backspace' && input === '' && tags.length > 0) {
      onChange(tags.slice(0, -1));
    }
  };

  const remove = (tag: string) => {
    onChange(tags.filter((item) => item !== tag));
  };

  const reason = inputError ?? error;
  const showSuggestions = focused && matches.length > 0;

  return (
    <div className={styles.field}>
      <label htmlFor={inputId} className={styles.label}>
        {t('panels.tags.label')}
      </label>
      {tags.length > 0 ? (
        <div className={styles.chips}>
          {tags.map((tag) => (
            <Chip
              key={tag}
              label={tag}
              onRemove={() => remove(tag)}
              removeLabel={t('panels.tags.remove', { tag })}
              data-tag={tag}
              data-testid={`${testIdPrefix}-tag`}
            />
          ))}
        </div>
      ) : null}
      <TextInput
        id={inputId}
        placeholder={t('panels.tags.placeholder')}
        value={input}
        role="combobox"
        aria-autocomplete="list"
        aria-expanded={showSuggestions}
        aria-controls={showSuggestions ? listId : undefined}
        error={reason === undefined || reason === null ? undefined : t(`validation.${reason}`)}
        onChange={(event) => handleChange(event.target.value)}
        onKeyDown={handleKeyDown}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        data-testid={`${testIdPrefix}-tags-input`}
      />
      {showSuggestions ? (
        <div id={listId} role="listbox" aria-label={t('panels.tags.suggestions')} className={styles.suggestions}>
          {matches.map((tag) => (
            <div
              key={tag}
              role="option"
              aria-selected="false"
              className={styles.option}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => {
                if (addTags([tag])) {
                  setInput('');
                }
              }}
              data-tag={tag}
              data-testid={`${testIdPrefix}-tags-suggestion`}
            >
              <span className={styles.optionLabel}>{tag}</span>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}

export default TagsField;
