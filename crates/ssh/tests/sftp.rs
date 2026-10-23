//! The SFTP engine (night plan, stage 4; D45) against OpenSSH in Docker
//! (`SHELLPP_TEST_SSHD=openssh`; on Windows with
//! `SHELLPP_OPENSSH_SERVER_JSON`; skipped with a printed reason
//! otherwise): SFTP on the connection a shell already uses, upload and
//! download with a SHA-256 check, resume from a `.part` file, Cyrillic,
//! spaces and emoji in names, a tree of small files, permissions, rename,
//! a recursive delete that leaves the target of a symbolic link alone,
//! folder transfers of 2000 files that leave links out, and a copy from
//! one session to another (remote ↔ remote), and a download that a dropped
//! connection cut, resumed over a new connection.
//! Every file is made of random bytes at run time.

mod support;

use std::path::Path;

use sha2::{Digest as _, Sha256};
use shellpp_ssh::{AuthPlan, Connection, EntryKind, KeyCredential, Sftp, SftpError, connect};
use support::openssh::{OpenSshServer, Provider, provider};
use support::*;

/// Size of the large file (a 1 GB file is the morning's manual check).
const LARGE: usize = 24 * 1024 * 1024;
/// Files of the small-file tree.
const SMALL_FILES: usize = 300;

fn random_bytes(len: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(len + 8);
    while bytes.len() < len {
        bytes.extend_from_slice(&rand::random::<u64>().to_le_bytes());
    }
    bytes.truncate(len);
    bytes
}

fn sha256_file(path: &Path) -> Vec<u8> {
    Sha256::digest(std::fs::read(path).expect("read")).to_vec()
}

