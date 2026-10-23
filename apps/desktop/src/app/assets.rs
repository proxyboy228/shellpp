//! Embedded assets: gpui-kit's default component icons plus the Lucide icons
//! the app uses beyond them (embedding the whole catalog would add every
//! icon to the binary), and the app's own icons. Add an icon here before
//! using it in a view.
//!
//! The vault icon (owner's step 1.2): Lucide's `vault` draws four bolts that
//! read as a cross ("close") at 16 px. The app serves its own safe drawing
//! (`assets/ui-icons/vault.svg`: a dial, a handle and feet on the Lucide
//! 24 px grid) under the path of [`IconName::Vault`], so every
//! `IconName::Vault` in the app shows the safe: the Vaults tab and the Vault
//! selectors of the host and group panels. [`vault_icon`] is the helper for
//! new code.
//!
//! The Linux icon (stage 2b, owner's step 0.1): Lucide has no penguin, and
//! the terminal glyph it replaces is the session type icon of a tab (UI
//! 1.1). The app serves its own simple penguin (`assets/ui-icons/linux.svg`,
//! on the Lucide grid; not a logo) under the path of [`LINUX_ICON`]
//! (Lucide's `bird`, which the app does not use otherwise).

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;

gpui_kit::assets::icon_assets!(pub AppIcons, [
    ArrowLeft,
    ArrowLeftRight,
    ArrowRight,
    BrickWall,
    Database,
    Globe,
    ArrowUpDown,
    Bookmark,
    BookmarkCheck,
    Braces,
    BrushCleaning,
    CornerLeftUp,
    Download,
    FileSymlink,
    FingerprintPattern,
    Folder,
    FolderOpen,
    Import,
    Key,
    KeyRound,
    LayoutDashboard,
    LayoutGrid,
    List,
    Lock,
    Monitor,
    Pencil,
    Play,
    Plus,
    RadioTower,
    RotateCcw,
    RotateCcwClock,
    ScrollText,
    Server,
    Square,
    SquareTerminal,
    Tag,
    Trash,
    Type,
    Upload,
    Usb,
    Wrench,
]);

/// The app's vault (safe) icon, served under the path of [`IconName::Vault`].
pub const VAULT_SVG: &[u8] = include_bytes!("../../assets/ui-icons/vault.svg");

/// The OS icon of Linux hosts: the app's penguin, served under this name's
/// path.
pub const LINUX_ICON: IconName = IconName::Bird;

/// The app's penguin, served under the path of [`LINUX_ICON`].
pub const LINUX_SVG: &[u8] = include_bytes!("../../assets/ui-icons/linux.svg");

/// The vault icon of the Vaults tab and the Vault selectors (UI 1.1, 3.1, 3.4).
pub fn vault_icon() -> Icon {
    Icon::new(IconName::Vault)
}

/// The asset source of the app: our own icons, then [`AppIcons`], then
/// gpui-kit's defaults.
#[derive(Clone, Copy, Debug, Default)]
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == IconName::Vault.path().as_ref() {
            return Ok(Some(Cow::Borrowed(VAULT_SVG)));
        }
        if path == LINUX_ICON.path().as_ref() {
            return Ok(Some(Cow::Borrowed(LINUX_SVG)));
        }
        if let Some(bytes) = AppIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut names = AppIcons.list(path)?;
        for own in [IconName::Vault.path(), LINUX_ICON.path()] {
            if own.starts_with(path) && !names.contains(&own) {
                names.push(own);
            }
        }
        for name in gpui_kit::assets::Assets.list(path)? {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_icons_and_default_icons_load() {
        for icon in [
            IconName::Server,
            IconName::FingerprintPattern,
            IconName::Braces,
            IconName::Import,
            IconName::Upload,
            IconName::Trash,
        ] {
            let path = icon.path();
            assert!(AppAssets.load(&path).unwrap().is_some(), "{path}");
        }
        // A default component icon still comes from gpui-kit's source.
        assert!(AppAssets.load("icons/search.svg").unwrap().is_some());
    }

    /// Every icon the crate names loads from [`AppAssets`]: an icon missing
    /// from [`AppIcons`] and from gpui-kit's defaults renders blank, and a
    /// test that clicks by element id does not notice. The scan reads the
    /// crate's sources and looks for `IconName::X` and its local aliases
    /// (`use ... IconName as Alias`).
    #[test]
    fn every_icon_the_crate_names_loads() {
        fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    sources(&path, out);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    out.push(path);
                }
            }
        }
        /// The identifiers that follow `prefix` in `text`.
        fn idents_after<'a>(text: &'a str, prefix: &'a str) -> impl Iterator<Item = &'a str> {
            text.match_indices(prefix).filter_map(move |(at, _)| {
                let before = text[..at].chars().next_back();
                if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                    return None;
                }
                let rest = &text[at + prefix.len()..];
                let end = rest
                    .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                    .unwrap_or(rest.len());
                Some(&rest[..end])
            })
        }

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        sources(&root, &mut files);
        let mut named = std::collections::BTreeSet::new();
        for file in &files {
            let text = std::fs::read_to_string(file).unwrap();
            let mut prefixes = vec!["IconName::".to_owned()];
            prefixes.extend(
                idents_after(&text, "IconName as ")
                    .filter(|alias| !alias.is_empty())
                    .map(|alias| format!("{alias}::")),
            );
            for prefix in &prefixes {
                for name in idents_after(&text, prefix) {
                    // Variants only: `IconName::ALL`, `IconName::path` and
                    // the like are not icons.
                    if name.starts_with(|c: char| c.is_ascii_uppercase())
                        && name.chars().any(|c| c.is_ascii_lowercase())
                    {
                        named.insert(name.to_owned());
                    }
                }
            }
        }
        assert!(
            named.contains("Import") && named.contains("Trash"),
            "{named:?}"
        );

        let mut missing = Vec::new();
        for name in &named {
            let icon = IconName::ALL
                .iter()
                .copied()
                .find(|icon| format!("{icon:?}") == *name)
                .unwrap_or_else(|| panic!("{name} is not a Lucide icon name"));
            if AppAssets.load(&icon.path()).unwrap().is_none() {
                missing.push(name.clone());
            }
        }
        assert!(
            missing.is_empty(),
            "add these icons to AppIcons: {missing:?}"
        );
    }

    /// Owner's step 1.2: the vault icon is our safe, not Lucide's bolts.
    #[test]
    fn the_vault_icon_is_the_safe() {
        let path = IconName::Vault.path();
        let served = AppAssets.load(&path).unwrap().expect("vault icon");
        assert_eq!(served.as_ref(), VAULT_SVG);
        let svg = std::str::from_utf8(VAULT_SVG).unwrap();
        assert!(svg.contains("<circle cx=\"10.5\""), "the dial");
        assert!(!svg.contains("m7.9 7.9"), "no bolt that reads as a cross");
        assert!(AppAssets.list("icons/").unwrap().contains(&path));
        // The helper names the same icon.
        let _ = vault_icon();
    }

    /// Owner's stage 2b step 0.1: Linux hosts get the app's penguin, never
    /// the terminal glyph of the session type icon.
    #[test]
    fn the_linux_icon_is_the_penguin() {
        let path = LINUX_ICON.path();
        let served = AppAssets.load(&path).unwrap().expect("linux icon");
        assert_eq!(served.as_ref(), LINUX_SVG);
        assert_ne!(
            served.as_ref(),
            AppAssets
                .load(&IconName::SquareTerminal.path())
                .unwrap()
                .expect("terminal icon")
                .as_ref()
        );
        assert!(AppAssets.list("icons/").unwrap().contains(&path));
    }
}
