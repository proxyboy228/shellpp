// Connecting and connect actions (UI 2.1, 2.4, 3.1, 3.3, 4.8; stage 2 plan,
// section 6.3): the connection screen with steps, prompts and the failure
// state (used by S6 in every pane whose session is connecting or failed),
// connect helpers for the Hosts screen and the host panel, quick connect
// with the save offer, and the "+ -> Key" dialog.
//
// Owner: S7.

export { connectHost, connectHosts, connectQuick, onSessionConnected, quickHostData, saveQuickHost } from './actions';
export type { ConnectOptions } from './actions';
export { installAltKeyTracker, isAltHeld } from './altKey';
export { ConnectingView } from './ConnectingView';
export type { ConnectingViewProps } from './ConnectingView';
export { connectionAddress, hostKeyAddress, sessionAddress, sessionErrorMessage } from './format';
export { KeyAddDialog } from './KeyAddDialog';
export type { KeyAddDialogProps } from './KeyAddDialog';
export { SessionPromptForm } from './SessionPromptForm';
export { useKeys } from './useKeys';
