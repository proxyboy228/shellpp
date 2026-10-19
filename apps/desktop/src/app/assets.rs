//! Embedded assets: gpui-kit's default component icons plus the Lucide icons
//! the app uses beyond them (embedding the whole catalog would add every
//! icon to the binary). Add an icon here before using it in a view.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

gpui_kit::assets::icon_assets!(pub AppIcons, [
    ArrowLeftRight,
    ArrowUpDown,
    Braces,
    FingerprintPattern,
    Folder,
    FolderOpen,
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
    Vault,
]);

/// The asset source of the app: [`AppIcons`] first, then gpui-kit's defaults.
#[derive(Clone, Copy, Debug, Default)]
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = AppIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut names = AppIcons.list(path)?;
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
    use gpui_kit::assets::IconName;

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
}
