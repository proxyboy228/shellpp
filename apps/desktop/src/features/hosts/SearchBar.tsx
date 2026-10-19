// Search bar of the Hosts screen (UI 2.1): filters groups and hosts in real
// time. The Connect button is enabled when the text is a connection address
// (D23) or exactly matches a saved host; Enter or a click connects (Alt: a
// new window).

import { Search, X } from 'lucide-react';
import type { KeyboardEvent, MouseEvent, RefObject } from 'react';
import { Button, IconButton, TextInput } from '../../components';
import { t } from '../../i18n';
import { isVisible } from '../../stage';
import styles from './HostsScreen.module.css';

export interface SearchBarProps {
  value: string;
  onChange: (value: string) => void;
  onClear: () => void;
  inputRef?: RefObject<HTMLInputElement | null>;
  /** The text can be connected to (an address or an exact saved host match). */
  canConnect?: boolean;
  /** Enter or the Connect button; `newWindow` when Alt is held. */
  onConnect?: (options: { newWindow: boolean }) => void;
}

export function SearchBar({ value, onChange, onClear, inputRef, canConnect = false, onConnect }: SearchBarProps) {
  const quickConnect = isVisible('hosts-quick-connect');

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Escape' && value !== '') {
      event.preventDefault();
      event.stopPropagation();
      onClear();
    } else if (event.key === 'Enter' && quickConnect && canConnect && !event.nativeEvent.isComposing) {
      event.preventDefault();
      onConnect?.({ newWindow: event.altKey });
    }
  };

  return (
    <div className={styles.searchRow} role="search">
      <TextInput
        ref={inputRef}
        className={styles.searchField}
        value={value}
        // The quick-connect hint only together with quick connect (stage 2).
        placeholder={t(quickConnect ? 'hosts.search.placeholder' : 'hosts.search.placeholder_search')}
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
      {quickConnect ? (
        <Button
          variant="primary"
          disabled={!canConnect}
          title={t('hosts.search.connect_hint')}
          onClick={(event: MouseEvent<HTMLButtonElement>) => onConnect?.({ newWindow: event.altKey })}
          data-testid="hosts-search-connect"
        >
          {t('hosts.search.connect')}
        </Button>
      ) : null}
    </div>
  );
}

export default SearchBar;