async fn key_connection(server: &OpenSshServer) -> Connection {
    let plan = AuthPlan {
        key: Some(KeyCredential {
            label: "sftp test key".to_owned(),
            private_key: secret(&server.key_user.1),
            passphrase: None,
        }),
        ..AuthPlan::default()
    };
    connect(
        spec(server.addr, Some(&server.key_user.0), plan),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect("key connection")
}

/// A fresh folder for one scenario in the user's home folder.
async fn scratch(sftp: &Sftp) -> String {
    let home = sftp.canonicalize(".").await.expect("home folder");
    let dir = shellpp_ssh::sftp::join(&home, &random_secret("sftp-test"));
    sftp.create_dir(&dir).await.expect("scratch folder");
    dir
}

async fn transfers_keep_every_byte(sftp: &Sftp, dir: &str, local: &Path) {
    let source = local.join("large.bin");
    std::fs::write(&source, random_bytes(LARGE)).unwrap();
    let remote = shellpp_ssh::sftp::join(dir, "large.bin");
    let mut seen = Vec::new();
    let sent = sftp
        .upload(&source, &remote, false, &mut |done| seen.push(done))
        .await
        .expect("upload");
    assert_eq!(sent, LARGE as u64);
    assert_eq!(seen.last().copied(), Some(LARGE as u64));
    assert!(seen.windows(2).all(|pair| pair[0] <= pair[1]));
    let entry = sftp.stat(&remote).await.unwrap();
    assert_eq!(entry.size, LARGE as u64);
    assert_eq!(entry.kind, EntryKind::File);
    let back = local.join("large.back");
    let got = sftp
        .download(&remote, &back, false, &mut |_| {})
        .await
        .expect("download");
    assert_eq!(got, LARGE as u64);
    assert_eq!(sha256_file(&back), sha256_file(&source));
    assert!(!shellpp_ssh::sftp::local_part(&back).exists());
    // The modification time comes along both ways.
    let local_time = std::fs::metadata(&source).unwrap().modified().unwrap();
    let remote_time = entry.modified.unwrap();
    let local_secs = local_time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert_eq!(remote_time, local_secs);
    let back_secs = std::fs::metadata(&back)
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert_eq!(back_secs, remote_time, "and back");
}

async fn interrupted_transfers_resume(sftp: &Sftp, dir: &str, local: &Path) {
    let data = random_bytes(3 * 1024 * 1024 + 17);
    let source = local.join("resume.bin");
    std::fs::write(&source, &data).unwrap();
    // An upload that stopped after the first megabyte left its .part.
    let remote = shellpp_ssh::sftp::join(dir, "resume.bin");
    let first = local.join("first.bin");
    std::fs::write(&first, &data[..1024 * 1024]).unwrap();
    sftp.upload(&first, &format!("{remote}.part-seed"), false, &mut |_| {})
        .await
        .unwrap();
    sftp.rename(&format!("{remote}.part-seed"), &format!("{remote}.part"))
        .await
        .unwrap();
    let mut started = None;
    sftp.upload(&source, &remote, true, &mut |done| {
        started.get_or_insert(done);
    })
    .await
    .expect("resumed upload");
    assert_eq!(started, Some(1024 * 1024), "continued from the .part");
    // A download that stopped likewise.
    let back = local.join("resume.back");
    std::fs::write(shellpp_ssh::sftp::local_part(&back), &data[..1000]).unwrap();
    let mut started = None;
    sftp.download(&remote, &back, true, &mut |done| {
        started.get_or_insert(done);
    })
    .await
    .expect("resumed download");
    assert_eq!(started, Some(1000));
    assert_eq!(std::fs::read(&back).unwrap(), data);
    // A .part of another file (its last block differs) starts over.
    let other = local.join("other.back");
    std::fs::write(shellpp_ssh::sftp::local_part(&other), vec![0x55u8; 1000]).unwrap();
    let mut started = None;
    sftp.download(&remote, &other, true, &mut |done| {
        started.get_or_insert(done);
    })
    .await
    .expect("download over a foreign .part");
    assert_eq!(started, Some(0));
    assert_eq!(std::fs::read(&other).unwrap(), data);
}

async fn names_and_many_small_files(sftp: &Sftp, dir: &str, local: &Path) {
    let name = "Отчёт за 2026 год 😀.txt";
    let source = local.join(name);
    std::fs::write(&source, "текст").unwrap();
    let remote = shellpp_ssh::sftp::join(dir, name);
    sftp.upload(&source, &remote, false, &mut |_| {})
        .await
        .unwrap();
    let back = local.join("back.txt");
    sftp.download(&remote, &back, false, &mut |_| {})
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(&back).unwrap(), "текст");

    let tree = shellpp_ssh::sftp::join(dir, "tree");
    sftp.create_dir(&tree).await.unwrap();
    let small = local.join("small.txt");
    for n in 0..SMALL_FILES {
        std::fs::write(&small, format!("file {n}")).unwrap();
        let remote = shellpp_ssh::sftp::join(&tree, &format!("f{n:04}.txt"));
        sftp.upload(&small, &remote, false, &mut |_| {})
            .await
            .unwrap();
    }
    let listed = sftp.list(dir).await.unwrap();
    assert!(listed.iter().any(|entry| entry.name == name));
    let files = sftp.list(&tree).await.unwrap();
    assert_eq!(files.len(), SMALL_FILES);
    assert!(files.iter().all(|entry| entry.kind == EntryKind::File));
}

