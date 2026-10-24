//! The run 9 certificate states of the screenshot run (plan 1.3, 3.3;
//! D187): the dialog of a changed pinned certificate whose new certificate
//! the system trusts (both fingerprints, "Use the system's trust"), and the
//! confirmation of a CA file in the wizard's certificate step, and the
//! confirmation of weaker KDF parameters at a sign-in (D189). Made-up
//! certificates and parameters: no server is asked and nothing is stored.

use gpui::{AnyWindowHandle, AppContext as _, AsyncApp, Entity};
use gpui_kit::test::TestWindowExt as _;

use super::run7_shot::{bound_status, certificate};
use super::{ScreenshotError, ShotState, settle, shell_of};
use crate::app::RootView;
use crate::state::AppState;

/// The states of this module.
pub const STATES: [ShotState; 3] = [
    ShotState::SyncCertificateChanged,
    ShotState::SyncTrustCa,
    ShotState::SyncWeakerKdf,
];

const ORIGIN: &str = "https://sync.home.arpa";

fn info(subject: &str, issuer: &str, seed: u8) -> shellpp_core::sync::CertInfo {
    let der = vec![0x30, 0x03, 0x02, 0x01, seed];
    shellpp_core::sync::CertInfo {
        subject: subject.into(),
        issuer: issuer.into(),
        not_before: "2026-10-24T00:00:00Z".into(),
        not_after: "2027-01-22T00:00:00Z".into(),
        sha256: shellpp_core::sync::tls::sha256(&der),
        der,
    }
}

/// From the reset demo window: the state's own steps.
pub async fn reach(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    state: &Entity<AppState>,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    shell_of(cx, root, shot)?;
    let state = state.clone();
    match shot {
        ShotState::SyncCertificateChanged => {
            cx.update_window(handle, move |_, window, cx| {
                let mut status = bound_status();
                status.server_url = Some(ORIGIN.to_owned());
                status.last_error = Some("certificate_changed".to_owned());
                state.update(cx, |state, cx| state.set_sync_status(status, cx));
                let problem = shellpp_core::sync::TlsProblem::PinChanged {
                    pinned: info("CN=sync.home.arpa", "CN=sync.home.arpa", 1).sha256,
                    new: info("CN=sync.home.arpa", "CN=R11,O=Let's Encrypt,C=US", 2),
                    system_trusts: true,
                };
                let done: crate::sync_ui::trust::Trusted = std::rc::Rc::new(|_, _, _| {});
                crate::sync_ui::trust::open_trust(
                    state,
                    ORIGIN.to_owned(),
                    problem,
                    done,
                    window,
                    cx,
                );
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
            settle(cx, handle).await
        }
        ShotState::SyncTrustCa => {
            cx.update_window(handle, move |_, window, cx| {
                let dialog = crate::sync_ui::open_connect(state, false, window, cx);
                dialog.update(cx, |dialog, cx| {
                    dialog.show_trust(ORIGIN.to_owned(), certificate(), cx);
                    if let Some(panel) = dialog.trust.clone() {
                        panel.update(cx, |panel, cx| {
                            let info = info(
                                "CN=Caddy Local Authority - 2026 ECC Root",
                                "CN=Caddy Local Authority - 2026 ECC Root",
                                3,
                            );
                            panel.ca = Some(crate::sync_ui::trust::CaCandidate {
                                der: info.der.clone(),
                                info,
                            });
                            cx.notify();
                        });
                    }
                });
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
            settle(cx, handle).await
        }
        ShotState::SyncWeakerKdf => {
            cx.update_window(handle, move |_, window, cx| {
                let wizard = crate::sync_ui::open_connect(state, false, window, cx);
                let offer = shellpp_core::service::WeakerKdf {
                    local: shellpp_core::crypto::KdfParams::preset(
                        shellpp_core::crypto::PlatformClass::Desktop,
                    ),
                    server: shellpp_core::crypto::KdfParams::preset(
                        shellpp_core::crypto::PlatformClass::Mobile,
                    ),
                };
                crate::sync_ui::kdf::open_weaker_kdf(wizard, offer, window, cx);
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
            settle(cx, handle).await
        }
        _ => Err(ScreenshotError::state(shot, "not a certificate state")),
    }
}

pub fn check(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let shown = |cx: &mut AsyncApp, id: &'static str| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(gpui::SharedString::from(id))
                .is_some_and(|found| found.visible())
        })
        .map_err(|_| ScreenshotError::WindowClosed)
    };
    match shot {
        ShotState::SyncCertificateChanged
            if shown(cx, "sync-pin-changed")? && shown(cx, "sync-use-system")? =>
        {
            Ok(())
        }
        ShotState::SyncTrustCa if shown(cx, "sync-ca-confirm")? => Ok(()),
        ShotState::SyncWeakerKdf if shown(cx, "sync-weaker-kdf")? => Ok(()),
        ShotState::SyncCertificateChanged | ShotState::SyncTrustCa | ShotState::SyncWeakerKdf => {
            Err(ScreenshotError::state(
                shot,
                "the certificate dialog is not shown",
            ))
        }
        _ => Err(ScreenshotError::state(shot, "not a certificate state")),
    }
}
