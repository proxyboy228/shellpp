// Every base component in its variants and states (docs/decisions.md 1).
// Hover, press and keyboard focus are live: point at or tab to a component.

import {
  Copy,
  Cpu,
  Database,
  Ellipsis,
  Folder,
  FolderInput,
  KeyRound,
  LayoutGrid,
  Link,
  List,
  Pencil,
  Plus,
  Search,
  Settings,
  Terminal,
  Trash,
  Upload,
} from 'lucide-react';
import { useId, useRef, useState, type ReactNode } from 'react';
import {
  Button,
  Checkbox,
  Chip,
  HostCard,
  IconButton,
  Menu,
  Modal,
  PasswordInput,
  SegmentedControl,
  Select,
  SplitButton,
  STATUS_DOT_LABELS,
  StatusDot,
  TextInput,
  Toggle,
  type ButtonVariant,
  type HostCardEvent,
  type HostCardProps,
  type IconButtonVariant,
  type MenuEntry,
  type SegmentedControlOption,
  type SplitButtonItem,
  type StatusDotStatus,
} from '../components';
import type { ThemeName } from '../theme/tokens';
import styles from './Components.module.css';
import { Row, Section, Specimen } from './Section';

/** Reports an action triggered in a demo. */
export type Log = (action: string) => void;

interface DemoProps {
  log: Log;
}

function noop() {
  // Demo controls that only need to be interactive.
}

const STATUSES: readonly StatusDotStatus[] = ['connected', 'connecting', 'error', 'inactive'];

// --- Button ---------------------------------------------------------------

const BUTTON_SAMPLES: readonly { variant: ButtonVariant; label: string; icon: ReactNode }[] = [
  { variant: 'primary', label: 'Connect', icon: <Terminal /> },
  { variant: 'secondary', label: 'Edit', icon: <Pencil /> },
  { variant: 'danger', label: 'Remove', icon: <Trash /> },
  { variant: 'text', label: 'Add targets', icon: <Plus /> },
];

