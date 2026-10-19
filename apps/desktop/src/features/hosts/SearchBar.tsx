// Search bar of the Hosts screen (UI 2.1): filters groups and hosts in real
// time. The Connect button and Enter quick connect are stage 2
// (`hosts-quick-connect`); the address recognition of D23 is ready for them.

import { Search, X } from 'lucide-react';
import type { KeyboardEvent, RefObject } from 'react';
import { Button, IconButton, TextInput } from '../../components';
import { t } from '../../i18n';
import { isVisible } from '../../stage';
import { parseQuickConnect } from './quickConnect';
import styles from './HostsScreen.module.css';

export interface SearchBarProps {
  value: string;
  onChange: (value: string) => void;
  onClear: () => void;
  inputRef?: RefObject<HTMLInputElement | null>;
}

export function SearchBar({ value, onChange, onClear, inputRef }: SearchBarProps) {
  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Escape' && value !== '') {
      event.preventDefault();
      event.stopPropagation();
      onClear();
    }
  };

  return (
    <div className={styles.searchRow} role="search">
      <TextInput
        ref={inputRef}
        className={styles.searchField}
        value={value}
        // The quick-connect hint only together with quick connect (stage 2).
        placeholder={t(isVisible('hosts-quick-connect') ? 'hosts.search.placeholder' : 'hosts.search.placeholder_search')}
        aria-label={t('hosts.search.label')}
        autoComplete="off"
        spellCheck={false}
        leadingIcon={<Search />}
        trailing={
          value === '' ? undefined : (
            <IconButton
              size="sm"
              icon={<X />}
              aria-label={t('hosts.search.clear')}
              onClick={() => {
                onClear();
                inputRef?.current?.focus();
              }}
              data-testid="hosts-search-clear"
            />
          )
        }
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={handleKeyDown}
        data-testid="hosts-search-input"
      />
      {isVisible('hosts-quick-connect') ? (
        <Button variant="primary" disabled={parseQuickConnect(value) === null} data-testid="hosts-search-connect">
          {t('hosts.search.connect')}
        </Button>
      ) : null}
    </div>
  );
}

export default SearchBar;
