// App menu Edit commands (UI 1.4): undo, redo, cut, copy, paste and select
// all act on the editable element that had focus before the menu opened.
//
// Cut, copy and paste go through the core's clipboard commands
// (`clipboardWriteText`, `clipboardReadText`) rather than the webview
// clipboard, so they behave the same in every webview and in e2e builds with
// the mock clipboard. Text from password fields and other sensitive elements
// (`data-sensitive`: secret fields in every visibility state, the recovery
// key) is never copied or cut here: sensitive values reach the clipboard only
// through the core's protected copy (docs/security.md, section 9.4).
//
// Owner: W4.

import { clipboardReadText, clipboardWriteText } from '../ipc';

export type EditCommand = 'undo' | 'redo' | 'cut' | 'copy' | 'paste' | 'select-all';

/** Elements inside these containers (the app menu and its button) do not change the edit target. */
const KEEP_TARGET_SELECTOR = '[data-context-menu], [data-keep-edit-target]';
const TEXT_INPUT_TYPES = new Set(['text', 'search', 'url', 'tel', 'email', 'password', 'number', '']);

let lastEditable: HTMLElement | null = null;

function isEditable(element: Element): element is HTMLElement {
  if (element instanceof HTMLTextAreaElement) {
    return !element.readOnly && !element.disabled;
  }
  if (element instanceof HTMLInputElement) {
    return TEXT_INPUT_TYPES.has(element.type) && !element.readOnly && !element.disabled;
  }
  return element instanceof HTMLElement && element.isContentEditable;
}

function isTextField(element: HTMLElement): element is HTMLInputElement | HTMLTextAreaElement {
  return element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement;
}

/** Tracks the last focused editable element; returns a function that stops tracking. */
export function installEditTracking(doc: Document = document): () => void {
  const handleFocusIn = (event: FocusEvent) => {
    const target = event.target;
    if (!(target instanceof Element) || target.closest(KEEP_TARGET_SELECTOR) !== null) {
      return;
    }
    lastEditable = isEditable(target) ? target : null;
  };
  doc.addEventListener('focusin', handleFocusIn, true);
  return () => {
    doc.removeEventListener('focusin', handleFocusIn, true);
    lastEditable = null;
  };
}

/** The element Edit commands act on (for tests). */
export function editTarget(): HTMLElement | null {
  return lastEditable !== null && lastEditable.isConnected ? lastEditable : null;
}

function selectedText(element: HTMLElement): string {
  if (isTextField(element)) {
    const start = element.selectionStart ?? 0;
    const end = element.selectionEnd ?? start;
    return element.value.slice(start, end);
  }
  return element.ownerDocument.getSelection()?.toString() ?? '';
}

const SENSITIVE_SELECTOR = '[data-sensitive]';

function insideSensitive(node: Node | null | undefined): boolean {
  const element = node instanceof Element ? node : (node?.parentElement ?? null);
  return element?.closest(SENSITIVE_SELECTOR) != null;
}

/** A password field, an element marked `data-sensitive`, or a selection inside one. */
function isSensitive(element: HTMLElement): boolean {
  if (element instanceof HTMLInputElement && element.type === 'password') {
    return true;
  }
  if (insideSensitive(element)) {
    return true;
  }
  if (isTextField(element)) {
    return false;
  }
  // Other targets copy the document selection.
  const selection = element.ownerDocument.getSelection();
  return insideSensitive(selection?.anchorNode) || insideSensitive(selection?.focusNode);
}

function exec(command: string, value?: string): boolean {
  try {
    return document.execCommand(command, false, value);
  } catch {
    return false;
  }
}

/** Runs an Edit command on the remembered element (nothing happens without one). */
export async function runEditCommand(command: EditCommand): Promise<void> {
  const target = editTarget();
  if (target === null) {
    return;
  }
  target.focus();
  switch (command) {
    case 'undo':
    case 'redo':
      exec(command);
      return;
    case 'select-all':
      if (isTextField(target)) {
        target.select();
      } else {
        exec('selectAll');
      }
      return;
    case 'copy':
    case 'cut': {
      if (isSensitive(target)) {
        return;
      }
      const text = selectedText(target);
      if (text === '') {
        return;
      }
      await clipboardWriteText(text);
      if (command === 'cut') {
        target.focus();
        exec('delete');
      }
      return;
    }
    case 'paste': {
      const { text } = await clipboardReadText();
      if (text === null || text === '') {
        return;
      }
      target.focus();
      exec('insertText', text);
      return;
    }
  }
}
