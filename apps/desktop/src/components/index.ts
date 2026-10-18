// Base components (docs/design-system.md, docs/decisions.md 1).
// Import components from this barrel rather than from their files.

// Controls: buttons, text fields, the switch and the segmented control.
export { Button } from './Button/Button';
export type { ButtonProps, ButtonVariant } from './Button/Button';

export { SplitButton } from './SplitButton/SplitButton';
export type { SplitButtonItem, SplitButtonProps, SplitButtonVariant } from './SplitButton/SplitButton';

export { IconButton } from './IconButton/IconButton';
export type { IconButtonProps, IconButtonSize, IconButtonVariant } from './IconButton/IconButton';

export { TextInput } from './TextInput/TextInput';
export type { TextInputProps, TextInputType } from './TextInput/TextInput';

export { PasswordInput } from './PasswordInput/PasswordInput';
export type { PasswordInputProps } from './PasswordInput/PasswordInput';

export { Toggle } from './Toggle/Toggle';
export type { ToggleProps } from './Toggle/Toggle';

export { SegmentedControl } from './SegmentedControl/SegmentedControl';
export type { SegmentedControlOption, SegmentedControlProps } from './SegmentedControl/SegmentedControl';

// Display and overlays: chips, status dots, host cards, menus and modal windows.
export { Chip } from './Chip/Chip';
export type { ChipProps } from './Chip/Chip';

export { STATUS_DOT_LABELS, StatusDot } from './StatusDot/StatusDot';
export type { StatusDotProps, StatusDotStatus } from './StatusDot/StatusDot';

export { formatHostCountDefault, HostCard, TAG_COLORS } from './HostCard/HostCard';
export type {
  HostCardEvent,
  HostCardGroupProps,
  HostCardHostProps,
  HostCardProps,
  TagColor,
} from './HostCard/HostCard';

export { Menu } from './Menu/Menu';
export type { MenuEntry, MenuItemDef, MenuProps, MenuSeparatorDef } from './Menu/Menu';

export { Modal } from './Modal/Modal';
export type { ModalProps, ModalSize } from './Modal/Modal';
