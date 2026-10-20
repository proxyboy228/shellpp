//! The SFTP engine (night plan, stage 4; D45) against OpenSSH in Docker
//! (`SHELLPP_TEST_SSHD=openssh`; on Windows with
//! `SHELLPP_OPENSSH_SERVER_JSON`; skipped with a printed reason
//! otherwise): SFTP on the connection a shell already uses, upload and
//! download with a SHA-256 check, resume from a `.part` file, Cyrillic,
//! spaces and emoji in names, a tree of small files, permissions, rename,
//! a recursive delete that leaves the target of a symbolic link alone,
//! folder transfers of 2000 files that leave links out, and a copy from
//! one session to another (remote ↔ remote).
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
    use shellpp_ssh::transfer::{copy_between, download_tree, upload_tree};
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
    // A link in the remote tree to a folder outside it is left out.
    let outside = shellpp_ssh::sftp::join(dir, "outside-tree");
    let script = format!(
        "mkdir -p '{outside}' && echo secret > '{outside}/s.txt' && ln -s '{outside}' '{remote}/a/link'"
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
    assert_eq!(down.files, TREE_FILES);
    assert_eq!(down.links, 1);
    assert_eq!(down.bytes, up.bytes);
    assert!(
        !back.join("a").join("link").exists(),
        "the link is not followed"
    );
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
    other.close().await;
    second.disconnect().await;
    sftp.remove_tree(&dir).await.expect("clean up");
    sftp.close().await;
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
