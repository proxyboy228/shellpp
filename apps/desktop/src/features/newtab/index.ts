// New tab page, host picker and the Ctrl+K search (UI 6, 4.7, 12.5; stage 2
// plan, section 6.3).
//
// Owner: S7. Components and props are the contract used by S6 (new-tab tabs
// and empty split panes) and the app shell (search palette).

export { HostPicker } from './HostPicker';
export type { HostPickerProps } from './HostPicker';
export { NewTabPage } from './NewTabPage';
export type { NewTabPageProps } from './NewTabPage';
export { closeSearchPalette, isSearchPaletteOpen, openSearchPalette, SearchPalette } from './SearchPalette';
