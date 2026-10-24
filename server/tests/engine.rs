//! The sync engine against the real server (docs/sync-protocol.md, sections
//! 4.4 and 6.5-6.9): findings of the run 7 report fixed in run 9.

mod support;

use shellpp_core::service::SignInOutcome;
use shellpp_server::db::{DbError, TxKind, Ty, Val};
use support::*;

fn done(outcome: SignInOutcome) {
    assert!(matches!(outcome, SignInOutcome::Done(_)), "signed in");
}

fn recovery_wrap(server: &Server) -> Vec<u8> {
    server
        .db
        .tx(TxKind::Read, |tx| -> Result<Vec<u8>, DbError> {
            let rows = tx.query("SELECT recovery_wrap FROM users", &[], &[Ty::Bytes])?;
            Ok(
                match rows
                    .into_iter()
                    .next()
                    .and_then(|row| row.into_iter().next())
                {
                    Some(Val::Bytes(bytes)) => bytes,
                    _ => Vec::new(),
                },
            )
        })
        .expect("recovery wrap read")
}

fn set_recovery_wrap(server: &Server, wrap: Vec<u8>) {
    server
        .db
        .tx(TxKind::Write, |tx| -> Result<(), DbError> {
            tx.exec("UPDATE users SET recovery_wrap = ?", &[Val::Bytes(wrap)])?;
            Ok(())
        })
        .expect("recovery wrap written");
}

/// Run 7 finding 17 (section 4.4): the server's `recovery_wrap` is checked
/// in every cycle until the check passes, not only after the first pull:
/// a wrong wrap shows the warning, and once the server holds the right one
/// (whatever replaced it) the next cycle clears it.
#[test]
fn the_recovery_wrap_is_checked_until_it_is_right() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);
    let right = recovery_wrap(&server);
    let mut wrong = right.clone();
    let last = wrong.len() - 1;
    wrong[last] ^= 1;
    set_recovery_wrap(&server, wrong.clone());

    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));
    assert!(b.status().recovery_mismatch, "a wrong wrap is reported");
    b.sync();
    assert!(b.status().recovery_mismatch, "still wrong: still reported");

    set_recovery_wrap(&server, right);
    b.sync();
    assert!(!b.status().recovery_mismatch, "checked again and right now");

    // A device that checked keeps the result until the record or the
    // epoch changes, and checks again after an unlock.
    set_recovery_wrap(&server, wrong);
    b.sync();
    assert!(!b.status().recovery_mismatch, "not fetched again");
    let b = b.reopen("B");
    b.core.unlock(password.clone()).expect("unlocked");
    b.sync();
    assert!(
        b.status().recovery_mismatch,
        "checked again after the unlock"
    );
}