async fn permissions_rename_and_errors(sftp: &Sftp, dir: &str) {
    let path = shellpp_ssh::sftp::join(dir, "mode.txt");
    let local = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(local.path(), "x").unwrap();
    sftp.upload(local.path(), &path, false, &mut |_| {})
        .await
        .unwrap();
    sftp.set_permissions(&path, 0o640).await.unwrap();
    assert_eq!(sftp.stat(&path).await.unwrap().permissions, Some(0o640));
    // Uploading over a private file replaces its content and keeps it
    // private (SFTP v3 cannot rename over it: the old one is put aside).
    let secret = shellpp_ssh::sftp::join(dir, "secret.env");
    sftp.upload(local.path(), &secret, false, &mut |_| {})
        .await
        .unwrap();
    sftp.set_permissions(&secret, 0o600).await.unwrap();
    std::fs::write(local.path(), "edited").unwrap();
    sftp.upload(local.path(), &secret, false, &mut |_| {})
        .await
        .expect("upload over a file");
    let replaced = sftp.stat(&secret).await.unwrap();
    assert_eq!(replaced.permissions, Some(0o600));
    assert_eq!(replaced.size, "edited".len() as u64);
    // A group-writable file stays so (the server's umask 022 alone would
    // make it 0644).
    sftp.set_permissions(&secret, 0o664).await.unwrap();
    sftp.upload(local.path(), &secret, false, &mut |_| {})
        .await
        .expect("upload over a 0664 file");
    assert_eq!(sftp.stat(&secret).await.unwrap().permissions, Some(0o664));
    // A folder of that name is never moved aside for a file.
    let folder = shellpp_ssh::sftp::join(dir, "a-folder");
    sftp.create_dir(&folder).await.unwrap();
    assert_eq!(
        sftp.upload(local.path(), &folder, false, &mut |_| {})
            .await
            .err(),
        Some(SftpError::NotAFile)
    );
    assert_eq!(sftp.stat(&folder).await.unwrap().kind, EntryKind::Dir);
    let names: Vec<String> = sftp
        .list(dir)
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    assert!(
        !names.iter().any(|name| name.starts_with("secret.env.")),
        "{names:?}"
    );
    let renamed = shellpp_ssh::sftp::join(dir, "renamed.txt");
    sftp.rename(&path, &renamed).await.unwrap();
    assert_eq!(sftp.stat(&path).await.err(), Some(SftpError::NotFound));
    assert_eq!(
        sftp.list(&shellpp_ssh::sftp::join(dir, "missing"))
            .await
            .err(),
        Some(SftpError::NotFound)
    );
    // Root's home folder is not ours.
    assert!(matches!(
        sftp.list("/root").await.err(),
        Some(SftpError::PermissionDenied | SftpError::NotFound)
    ));
}

async fn a_recursive_delete_leaves_link_targets_alone(
    connection: &Connection,
    sftp: &Sftp,
    dir: &str,
) {
    // dir/outside/keep.txt is pointed at by dir/doomed/link; deleting
    // dir/doomed must not enter the link.
    let outside = shellpp_ssh::sftp::join(dir, "outside");
    let doomed = shellpp_ssh::sftp::join(dir, "doomed");
    let script = format!(
        "mkdir -p '{outside}' '{doomed}/sub' && echo keep > '{outside}/keep.txt' \
         && echo x > '{doomed}/sub/x.txt' && ln -s '{outside}' '{doomed}/link'"
    );
    let output = connection
        .exec(&script, std::time::Duration::from_secs(10), 4096)
        .await
        .expect("exec");
    assert_eq!(output.exit_status, Some(0));
    let listed = sftp.list(&doomed).await.unwrap();
    let link = listed.iter().find(|entry| entry.name == "link").unwrap();
    assert_eq!(
        link.kind,
        EntryKind::Symlink {
            target: Some(outside.clone()),
            to_dir: true
        }
    );
    // A trailing slash would make lstat follow the link: removing
    // "doomed/link/" removes the link only.
    let removed = sftp
        .remove_tree(&format!("{doomed}/link/"))
        .await
        .expect("remove a link with a slash");
    assert_eq!(removed, 1);
    let kept = sftp
        .stat(&shellpp_ssh::sftp::join(&outside, "keep.txt"))
        .await;
    assert_eq!(kept.map(|entry| entry.size), Ok(5));
    let output = connection
        .exec(
            &format!("ln -s '{outside}' '{doomed}/link'"),
            std::time::Duration::from_secs(10),
            4096,
        )
        .await
        .expect("exec");
    assert_eq!(output.exit_status, Some(0));
    // Permissions applied recursively leave the link and its target alone.
    let changed = sftp
        .set_permissions_tree(&doomed, 0o750)
        .await
        .expect("recursive permissions");
    assert_eq!(changed, 3, "doomed, sub, sub/x.txt");
    assert_eq!(
        sftp.stat(&shellpp_ssh::sftp::join(&doomed, "sub/x.txt"))
            .await
            .unwrap()
            .permissions,
        Some(0o750)
    );
    assert_ne!(sftp.stat(&outside).await.unwrap().permissions, Some(0o750));
    // The confirmation counts what the delete removes: x.txt and the link,
    // sub and doomed itself; nothing behind the link.
    assert_eq!(sftp.count_tree(&doomed).await.unwrap(), (2, 2));
    let removed = sftp.remove_tree(&doomed).await.expect("remove tree");
    assert_eq!(removed, 4, "sub/x.txt, sub, link, doomed");
    assert_eq!(sftp.stat(&doomed).await.err(), Some(SftpError::NotFound));
    let kept = sftp
        .stat(&shellpp_ssh::sftp::join(&outside, "keep.txt"))
        .await;
    assert_eq!(kept.map(|entry| entry.size), Ok(5));
}

