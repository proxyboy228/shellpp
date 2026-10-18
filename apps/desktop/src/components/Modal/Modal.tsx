import { X } from 'lucide-react';
import {
  useEffect,
  useId,
  useRef,
  useState,
  type HTMLAttributes,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
  type RefObject,
} from 'react';
import { createPortal } from 'react-dom';
import styles from './Modal.module.css';

export type ModalSize = 'small' | 'medium' | 'large';

export interface ModalProps extends Omit<HTMLAttributes<HTMLDivElement>, 'children' | 'title' | 'role'> {
  /** The modal window is rendered only while `open` is true. */
  open: boolean;
  /** Title, 20 px; it labels the dialog. */
  title: ReactNode;
  /** Called on Escape, on the close button and, when enabled, on a scrim click. */
  onClose: () => void;
  children?: ReactNode;
  /** Action row at the bottom, for example Cancel and the primary button. */
  footer?: ReactNode;
  /** Accessible name of the close button. Default: `Close`. */
  closeLabel?: string;
  /** Width: small 400 px, medium 520 px (default), large 720 px. */
  size?: ModalSize;
  /** Element focused on open. Default: the first focusable element of the content, else the close button. */
  initialFocusRef?: RefObject<HTMLElement | null>;
  /** Close on a click on the scrim. Default: false. */
  closeOnScrimClick?: boolean;
  /** Where the modal window is rendered. Default: `document.body`. */
  container?: HTMLElement | null;
  /** Test id of the dialog; the scrim gets `<id>-scrim`, the close button `<id>-close`. */
  'data-testid'?: string;
}

const FOCUSABLE_SELECTOR = [
  'a[href]',
  'area[href]',
  'button:not([disabled])',
  'input:not([disabled]):not([type="hidden"])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  'iframe',
  '[contenteditable=""]',
  '[contenteditable="true"]',
  '[tabindex]:not([tabindex="-1"])',
].join(',');

function getFocusable(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(
    (element) => !element.hasAttribute('hidden') && element.closest('[inert], [aria-hidden="true"]') === null,
  );
}

const sizeClass: Readonly<Record<ModalSize, string>> = {
  small: styles.small,
  medium: styles.medium,
  large: styles.large,
};

/**
 * Modal window (`role="dialog"`, `aria-modal`): scrim, title, close button,
 * content and an optional footer. Escape and the close button call
 * `onClose`; Tab and Shift+Tab stay inside; focus returns to the previously
 * focused element when it closes.
 */
export function Modal({ open, container, ...props }: ModalProps) {
  if (!open) {
    return null;
  }
  const target = container ?? (typeof document === 'undefined' ? null : document.body);
  if (target === null) {
    return null;
  }
  return createPortal(<ModalWindow {...props} />, target);
}

type ModalWindowProps = Omit<ModalProps, 'open' | 'container'>;

function ModalWindow({
  title,
  onClose,
  children,
  footer,
  closeLabel = 'Close',
  size = 'medium',
  initialFocusRef,
  closeOnScrimClick = false,
  className,
  onKeyDown,
  'data-testid': testId,
  ...rest
}: ModalWindowProps) {
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  // Read on the first render, before the effects of the content (for example
  // an auto-focused menu) can move focus into the dialog.
  const [previousFocus] = useState<HTMLElement | null>(() =>
    document.activeElement instanceof HTMLElement ? document.activeElement : null,
  );

  // Ref objects are stable, so this runs once, when the window opens.
  useEffect(() => {
    const dialog = dialogRef.current;
    if (dialog !== null) {
      const closeButton = closeButtonRef.current;
      const explicit = initialFocusRef?.current ?? null;
      if (explicit !== null) {
        explicit.focus();
      } else if (!dialog.contains(document.activeElement)) {
        const firstInContent = getFocusable(dialog).find((element) => element !== closeButton);
        (firstInContent ?? closeButton ?? dialog).focus();
      }
    }
    return () => {
      if (previousFocus !== null && previousFocus !== document.body && previousFocus.isConnected) {
        previousFocus.focus();
      }
    };
  }, [initialFocusRef, previousFocus]);

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    onKeyDown?.(event);
    const dialog = dialogRef.current;
    if (event.defaultPrevented || dialog === null) {
      return;
    }
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      onClose();
      return;
    }
    if (event.key !== 'Tab') {
      return;
    }
    const focusable = getFocusable(dialog);
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const active = document.activeElement;
    if (first === undefined || last === undefined) {
      event.preventDefault();
      dialog.focus();
      return;
    }
    const outside = active === null || active === dialog || !dialog.contains(active);
    if (event.shiftKey && (active === first || outside)) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && (active === last || (outside && active !== dialog))) {
      event.preventDefault();
      first.focus();
    }
  };

  const handleScrimMouseDown = (event: MouseEvent<HTMLDivElement>) => {
    if (event.target !== event.currentTarget) {
      return;
    }
    // Keep focus inside the dialog: a press on the scrim would move it to the body.
    event.preventDefault();
    if (closeOnScrimClick) {
      onClose();
    }
  };

  const dialogClassName = [styles.dialog, sizeClass[size], className].filter(Boolean).join(' ');

  return (
    <div
      className={styles.scrim}
      onMouseDown={handleScrimMouseDown}
      data-testid={testId === undefined ? undefined : `${testId}-scrim`}
    >
      <div
        aria-labelledby={titleId}
        {...rest}
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        tabIndex={-1}
        className={dialogClassName}
        data-testid={testId}
        onKeyDown={handleKeyDown}
      >
        <header className={styles.header}>
          <h2 id={titleId} className={styles.title}>
            {title}
          </h2>
          <button
            ref={closeButtonRef}
            type="button"
            className={styles.close}
            aria-label={closeLabel}
            onClick={() => onClose()}
            data-testid={testId === undefined ? undefined : `${testId}-close`}
          >
            <X size={16} strokeWidth={2} aria-hidden="true" focusable="false" />
          </button>
        </header>
        <div className={styles.body}>{children}</div>
        {footer === undefined || footer === null ? null : <footer className={styles.footer}>{footer}</footer>}
      </div>
    </div>
  );
}

export default Modal;
