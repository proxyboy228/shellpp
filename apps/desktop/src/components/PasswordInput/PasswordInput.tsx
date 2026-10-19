import { Eye, EyeOff } from 'lucide-react';
import { useState } from 'react';
import { IconButton } from '../IconButton/IconButton';
import { TextInput } from '../TextInput/TextInput';
import type { TextInputProps } from '../TextInput/TextInput';
import styles from './PasswordInput.module.css';

export interface PasswordInputProps extends Omit<TextInputProps, 'type' | 'trailing'> {
  /** Controlled visibility of the value. Omit to let the component keep its own state. */
  visible?: boolean;
  /** Initial visibility when uncontrolled. Default: `false` (masked). */
  defaultVisible?: boolean;
  /** Called when the show/hide toggle is used. */
  onVisibleChange?: (visible: boolean) => void;
  /**
   * Accessible name of the show/hide toggle. It stays the same in both states;
   * `aria-pressed` tells whether the value is shown. Default: "Show password".
   */
  toggleLabel?: string;
}

/**
 * Text field for secrets: masked by default, with a toggle that shows the
 * value. Autocomplete, spell check and autocorrection are off unless the
 * caller overrides them.
 *
 * The value never leaves through copy or cut, also while it is shown: the
 * field is marked `data-sensitive` (the app's Edit menu refuses it) and the
 * native copy and cut events are cancelled before the caller's handlers run.
 * Sensitive values reach the clipboard only through the core's protected copy
 * (docs/security.md, section 9.4).
 */
export function PasswordInput({
  visible,
  defaultVisible = false,
  onVisibleChange,
  toggleLabel = 'Show password',
  disabled = false,
  autoComplete = 'off',
  spellCheck = false,
  autoCapitalize = 'off',
  autoCorrect = 'off',
  className,
  onCopy,
  onCut,
  'data-testid': testId,
  ...rest
}: PasswordInputProps) {
  const [internalVisible, setInternalVisible] = useState(defaultVisible);
  const isVisible = visible ?? internalVisible;

  function handleToggle() {
    const next = !isVisible;
    if (visible === undefined) {
      setInternalVisible(next);
    }
    onVisibleChange?.(next);
  }

  return (
    <TextInput
      {...rest}
      className={[styles.root, className].filter(Boolean).join(' ')}
      type={isVisible ? 'text' : 'password'}
      disabled={disabled}
      autoComplete={autoComplete}
      spellCheck={spellCheck}
      autoCapitalize={autoCapitalize}
      autoCorrect={autoCorrect}
      data-sensitive=""
      onCopy={(event) => {
        event.preventDefault();
        onCopy?.(event);
      }}
      onCut={(event) => {
        event.preventDefault();
        onCut?.(event);
      }}
      data-testid={testId}
      trailing={
        <IconButton
          size="sm"
          className={styles.toggle}
          icon={isVisible ? <EyeOff /> : <Eye />}
          aria-label={toggleLabel}
          pressed={isVisible}
          disabled={disabled}
          onClick={handleToggle}
          data-testid={testId === undefined ? undefined : `${testId}-toggle`}
        />
      }
    />
  );
}

export default PasswordInput;