/// Files of the tree transfer (the plan's 2000 small files).
const TREE_FILES: usize = 2000;

async fn trees_and_remote_to_remote(
    connection: &Connection,
    sftp: &Sftp,
    other: &Sftp,
    dir: &str,
    local: &Path,
) {
    use shellpp_ssh::transfer::{SkipReason, copy_between, download_tree, upload_tree};
    // A local tree of small files in three folders.
    let tree = local.join("tree-up");
    for (n, folder) in ["", "a", "a/b"].iter().cycle().take(TREE_FILES).enumerate() {
        let folder = tree.join(folder);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(format!("f{n:04}.txt")), format!("file {n}")).unwrap();
    }
    let remote = shellpp_ssh::sftp::join(dir, "tree-2000");
    let mut last = 0;
    let up = upload_tree(sftp, &tree, &remote, &mut |done| last = done)
        .await
        .expect("upload tree");
    assert_eq!(up.files, TREE_FILES);
    assert_eq!(up.folders, 3);
    assert_eq!(last, up.bytes);
    // Links in the remote tree (D50): to a folder outside it (left out), to
    // a file outside it (copied as the file's content) and to nothing
    // (left out); the left-out ones are listed by their path in the tree.
    let outside = shellpp_ssh::sftp::join(dir, "outside-tree");
    let script = format!(
        "mkdir -p '{outside}' && echo secret > '{outside}/s.txt' && ln -s '{outside}' '{remote}/a/link' && ln -s '{outside}/s.txt' '{remote}/a/file-link' && ln -s '{outside}/gone' '{remote}/broken' && touch \"{remote}/$(printf 'bad\\377')\""
    );
    let output = connection
        .exec(&script, std::time::Duration::from_secs(10), 4096)
        .await
        .expect("exec");
    assert_eq!(output.exit_status, Some(0));
    let back = local.join("tree-down");
    let down = download_tree(sftp, &remote, &back, &mut |_| {})
        .await
        .expect("download tree");
    assert_eq!(down.files, TREE_FILES + 1);
    assert_eq!(down.links, 1);
    assert_eq!(down.bytes, up.bytes + "secret\n".len() as u64);
    let mut skipped: Vec<(String, SkipReason)> = down
        .skipped
        .iter()
        .map(|entry| (entry.path.clone(), entry.reason))
        .collect();
    skipped.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        skipped,
        [
            ("a/link".to_owned(), SkipReason::LinkToFolder),
            ("bad\u{fffd}".to_owned(), SkipReason::NameNotUtf8),
            ("broken".to_owned(), SkipReason::BrokenLink),
        ]
    );
    assert!(
        !back.join("a").join("link").exists(),
        "the link to a folder is not followed"
    );
    assert_eq!(
        std::fs::read_to_string(back.join("a").join("file-link")).unwrap(),
        "secret\n",
        "the link to a file arrives as its content"
    );
    // SFTP cannot reach that name to delete it either (the known limitation).
    let output = connection
        .exec(
            &format!("rm -f \"{remote}/$(printf 'bad\\377')\""),
            std::time::Duration::from_secs(10),
            4096,
        )
        .await
        .expect("exec");
    assert_eq!(output.exit_status, Some(0));
    // The same rule for an upload of a local tree with links.
    #[cfg(unix)]
    {
        let linked = local.join("tree-links");
        std::fs::create_dir_all(linked.join("sub")).unwrap();
        std::fs::write(local.join("target.txt"), "pointed at").unwrap();
        std::os::unix::fs::symlink(local.join("target.txt"), linked.join("to-file")).unwrap();
        std::os::unix::fs::symlink(&tree, linked.join("sub").join("to-folder")).unwrap();
        std::os::unix::fs::symlink(local.join("gone"), linked.join("broken")).unwrap();
        let remote_links = shellpp_ssh::sftp::join(dir, "tree-links");
        let up = upload_tree(sftp, &linked, &remote_links, &mut |_| {})
            .await
            .expect("upload tree with links");
        assert_eq!((up.files, up.links), (1, 1));
        let mut skipped: Vec<(String, SkipReason)> = up
            .skipped
            .iter()
            .map(|entry| (entry.path.clone(), entry.reason))
            .collect();
        skipped.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            skipped,
            [
                ("broken".to_owned(), SkipReason::BrokenLink),
                ("sub/to-folder".to_owned(), SkipReason::LinkToFolder),
            ]
        );
        let arrived = sftp
            .stat(&shellpp_ssh::sftp::join(&remote_links, "to-file"))
            .await
            .expect("uploaded");
        assert_eq!(arrived.kind, shellpp_ssh::EntryKind::File);
    }
    assert_eq!(
        std::fs::read_to_string(back.join("a").join("b").join("f0002.txt")).unwrap(),
        "file 2"
    );
    // Remote to remote: from one session to another.
    let source = shellpp_ssh::sftp::join(&remote, "f0000.txt");
    let target = shellpp_ssh::sftp::join(dir, "copied.txt");
    let size = copy_between(sftp, &source, other, &target, &mut |_| {})
        .await
        .expect("remote to remote");
    assert_eq!(size, "file 0".len() as u64);
    let check = local.join("copied.txt");
    sftp.download(&target, &check, false, &mut |_| {})
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(&check).unwrap(), "file 0");
}

