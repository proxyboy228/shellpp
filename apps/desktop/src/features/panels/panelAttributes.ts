// Puts `data-record-id` and `data-save-state` on the details panel root
// (stage plan 7.5, 7.6). `DetailsPanel` (W4) receives them as props as well;
// this keeps them on the element whatever props its root forwards.

import { useLayoutEffect, type RefObject } from 'react';

export function usePanelRootAttributes(
  containerRef: RefObject<HTMLElement | null>,
  testId: string,
  attributes: Readonly<Record<string, string | undefined>>,
): void {
  useLayoutEffect(() => {
    const root = containerRef.current?.querySelector(`[data-testid="${testId}"]`);
    if (root === null || root === undefined) {
      return;
    }
    for (const [name, value] of Object.entries(attributes)) {
      if (value === undefined) {
        root.removeAttribute(name);
      } else if (root.getAttribute(name) !== value) {
        root.setAttribute(name, value);
      }
    }
  });
}
