// Prompt kinds of the connection screen (UI 4.8).
//
// Owner: S7.

import type { SessionPrompt } from '../../ipc';

/** Prompts answered with text (host key prompts have their own dialog). */
export type TextPrompt = Extract<SessionPrompt, { kind: 'username' | 'password' | 'passphrase' | 'keyboard_interactive' }>;

export function isTextPrompt(prompt: SessionPrompt | undefined): prompt is TextPrompt {
  return (
    prompt !== undefined &&
    (prompt.kind === 'username' || prompt.kind === 'password' || prompt.kind === 'passphrase' || prompt.kind === 'keyboard_interactive')
  );
}