/// Open / Open with (D49): a copy in a folder of its own, saved changes
/// uploaded back, a file changed on the server asked about first, the copy
/// removed when done or at exit.
async fn open_with_round_trip(sftp: &Sftp, dir: &str, local: &Path) {
    use shellpp_ssh::open_with::{OpenedCopy, UploadOutcome, discard_all};
    let remote = shellpp_ssh::sftp::join(dir, "edit me.txt");
    let source = local.join("edit-source.txt");
    std::fs::write(&source, "first").unwrap();
    sftp.upload(&source, &remote, false, &mut |_| {})
        .await
        .unwrap();
    let root = local.join("open-copies");
    let mut copy = OpenedCopy::open(sftp, &remote, &root).await.expect("open");
    assert_eq!(std::fs::read_to_string(copy.local_path()).unwrap(), "first");
    assert_eq!(
        copy.local_path().file_name().unwrap().to_string_lossy(),
        "edit me.txt"
    );
    assert!(!copy.changed_locally());
    assert_eq!(
        copy.upload_back(sftp, false).await,
        Ok(UploadOutcome::Unchanged)
    );
    // The program saves: uploaded back.
    std::fs::write(copy.local_path(), "second version").unwrap();
    assert!(copy.changed_locally());
    assert_eq!(
        copy.upload_back(sftp, false).await,
        Ok(UploadOutcome::Uploaded)
    );
    assert!(!copy.changed_locally());
    let check = local.join("edit-check.txt");
    sftp.download(&remote, &check, false, &mut |_| {})
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(&check).unwrap(), "second version");
    // Someone changes the file on the server; the program saves again.
    std::fs::write(&source, "changed on the server").unwrap();
    sftp.upload(&source, &remote, false, &mut |_| {})
        .await
        .unwrap();
    std::fs::write(copy.local_path(), "third").unwrap();
    assert_eq!(
        copy.upload_back(sftp, false).await,
        Ok(UploadOutcome::RemoteChanged)
    );
    sftp.download(&remote, &check, false, &mut |_| {})
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&check).unwrap(),
        "changed on the server"
    );
    assert_eq!(
        copy.upload_back(sftp, true).await,
        Ok(UploadOutcome::Uploaded)
    );
    sftp.download(&remote, &check, false, &mut |_| {})
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(&check).unwrap(), "third");
    // The same size within the same second still counts as a change on the
    // server (the content of a small file is compared too).
    std::fs::write(&source, "THIRD").unwrap();
    sftp.upload(&source, &remote, false, &mut |_| {})
        .await
        .unwrap();
    std::fs::write(copy.local_path(), "fourth").unwrap();
    assert_eq!(
        copy.upload_back(sftp, false).await,
        Ok(UploadOutcome::RemoteChanged)
    );
    // A changed copy is not thrown away unless forced.
    let copy = *copy.discard(false).expect_err("kept: not uploaded");
    let folder = copy.local_path().parent().unwrap().to_path_buf();
    copy.discard(true).unwrap();
    assert!(!folder.exists());
    // Copies left behind: the unchanged one goes, the edited one is kept.
    let left = OpenedCopy::open(sftp, &remote, &root).await.unwrap();
    let unchanged = left.local_path().to_path_buf();
    drop(left);
    let edited = OpenedCopy::open(sftp, &remote, &root).await.unwrap();
    std::fs::write(edited.local_path(), "an edit that was not uploaded").unwrap();
    let edited_path = edited.local_path().to_path_buf();
    drop(edited);
    let kept = discard_all(&root).unwrap();
    assert!(!unchanged.exists());
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].local, edited_path);
    assert_eq!(kept[0].remote.as_deref(), Some(remote.as_str()));
    std::fs::remove_dir_all(&root).unwrap();
    // A folder is not opened.
    assert_eq!(
        OpenedCopy::open(sftp, dir, &root).await.err(),
        Some(SftpError::NotAFile)
    );
}

