// Paste into the terminal (UI 4.3, 12.5; docs/security.md, sections 9.4 and
// 12): the bracketed paste markers `ESC[200~` / `ESC[201~`, ESC and every
// other control character except TAB, CR and LF are removed; CRLF and LF
// become CR (what the Enter key sends); a paste that still contains a line
// break is multi-line and asks for confirmation when
// `terminal.confirm_multiline_paste` is on; when the application turned on
// bracketed paste mode, the text is wrapped in `ESC[200~` ... `ESC[201~`.
//
// Owner: S6.

const PASTE_START = '\x1b[200~';
const PASTE_END = '\x1b[201~';

/**
 * Removes the paste markers, ESC and control characters (C0 except TAB, CR,
 * LF; DEL; C1), then turns CRLF and LF into CR.
 */
export function filterPaste(text: string): string {
  const withoutMarkers = text.split(PASTE_START).join('').split(PASTE_END).join('');
  // eslint-disable-next-line no-control-regex
  const withoutControls = withoutMarkers.replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f-\x9f]/g, '');
  return withoutControls.replace(/\r\n|\n/g, '\r');
}

/** Number of lines of filtered text (a trailing line break does not start a new line). */
export function pasteLineCount(filtered: string): number {
  if (filtered === '') {
    return 0;
  }
  const trimmed = filtered.endsWith('\r') ? filtered.slice(0, -1) : filtered;
  return trimmed.split('\r').length;
}

/** True when the filtered text contains a line break (paste confirmation, UI 4.3). */
export function isMultiline(filtered: string): boolean {
  return filtered.includes('\r');
}

/** The bytes to send for filtered text. */
export function wrapPaste(filtered: string, bracketedPasteMode: boolean): string {
  return bracketedPasteMode ? `${PASTE_START}${filtered}${PASTE_END}` : filtered;
}

/** The preview shown in the confirmation: line breaks as new lines, at most `maxLines` lines. */
export function pastePreview(filtered: string, maxLines = 12): string {
  const lines = filtered.split('\r');
  if (lines[lines.length - 1] === '') {
    lines.pop();
  }
  const shown = lines.slice(0, maxLines).join('\n');
  return lines.length > maxLines ? `${shown}\n…` : shown;
}
