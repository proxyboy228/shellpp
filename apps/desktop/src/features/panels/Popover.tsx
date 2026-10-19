// A small anchored popover for the Hosts screen menus (sort, tag filter) and
// the panel pickers. Closes on Escape (focus returns to the anchor) and on a
// pointer press outside the popover and its anchor.

import { useEffect, useRef, type AriaRole, type KeyboardEvent, type ReactNode, type RefObject } from 'react';
import styles from './Popover.module.css';

export type PopoverPlacement = 'inline' | 'below-start' | 'below-end';

export interface PopoverProps {
  anchorRef: RefObject<HTMLElement | null>;
  onClose: () => void;
  /** `inline`: in the document flow under the anchor (inside scrolling panels); otherwise an overlay. */
  placement?: PopoverPlacement;
  role?: AriaRole;
  'aria-label'?: string;
  className?: string;
  children: ReactNode;
  'data-testid'?: string;
}

const placementClass: Readonly<Record<PopoverPlacement, string>> = {
  inline: styles.inline,
  'below-start': styles.belowStart,
  'below-end': styles.belowEnd,
};

export function Popover({
  anchorRef,
  onClose,
  placement = 'below-start',
  role,
  'aria-label': ariaLabel,
  className,
  children,
  'data-testid': testId,
}: PopoverProps) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) {
        return;
      }
      if (ref.current?.contains(target) || anchorRef.current?.contains(target)) {
        return;
      }
      onClose();
    };
    document.addEventListener('pointerdown', handlePointerDown, true);
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown, true);
    };
  }, [anchorRef, onClose]);

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      onClose();
      anchorRef.current?.focus();
    }
  };

  return (
    <div
      ref={ref}
      role={role}
      aria-label={ariaLabel}
      className={[styles.popover, placementClass[placement], className].filter(Boolean).join(' ')}
      onKeyDown={handleKeyDown}
      data-testid={testId}
    >
      {children}
    </div>
  );
}

export default Popover;