/// A download cut by a dropped connection resumes over a new one from its
/// `.part`. Last: the drop ends every connection of the server.
async fn a_dropped_download_resumes(server: &OpenSshServer, dir: &str, local: &Path) {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let remote = shellpp_ssh::sftp::join(dir, "large.bin");
    let target = local.join("dropped.bin");
    let connection = key_connection(server).await;
    let sftp = connection.open_sftp().await.expect("sftp");
    let cut = Arc::new(AtomicBool::new(false));
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let mut tx = Some(tx);
    let flag = Arc::clone(&cut);
    // After 4 MB the download waits (at most 60 s) until the drop landed.
    let mut slow = move |done: u64| {
        if done >= 4 * 1024 * 1024 {
            if let Some(tx) = tx.take() {
                let _ = tx.send(());
            }
            let waited = std::time::Instant::now();
            while !flag.load(Ordering::SeqCst)
                && waited.elapsed() < std::time::Duration::from_secs(60)
            {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    };
    // On its own task: the download's task is slowed down by the callback.
    let drop_now = server.connection_dropper();
    let dropper = tokio::spawn(async move {
        let _ = rx.await;
        tokio::task::spawn_blocking(drop_now)
            .await
            .expect("control.sh ran");
        cut.store(true, Ordering::SeqCst);
    });
    let result = sftp.download(&remote, &target, false, &mut slow).await;
    // The sender goes with the callback, so the dropper never waits forever.
    drop(slow);
    dropper.await.expect("dropper");
    assert!(result.is_err(), "the drop ends the download");
    let part = shellpp_ssh::sftp::local_part(&target);
    let kept = std::fs::metadata(&part).expect("the .part stays").len();
    assert!(kept >= 4 * 1024 * 1024, "{kept}");
    // A new connection goes on from there.
    let connection = key_connection(server).await;
    let sftp = connection.open_sftp().await.expect("sftp again");
    let mut started = None;
    sftp.download(&remote, &target, true, &mut |done| {
        started.get_or_insert(done);
    })
    .await
    .expect("resumed after the drop");
    assert_eq!(started, Some(kept));
    assert_eq!(sha256_file(&target), sha256_file(&local.join("large.bin")));
    sftp.remove_tree(dir).await.expect("clean up");
    connection.disconnect().await;
}

#[allow(clippy::print_stderr)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sftp_against_openssh() {
    if provider() != Provider::OpenSsh {
        eprintln!("skipped: SFTP runs against OpenSSH (SHELLPP_TEST_SSHD=openssh)");
        return;
    }
    let Some(server) = OpenSshServer::start() else {
        return;
    };
    let connection = key_connection(&server).await;
    // A shell is open on the same connection, as in a terminal tab.
    let _shell = connection.open_shell(shell_request()).await.expect("shell");
    let sftp = connection.open_sftp().await.expect("sftp");
    let local = tempfile::tempdir().unwrap();
    let dir = scratch(&sftp).await;
    transfers_keep_every_byte(&sftp, &dir, local.path()).await;
    interrupted_transfers_resume(&sftp, &dir, local.path()).await;
    names_and_many_small_files(&sftp, &dir, local.path()).await;
    permissions_rename_and_errors(&sftp, &dir).await;
    a_recursive_delete_leaves_link_targets_alone(&connection, &sftp, &dir).await;
    // A second connection, as a second remote panel.
    let second = key_connection(&server).await;
    let other = second.open_sftp().await.expect("second sftp");
    trees_and_remote_to_remote(&connection, &sftp, &other, &dir, local.path()).await;
    open_with_round_trip(&sftp, &dir, local.path()).await;
    other.close().await;
    second.disconnect().await;
    sftp.close().await;
    connection.disconnect().await;
    a_dropped_download_resumes(&server, &dir, local.path()).await;
    drop(server);
}

/// The server's SFTP process exits while the connection stays (the
/// subsystem ends alone): the session reports itself closed, so the
/// session layer ends the SFTP tab with Reconnect instead of showing it
/// connected while every request fails at once; requests fail with
/// `ConnectionLost`, and the connection still opens a new session.
#[allow(clippy::print_stderr)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_whose_sftp_server_exits_is_closed() {
    if provider() != Provider::OpenSsh {
        eprintln!("skipped: SFTP runs against OpenSSH (SHELLPP_TEST_SSHD=openssh)");
        return;
    }
    let Some(server) = OpenSshServer::start() else {
        return;
    };
    let connection = key_connection(&server).await;
    let sftp = connection.open_sftp().await.expect("sftp");
    sftp.canonicalize(".").await.expect("a first request");
    assert!(!sftp.is_closed());
    // The exec channel's shell and the SFTP process are children of the
    // same connection process on the server: only this session's SFTP
    // process ends (`[i]` keeps pkill from matching its own command line).
    let killed = connection
        .exec(
            "pkill -P \"$PPID\" -f '[i]nternal-sftp|[s]ftp-server'",
            std::time::Duration::from_secs(10),
            4096,
        )
        .await
        .expect("pkill runs");
    assert_eq!(killed.exit_status, Some(0), "an SFTP process was there");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !sftp.is_closed() {
        assert!(
            std::time::Instant::now() < deadline,
            "the session still looks open after its SFTP process ended"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(!connection.is_closed(), "the connection stays");
    assert_eq!(sftp.canonicalize(".").await, Err(SftpError::ConnectionLost));
    let again = connection.open_sftp().await.expect("a new session");
    again.canonicalize(".").await.expect("works");
    again.close().await;
    connection.disconnect().await;
    drop(server);
}

/// The speed measurement of stage 4 (`SHELLPP_SFTP_BENCH=1`, against
/// OpenSSH): 200 MB up and down, printed as MB/s, to compare with OpenSSH's
/// `sftp` in the same conditions. Does nothing otherwise.
#[allow(clippy::print_stderr)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sftp_speed() {
    if std::env::var("SHELLPP_SFTP_BENCH").as_deref() != Ok("1") || provider() != Provider::OpenSsh
    {
        return;
    }
    let Some(server) = OpenSshServer::start() else {
        return;
    };
    const SIZE: usize = 200 * 1024 * 1024;
    let connection = key_connection(&server).await;
    let sftp = connection.open_sftp().await.expect("sftp");
    let local = tempfile::tempdir().unwrap();
    let dir = scratch(&sftp).await;
    let source = local.path().join("bench.bin");
    std::fs::write(&source, random_bytes(SIZE)).unwrap();
    let remote = shellpp_ssh::sftp::join(&dir, "bench.bin");
    let mb = SIZE as f64 / (1024.0 * 1024.0);
    let started = std::time::Instant::now();
    sftp.upload(&source, &remote, false, &mut |_| {})
        .await
        .unwrap();
    let up = mb / started.elapsed().as_secs_f64();
    let started = std::time::Instant::now();
    sftp.download(&remote, &local.path().join("back.bin"), false, &mut |_| {})
        .await
        .unwrap();
    let down = mb / started.elapsed().as_secs_f64();
    eprintln!("bench: sftp upload {up:.1} MB/s, download {down:.1} MB/s ({mb:.0} MB)");
    sftp.remove_tree(&dir).await.unwrap();
    connection.disconnect().await;
}