function ButtonDemo({ log }: DemoProps) {
  const [running, setRunning] = useState(false);

  function run() {
    setRunning(true);
    log('Run started');
    window.setTimeout(() => setRunning(false), 1500);
  }

  return (
    <Specimen
      id="button"
      title="Button"
      description="Primary is the single main action of a screen, danger a destructive action, text a link-style action. Loading keeps focus and ignores clicks."
    >
      <div className={styles.tableScroll}>
        <table className={styles.matrix}>
          <thead>
            <tr>
              <th scope="col">Variant</th>
              <th scope="col">Default</th>
              <th scope="col">With icon</th>
              <th scope="col">Loading</th>
              <th scope="col">Disabled</th>
            </tr>
          </thead>
          <tbody>
            {BUTTON_SAMPLES.map(({ variant, label, icon }) => (
              <tr key={variant}>
                <th scope="row">
                  <code>{variant}</code>
                </th>
                <td>
                  <Button variant={variant} onClick={() => log(`${label} (${variant})`)}>
                    {label}
                  </Button>
                </td>
                <td>
                  <Button variant={variant} icon={icon} onClick={() => log(`${label} (${variant}, icon)`)}>
                    {label}
                  </Button>
                </td>
                <td>
                  <Button variant={variant} loading>
                    {label}
                  </Button>
                </td>
                <td>
                  <Button variant={variant} disabled>
                    {label}
                  </Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <Row label="Loading on click">
        <Button variant="primary" icon={<Terminal />} loading={running} onClick={run}>
          Run
        </Button>
      </Row>
      <Row label="Full width">
        <div className={styles.narrow}>
          <Button variant="primary" fullWidth onClick={() => log('Continue')}>
            Continue
          </Button>
        </div>
      </Row>
    </Specimen>
  );
}

// --- SplitButton ----------------------------------------------------------

function splitItems(log: Log): SplitButtonItem[] {
  return [
    { id: 'group', label: 'New group', icon: <Folder />, onSelect: () => log('New group') },
    { id: 'key', label: 'New key', icon: <KeyRound />, onSelect: () => log('New key') },
    { id: 'import', label: 'Import', icon: <Upload />, disabled: true, onSelect: () => log('Import') },
    { id: 'clear', label: 'Remove selected', icon: <Trash />, danger: true, onSelect: () => log('Remove selected') },
  ];
}

function SplitButtonDemo({ log }: DemoProps) {
  const items = splitItems(log);
  return (
    <Specimen
      id="split-button"
      title="SplitButton"
      description="A main action plus an arrow with related actions. The menu has a disabled and a danger item; arrow keys, Home, End and type-ahead move between items."
    >
      <Row label="Primary">
        <SplitButton icon={<Plus />} items={items} onClick={() => log('New host')} data-testid="split-primary">
          New host
        </SplitButton>
      </Row>
      <Row label="Secondary">
        <SplitButton variant="secondary" items={items} onClick={() => log('Connect')}>
          Connect
        </SplitButton>
      </Row>
      <Row label="Menu aligned to the end">
        <SplitButton variant="secondary" menuAlign="end" items={items} onClick={() => log('Duplicate')}>
          Duplicate
        </SplitButton>
      </Row>
      <Row label="Disabled">
        <SplitButton icon={<Plus />} items={items} disabled>
          New host
        </SplitButton>
      </Row>
    </Specimen>
  );
}

// --- IconButton -----------------------------------------------------------

const ICON_BUTTON_SAMPLES: readonly { variant: IconButtonVariant; label: string; icon: ReactNode }[] = [
  { variant: 'ghost', label: 'Settings', icon: <Settings /> },
  { variant: 'secondary', label: 'Copy', icon: <Copy /> },
  { variant: 'danger', label: 'Remove', icon: <Trash /> },
];

function PressableIconButton({ variant, label, icon }: { variant: IconButtonVariant; label: string; icon: ReactNode }) {
  const [pressed, setPressed] = useState(true);
  return (
    <IconButton
      variant={variant}
      icon={icon}
      aria-label={`${label}, toggle`}
      pressed={pressed}
      onClick={() => setPressed((value) => !value)}
    />
  );
}

function IconButtonDemo({ log }: DemoProps) {
  const [view, setView] = useState<'grid' | 'list'>('grid');
  return (
    <Specimen
      id="icon-button"
      title="IconButton"
      description="A square button with only an icon; the accessible name comes from aria-label. Small buttons fit inside fields and dense rows."
    >
      <div className={styles.tableScroll}>
        <table className={styles.matrix}>
          <thead>
            <tr>
              <th scope="col">Variant</th>
              <th scope="col">Medium</th>
              <th scope="col">Small</th>
              <th scope="col">Pressed</th>
              <th scope="col">Disabled</th>
            </tr>
          </thead>
          <tbody>
            {ICON_BUTTON_SAMPLES.map(({ variant, label, icon }) => (
              <tr key={variant}>
                <th scope="row">
                  <code>{variant}</code>
                </th>
                <td>
                  <IconButton variant={variant} icon={icon} aria-label={label} onClick={() => log(`${label} (${variant})`)} />
                </td>
                <td>
                  <IconButton
                    variant={variant}
                    size="sm"
                    icon={icon}
                    aria-label={`${label}, small`}
                    onClick={() => log(`${label} (${variant}, small)`)}
                  />
                </td>
                <td>
                  <PressableIconButton variant={variant} label={label} icon={icon} />
                </td>
                <td>
                  <IconButton variant={variant} icon={icon} aria-label={`${label}, disabled`} disabled />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <Row label="View switch (pressed)">
        <div className={styles.inlineGroup}>
          <IconButton
            icon={<LayoutGrid />}
            aria-label="Grid view"
            pressed={view === 'grid'}
            onClick={() => setView('grid')}
          />
          <IconButton icon={<List />} aria-label="List view" pressed={view === 'list'} onClick={() => setView('list')} />
        </div>
      </Row>
    </Specimen>
  );
}

// --- TextInput ------------------------------------------------------------

function TextInputDemo({ log }: DemoProps) {
  return (
    <Specimen
      id="text-input"
      title="TextInput"
      description="A single-line field with an optional label, helper text and error message."
    >
      <div className={styles.fields}>
        <TextInput label="Label" placeholder="Production web" />
        <TextInput label="Address" placeholder="10.0.0.5" description="IP address or host name" />
        <TextInput
          label="Port"
          type="number"
          defaultValue="70000"
          error="Enter a port from 1 to 65535"
          description="Default: 22"
        />
        <TextInput label="Username" defaultValue="root " description="Invalid without a message" error />
        <TextInput label="Startup command" mono defaultValue="tmux attach || tmux new" />
        <TextInput type="search" aria-label="Search hosts" placeholder="Search" leadingIcon={<Search />} />
        <TextInput
          label="Fingerprint"
          mono
          readOnly
          defaultValue="SHA256:1n2A3d9xQe0R7k"
          trailing={<IconButton size="sm" icon={<Copy />} aria-label="Copy fingerprint" onClick={() => log('Copy fingerprint')} />}
        />
        <TextInput label="Disabled" defaultValue="Read-only value" disabled />
      </div>
    </Specimen>
  );
}

// --- PasswordInput --------------------------------------------------------

function PasswordInputDemo() {
  return (
    <Specimen
      id="password-input"
      title="PasswordInput"
      description="Masked by default; the toggle shows the value. Autocomplete, spell check and autocorrection are off."
    >
      <div className={styles.fields}>
        <PasswordInput label="Password" defaultValue="not-a-real-secret" />
        <PasswordInput label="Passphrase" defaultValue="not-a-real-secret" defaultVisible description="Starts visible" />
        <PasswordInput label="Master password" defaultValue="short" error="Use at least 12 characters" />
        <PasswordInput label="Disabled" defaultValue="not-a-real-secret" disabled />
      </div>
    </Specimen>
  );
}

// --- Toggle ---------------------------------------------------------------

function ToggleDemo({ log }: DemoProps) {
  return (
    <Specimen id="toggle" title="Toggle" description="An on/off switch. On uses the accent track, off the strong border track.">
      <div className={styles.fields}>
        <Toggle label="Agent forwarding" onCheckedChange={(checked) => log(`Agent forwarding ${checked ? 'on' : 'off'}`)} />
        <Toggle label="Keepalive" defaultChecked />
        <Toggle label="Reconnect automatically" description="Reconnect when the connection drops" defaultChecked />
        <Toggle label="Label at the start" labelPosition="start" />
        <Toggle label="Disabled, off" disabled />
        <Toggle label="Disabled, on" disabled defaultChecked />
      </div>
    </Specimen>
  );
}

// --- Select ---------------------------------------------------------------

type AutoLockDemoValue = '1' | '5' | '15' | '30' | '60' | 'never';

const AUTO_LOCK_DEMO_OPTIONS: readonly { value: AutoLockDemoValue; label: string }[] = [
  { value: '1', label: '1 minute' },
  { value: '5', label: '5 minutes' },
  { value: '15', label: '15 minutes' },
  { value: '30', label: '30 minutes' },
  { value: '60', label: '60 minutes' },
  { value: 'never', label: 'Never' },
];

function SelectDemo({ log }: DemoProps) {
  const [value, setValue] = useState<AutoLockDemoValue>('15');
  return (
    <Specimen id="select" title="Select" description="A dropdown on the native select element: platform keyboard and screen reader support.">
      <div className={styles.fields}>
        <Select
          label="Auto-lock"
          description="Lock after this long without input"
          options={AUTO_LOCK_DEMO_OPTIONS}
          value={value}
          onValueChange={(next) => {
            setValue(next);
            log(`Auto-lock ${next}`);
          }}
        />
        <Select label="With an error" options={AUTO_LOCK_DEMO_OPTIONS} value="5" onValueChange={noop} error="This value is not valid." />
        <Select label="Disabled" options={AUTO_LOCK_DEMO_OPTIONS} value="30" onValueChange={noop} disabled />
      </div>
    </Specimen>
  );
}

// --- Checkbox -------------------------------------------------------------

function CheckboxDemo({ log }: DemoProps) {
  const [saved, setSaved] = useState(false);
  return (
    <Specimen id="checkbox" title="Checkbox" description="A native checkbox with a label, for required confirmations.">
      <div className={styles.fields}>
        <Checkbox
          label="I saved the recovery key"
          checked={saved}
          onCheckedChange={(checked) => {
            setSaved(checked);
            log(`Saved ${checked ? 'checked' : 'unchecked'}`);
          }}
        />
        <Checkbox label="Disabled, checked" checked onCheckedChange={noop} disabled />
      </div>
    </Specimen>
  );
}

// --- SegmentedControl -----------------------------------------------------

const SORT_OPTIONS: readonly SegmentedControlOption<'name' | 'address' | 'recent'>[] = [
  { value: 'name', label: 'Name' },
  { value: 'address', label: 'Address' },
  { value: 'recent', label: 'Last used' },
];

const VIEW_OPTIONS: readonly SegmentedControlOption<'grid' | 'list'>[] = [
  { value: 'grid', label: 'Grid', icon: <LayoutGrid /> },
  { value: 'list', label: 'List', icon: <List /> },
];

const VIEW_ICON_OPTIONS: readonly SegmentedControlOption<'grid' | 'list'>[] = [
  { value: 'grid', icon: <LayoutGrid />, 'aria-label': 'Grid' },
  { value: 'list', icon: <List />, 'aria-label': 'List' },
];

const CURSOR_OPTIONS: readonly SegmentedControlOption<'block' | 'bar' | 'underline'>[] = [
  { value: 'block', label: 'Block' },
  { value: 'bar', label: 'Bar' },
  { value: 'underline', label: 'Underline', disabled: true },
];

function SegmentedControlDemo() {
  return (
    <Specimen
      id="segmented-control"
      title="SegmentedControl"
      description="Mutually exclusive segments with radio group semantics. Arrow keys move the selection; only the selected segment is in the tab order."
    >
      <Row label="Text">
        <SegmentedControl aria-label="Sort by" options={SORT_OPTIONS} defaultValue="name" />
      </Row>
      <Row label="Icon and text">
        <SegmentedControl aria-label="View" options={VIEW_OPTIONS} defaultValue="grid" />
      </Row>
      <Row label="Icons only">
        <SegmentedControl aria-label="View, icons only" options={VIEW_ICON_OPTIONS} defaultValue="list" />
      </Row>
      <Row label="Disabled segment">
        <SegmentedControl aria-label="Cursor style" options={CURSOR_OPTIONS} defaultValue="block" />
      </Row>
      <Row label="Disabled">
        <SegmentedControl aria-label="Sort by, disabled" options={SORT_OPTIONS} defaultValue="address" disabled />
      </Row>
      <Row label="Full width">
        <div className={styles.narrow}>
          <SegmentedControl aria-label="Sort by, full width" options={SORT_OPTIONS} defaultValue="recent" fullWidth />
        </div>
      </Row>
    </Specimen>
  );
}

// --- Chip -----------------------------------------------------------------

const INITIAL_TAGS: readonly string[] = ['staging', 'eu-west', 'docker'];
const FILTERS: readonly string[] = ['web', 'db', 'cache'];

function ChipDemo({ log }: DemoProps) {
  const [tags, setTags] = useState<readonly string[]>(INITIAL_TAGS);
  const [filters, setFilters] = useState<ReadonlySet<string>>(() => new Set(['web']));

  function toggleFilter(filter: string) {
    setFilters((previous) => {
      const next = new Set(previous);
      if (next.has(filter)) {
        next.delete(filter);
      } else {
        next.add(filter);
      }
      return next;
    });
  }

  function removeTag(tag: string) {
    setTags((previous) => previous.filter((item) => item !== tag));
    log(`Removed tag ${tag}`);
  }

  return (
    <Specimen
      id="chip"
      title="Chip"
      description="Tag chips are neutral. Only an active filter chip gets the accent fill; tag chips never use the tile colors."
    >
      <Row label="Tag">
        <Chip label="production" />
        <Chip label="linux" />
      </Row>
      <Row label="Removable">
        {tags.map((tag) => (
          <Chip key={tag} label={tag} onRemove={() => removeTag(tag)} />
        ))}
        {tags.length < INITIAL_TAGS.length ? (
          <Button variant="text" onClick={() => setTags(INITIAL_TAGS)}>
            Restore tags
          </Button>
        ) : null}
      </Row>
      <Row label="Filter (toggle)">
        {FILTERS.map((filter) => (
          <Chip key={filter} label={filter} active={filters.has(filter)} onClick={() => toggleFilter(filter)} />
        ))}
      </Row>
      <Row label="Active filter, removable">
        <Chip label="ssh" active onClick={noop} onRemove={() => log('Removed filter ssh')} />
      </Row>
      <Row label="Disabled">
        <Chip label="archived" disabled />
        <Chip label="archived" disabled onRemove={noop} />
        <Chip label="web" disabled active onClick={noop} />
      </Row>
    </Specimen>
  );
}

// --- StatusDot ------------------------------------------------------------

function StatusDotDemo() {
  return (
    <Specimen
      id="status-dot"
      title="StatusDot"
      description="An 8 px dot. Alone it is labelled for assistive technology; next to text it is decorative."
    >
      <Row label="Labelled">
        {STATUSES.map((status) => (
          <StatusDot key={status} status={status} />
        ))}
      </Row>
      <Row label="Next to text">
        {STATUSES.map((status) => (
          <span key={status} className={styles.statusText}>
            <StatusDot status={status} decorative />
            {STATUS_DOT_LABELS[status]}
          </span>
        ))}
      </Row>
    </Specimen>
  );
}

// --- HostCard -------------------------------------------------------------

interface CardSample {
  readonly id: string;
  readonly title: string;
  readonly card: HostCardProps;
}

const CARD_SAMPLES: readonly CardSample[] = [
  {
    id: 'web-1',
    title: 'web-1',
    card: {
      label: 'web-1',
      address: '10.0.0.5',
      subtitle: 'SSH, deploy',
      color: 'jade',
      trailing: <StatusDot status="connected" />,
    },
  },
  {
    id: 'address-only',
    title: '192.168.1.20',
    card: { address: '192.168.1.20', subtitle: 'SSH, root' },
  },
  {
    id: 'db-primary',
    title: 'db-primary',
    card: {
      label: 'db-primary',
      address: '10.0.2.10',
      subtitle: 'SSH, postgres, Production',
      icon: Database,
      color: 'ocean',
      trailing: <StatusDot status="connecting" />,
    },
  },
  {
    id: 'build-agent',
    title: 'build-agent',
    card: {
      label: 'build-agent',
      address: '10.0.3.7',
      subtitle: 'SSH, ci, Build',
      icon: Cpu,
      color: 'amber',
      trailing: <StatusDot status="error" />,
    },
  },
  { id: 'production', title: 'Production', card: { variant: 'group', label: 'Production', hostCount: 12, color: 'coral' } },
  { id: 'lab', title: 'Lab', card: { variant: 'group', label: 'Lab', hostCount: 1, color: 'moss' } },
  {
    id: 'long-label',
    title: 'reporting-and-analytics-worker-eu-west-3',
    card: {
      label: 'reporting-and-analytics-worker-eu-west-3',
      address: '10.0.9.31',
      subtitle: 'SSH, analytics, Reporting, Long subtitles are truncated',
      color: 'rose',
    },
  },
  {
    id: 'legacy',
    title: 'legacy-box',
    card: { label: 'legacy-box', address: '10.0.0.99', subtitle: 'Disabled card', color: 'plum', disabled: true },
  },
];

function HostCardDemo({ log }: DemoProps) {
  const [selected, setSelected] = useState<ReadonlySet<string>>(() => new Set(['web-1']));

  function select(id: string, event: HostCardEvent) {
    const toggle = event.ctrlKey || event.metaKey;
    setSelected((previous) => {
      if (!toggle) {
        return new Set([id]);
      }
      const next = new Set(previous);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }

  return (
    <Specimen
      id="host-card"
      title="HostCard"
      description="Host and group cards with all eight tile colors. Click or Space selects, Ctrl+click adds to the selection, double click or Enter opens."
    >
      <div role="listbox" aria-label="Hosts" aria-multiselectable="true" className={styles.cards}>
        {CARD_SAMPLES.map(({ id, title, card }) => (
          <HostCard
            key={id}
            {...card}
            selected={selected.has(id)}
            onSelect={(event) => select(id, event)}
            onOpen={() => log(`Open ${title}`)}
            data-testid={`host-card-${id}`}
          />
        ))}
      </div>
    </Specimen>
  );
}

// --- Menu -----------------------------------------------------------------

const HOST_MENU: readonly MenuEntry[] = [
  { id: 'connect', label: 'Connect', icon: Terminal, shortcut: 'Enter' },
  { id: 'edit', label: 'Edit', icon: Pencil, shortcut: 'Ctrl+E' },
  { id: 'duplicate', label: 'Duplicate', icon: Copy, shortcut: 'Ctrl+D' },
  { id: 'move', label: 'Move to', icon: FolderInput, submenu: true },
  { type: 'separator', id: 'separator-1' },
  { id: 'share', label: 'Share', icon: Link, disabled: true },
  { type: 'separator', id: 'separator-2' },
  { id: 'remove', label: 'Remove', icon: Trash, danger: true, shortcut: 'Del' },
];

const TERMINAL_MENU: readonly MenuEntry[] = [
  { id: 'copy', label: 'Copy', shortcut: 'Ctrl+Shift+C' },
  { id: 'paste', label: 'Paste', shortcut: 'Ctrl+Shift+V' },
  { id: 'select-all', label: 'Select all' },
  { id: 'find', label: 'Find', shortcut: 'Ctrl+Shift+F', disabled: true },
  { type: 'separator', id: 'separator' },
  { id: 'close', label: 'Close', danger: true },
];

function PopoverMenu({ log }: DemoProps) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  return (
    <div className={styles.popoverAnchor}>
      <Button
        ref={triggerRef}
        icon={<Ellipsis />}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        Actions
      </Button>
      {open ? (
        <Menu
          className={styles.popover}
          items={HOST_MENU}
          anchorRef={triggerRef}
          aria-label="Actions"
          onAction={(id) => log(`Menu: ${id}`)}
          onClose={() => setOpen(false)}
          data-testid="popover-menu"
        />
      ) : null}
    </div>
  );
}

function MenuDemo({ log }: DemoProps) {
  return (
    <Specimen
      id="menu"
      title="Menu"
      description="Items with icons and shortcut hints, separators, a submenu item, an inactive item and a danger item. The open menus below do not take focus; the Actions button opens a real popover."
    >
      <div className={styles.menus}>
        <Menu items={HOST_MENU} autoFocus={false} aria-label="Host actions" onAction={(id) => log(`Menu: ${id}`)} />
        <Menu items={TERMINAL_MENU} autoFocus={false} aria-label="Terminal actions" onAction={(id) => log(`Menu: ${id}`)} />
        <PopoverMenu log={log} />
      </div>
    </Specimen>
  );
}

// --- Modal ----------------------------------------------------------------

type ModalDemoId = 'confirm' | 'form' | 'content';

function ModalDemo({ log }: DemoProps) {
  const [openId, setOpenId] = useState<ModalDemoId | null>(null);
  const formId = useId();
  const close = () => setOpenId(null);

  return (
    <Specimen
      id="modal"
      title="Modal"
      description="Scrim, title, close button, content and an optional footer. Escape closes; Tab stays inside; focus returns to the button that opened it."
    >
      <Row label="Sizes">
        <Button onClick={() => setOpenId('confirm')}>Small: confirmation</Button>
        <Button onClick={() => setOpenId('form')}>Medium: form</Button>
        <Button onClick={() => setOpenId('content')}>Large: no footer</Button>
      </Row>

      <Modal
        open={openId === 'confirm'}
        size="small"
        title="Remove host?"
        onClose={close}
        data-testid="modal-confirm"
        footer={
          <>
            <Button onClick={close}>Cancel</Button>
            <Button
              variant="danger"
              onClick={() => {
                log('Removed host web-1');
                close();
              }}
            >
              Remove
            </Button>
          </>
        }
      >
        <p className={styles.modalText}>The host web-1 and its port forwarding rules are removed from this device.</p>
      </Modal>

      <Modal
        open={openId === 'form'}
        title="New group"
        onClose={close}
        data-testid="modal-form"
        footer={
          <>
            <Button onClick={close}>Cancel</Button>
            <Button variant="primary" type="submit" form={formId}>
              Create
            </Button>
          </>
        }
      >
        <form
          id={formId}
          className={styles.modalForm}
          onSubmit={(event) => {
            event.preventDefault();
            log('Created group');
            close();
          }}
        >
          <TextInput label="Name" placeholder="Production" />
          <Toggle label="Collapse in the sidebar" />
        </form>
      </Modal>

      <Modal
        open={openId === 'content'}
        size="large"
        title="Terminal themes"
        onClose={close}
        closeOnScrimClick
        data-testid="modal-content"
      >
        <p className={styles.modalText}>
          A large modal window without a footer. It also closes on a click on the scrim, which is off by default.
        </p>
        <p className={styles.modalText}>
          The terminal theme is chosen per host. Graphite is the default; Paper is the light theme. Imported themes are
          converted to the same format.
        </p>
      </Modal>
    </Specimen>
  );
}

// --- Both themes ----------------------------------------------------------

function ThemePanel({ theme }: { theme: ThemeName }) {
  const title = theme === 'dark' ? 'Dark' : 'Light';
  return (
    <div data-theme={theme} className={styles.themePanel} role="group" aria-label={`${title} theme`}>
      <h4 className={styles.panelTitle}>{title}</h4>
      <div className={styles.panelRow}>
        <Button variant="primary">Connect</Button>
        <Button>Cancel</Button>
        <Button variant="danger">Remove</Button>
        <Button variant="text">Add targets</Button>
      </div>
      <TextInput label="Address" placeholder="10.0.0.5" description="IP address or host name" />
      <div className={styles.panelRow}>
        <Toggle label="Agent forwarding" defaultChecked />
        <Chip label="production" />
        <Chip label="web" active onClick={noop} />
      </div>
      <div className={styles.panelRow}>
        {STATUSES.map((status) => (
          <span key={status} className={styles.statusText}>
            <StatusDot status={status} decorative />
            {STATUS_DOT_LABELS[status]}
          </span>
        ))}
      </div>
      <div role="listbox" aria-label={`Hosts, ${theme} theme`} className={styles.cards}>
        <HostCard label="web-1" address="10.0.0.5" subtitle="SSH, deploy" color="jade" selected />
        <HostCard variant="group" label="Production" hostCount={12} color="coral" />
      </div>
    </div>
  );
}

function BothThemesDemo() {
  return (
    <Specimen
      id="both-themes"
      title="Both themes"
      description="data-theme on an inner element shows part of the page in a fixed theme, whatever the page theme is."
    >
      <div className={styles.themePanels}>
        <ThemePanel theme="dark" />
        <ThemePanel theme="light" />
      </div>
    </Specimen>
  );
}

// --- Section --------------------------------------------------------------

/** Ids and titles of the component specimens, in page order (for the navigation). */
export const COMPONENT_SPECIMENS: readonly { id: string; title: string }[] = [
  { id: 'button', title: 'Button' },
  { id: 'split-button', title: 'SplitButton' },
  { id: 'icon-button', title: 'IconButton' },
  { id: 'text-input', title: 'TextInput' },
  { id: 'password-input', title: 'PasswordInput' },
  { id: 'toggle', title: 'Toggle' },
  { id: 'segmented-control', title: 'SegmentedControl' },
  { id: 'select', title: 'Select' },
  { id: 'checkbox', title: 'Checkbox' },
  { id: 'chip', title: 'Chip' },
  { id: 'status-dot', title: 'StatusDot' },
  { id: 'host-card', title: 'HostCard' },
  { id: 'menu', title: 'Menu' },
  { id: 'modal', title: 'Modal' },
  { id: 'both-themes', title: 'Both themes' },
];

interface ComponentsProps {
  /** Called with a short description of each action triggered in a demo. */
  onAction: Log;
}

export function Components({ onAction: log }: ComponentsProps) {
  return (
    <Section
      id="components"
      title="Components"
      description="Hover, press and keyboard focus are live: point at a component or move to it with Tab. The last action triggered in a demo is shown in the top bar."
    >
      <ButtonDemo log={log} />
      <SplitButtonDemo log={log} />
      <IconButtonDemo log={log} />
      <TextInputDemo log={log} />
      <PasswordInputDemo />
      <ToggleDemo log={log} />
      <SegmentedControlDemo />
      <SelectDemo log={log} />
      <CheckboxDemo log={log} />
      <ChipDemo log={log} />
      <StatusDotDemo />
      <HostCardDemo log={log} />
      <MenuDemo log={log} />
      <ModalDemo log={log} />
      <BothThemesDemo />
    </Section>
  );
}
