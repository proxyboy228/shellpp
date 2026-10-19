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

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;

gpui_kit::assets::icon_assets!(pub AppIcons, [
    ArrowLeftRight,
    ArrowUpDown,
    Braces,
    FingerprintPattern,
    Folder,
    FolderOpen,
    Key,
    KeyRound,
    LayoutGrid,
    List,
    Lock,
    Monitor,
    Plus,
    ScrollText,
    Server,
    SquareTerminal,
    Tag,
    Usb,
]);

/// The app's vault (safe) icon, served under the path of [`IconName::Vault`].
pub const VAULT_SVG: &[u8] = include_bytes!("../../assets/ui-icons/vault.svg");

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
        if let Some(bytes) = AppIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut names = AppIcons.list(path)?;
        let vault: SharedString = IconName::Vault.path();
        if vault.starts_with(path) && !names.contains(&vault) {
            names.push(vault);
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
        ] {
            let path = icon.path();
            assert!(AppAssets.load(&path).unwrap().is_some(), "{path}");
        }
        // A default component icon still comes from gpui-kit's source.
        assert!(AppAssets.load("icons/search.svg").unwrap().is_some());
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
}
