// History tab helpers (UI 4.2): which history list a session uses and the
// most frequent commands for the stage-5 "save as snippets" card.
//
// Owner: S6.

import type { HistoryEntryView, RecordRef, SessionInfo } from '../../ipc';

/** The host whose history a session writes: a saved host, or none (local terminals, quick connects). */
export function historyHost(session: SessionInfo | undefined): RecordRef | null {
  return session?.target?.kind === 'host' ? { vault_id: session.target.vault_id, record_id: session.target.record_id } : null;
}

/** The most frequent commands, most frequent first (the stage-5 "save as snippets" card). */
export function frequentCommands(entries: readonly HistoryEntryView[], limit = 3): string[] {
  const counts = new Map<string, number>();
  for (const entry of entries) {
    counts.set(entry.command, (counts.get(entry.command) ?? 0) + 1);
  }
  return [...counts.entries()]
    .filter(([, count]) => count > 1)
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .slice(0, limit)
    .map(([command]) => command);
}
