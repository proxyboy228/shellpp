//! Tests of the capture rules and the secret detector (docs/security.md,
//! sections 10.1 and 10.2). Values that look like secrets are fictional or
//! built at run time from fixed fragments; none is a real credential.

use super::*;

fn candidate(command: &str) -> HistoryCandidate {
    HistoryCandidate {
        command: command.to_owned(),
        prompt_text: "user@host:~$".to_owned(),
        typed_printable: true,
        alt_screen: false,
    }
}

/// `n` characters cycling through `alphabet`.
fn cycle(alphabet: &str, n: usize) -> String {
    alphabet.chars().cycle().take(n).collect()
}

// ----- secret detector: one block per rule, positives and negatives --------

#[test]
fn s1_dash_p_value_token() {
    for command in [
        "mysql -uroot -phunter2",
        "mysql -u root -pletmein db",
        "mysqldump -pS3cr3t! --all-databases",
        "mkdir -pv /tmp/a/b",
        "ssh -p2222 deploy@203.0.113.5",
        "mysql -u root '-pquoted'",
        "mysql -u root \"-pdouble\"",
    ] {
        assert_eq!(secret_rule(command), Some(SecretRule::S1), "{command}");
    }
    for command in [
        "mkdir -p /tmp/a",
        "cp -p a b",
        "ssh -p 2222 deploy@203.0.113.5",
        "ls -P",
        "tar -Pczf a.tgz dir",
        "grep -e -p file",
        "echo x-pvalue",
    ] {
        assert_ne!(secret_rule(command), Some(SecretRule::S1), "{command}");
        assert!(!looks_like_secret(command), "{command}");
    }
}

#[test]
fn s2_to_s9_case_insensitive_rules() {
    let cases: &[(&str, SecretRule)] = &[
        ("tool --password=x", SecretRule::S2),
        ("tool --PASSWORD hunter", SecretRule::S2),
        ("tool --password-file /run/pw", SecretRule::S2),
        ("PGPASSWORD=abc psql", SecretRule::S3),
        ("export DB_PASSWORD=abc", SecretRule::S3),
        ("password=abc ./run", SecretRule::S3),
        ("Mysql_Password=abc", SecretRule::S3),
        ("GITHUB_TOKEN=abc gh pr list", SecretRule::S4),
        ("export token=abc", SecretRule::S4),
        (
            "curl 'https://example.com/?access_token=abc'",
            SecretRule::S4,
        ),
        ("export AWS_SECRET_ACCESS_KEY=abc", SecretRule::S5),
        ("kubectl get Secrets", SecretRule::S5),
        ("vault kv get secret/app", SecretRule::S5),
        ("curl --api-key abc https://example.com", SecretRule::S6),
        ("export API_KEY=abc", SecretRule::S6),
        ("export apikey=abc", SecretRule::S6),
        (
            "git clone https://bob:pw@example.com/repo.git",
            SecretRule::S7,
        ),
        ("psql postgres://app:pw@db.example.com/app", SecretRule::S7),
        (
            "curl -H 'Authorization: x' https://example.com",
            SecretRule::S8,
        ),
        (
            "curl -H \"AUTHORIZATION: x\" https://example.com",
            SecretRule::S8,
        ),
        ("curl -H \"Bearer abc\" https://example.com", SecretRule::S9),
        ("echo bearer", SecretRule::S9),
    ];
    for (command, rule) in cases {
        assert_eq!(secret_rule(command), Some(*rule), "{command}");
    }
    for command in [
        "passwd",
        "sudo passwd alice",
        "echo password",
        "grep -r token src/",
        "cat tokens.txt",
        "git log --oneline",
        "ssh deploy@203.0.113.5",
        "scp file deploy@203.0.113.5:/tmp/",
        "rsync -av src/ deploy@db.example.com:/srv/",
        "curl https://example.com/api/keys",
        "echo bearers",
        "docker run -p 8080:80 nginx",
        "ping 192.0.2.1",
    ] {
        assert!(
            !looks_like_secret(command),
            "{command}: {:?}",
            secret_rule(command)
        );
    }
}

#[test]
fn s7_needs_user_and_password_before_at() {
    assert_eq!(
        secret_rule("ftp://anon:x@ftp.example.com"),
        Some(SecretRule::S7)
    );
    // `user@host:path` (scp, git over ssh) is not `user:pass@`.
    assert!(!looks_like_secret(
        "git clone git@example.com:team/repo.git"
    ));
    assert!(!looks_like_secret("scp a.txt user@example.com:dir/"));
    // A colon in a path before an `@` elsewhere does not match across `/`.
    assert!(!looks_like_secret("ls /a:b/c@d"));
}

#[test]
fn s10_known_token_prefixes() {
    let alnum = "aB3dE5fG7hJ9kL1mN2pQ4rS6tU8vW0xY2zA4bC6";
    let samples = [
        format!("gh auth login --with-token ghp_{}", &alnum[..36]),
        format!("echo gho_{}", &alnum[..36]),
        format!("x github_pat_{}", cycle("ab12_", 30)),
        format!("x glpat-{}", cycle("ab12-", 24)),
        format!("x xoxb-{}", cycle("12ab-", 14)),
        "aws configure set aws_access_key_id AKIAABCDEFGHIJKLMNOP".to_owned(),
        "x ASIAABCDEFGHIJKLMNOP".to_owned(),
        format!("x AIza{}", cycle("ab12_-", 35)),
        format!("x sk_live_{}", cycle("ab12", 24)),
        format!("x rk_live_{}", cycle("ab12", 16)),
        format!("npm config set npm_{}", &alnum[..36]),
        format!("x pypi-{}", cycle("ab12_-", 60)),
        "echo '-----BEGIN OPENSSH PRIVATE KEY-----' > k".to_owned(),
        "echo -----BEGIN RSA PRIVATE KEY-----".to_owned(),
        "echo eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0.sig".to_owned(),
        format!("x shellpp_rt_{}", cycle("ab12_", 24)),
        "x RK1-AB12C-xyz".to_owned(),
    ];
    for command in &samples {
        assert!(looks_like_secret(command), "{command}");
    }
    // Rule S10 itself, for samples that no earlier rule takes.
    assert_eq!(secret_rule("x AKIAABCDEFGHIJKLMNOP"), Some(SecretRule::S10));
    assert_eq!(secret_rule("x RK1-AB12C-xyz"), Some(SecretRule::S10));
    // Case-sensitive and length-bound.
    for command in [
        "echo akiaabcdefghijklmnop",
        "echo AKIA1234",
        "echo ghp_short",
        "echo xoxb-",
        "echo -----BEGIN PUBLIC KEY-----",
        "echo rk1-ab12c-",
        "echo sk_test_abcdefghijklmnop",
    ] {
        assert!(
            !looks_like_secret(command),
            "{command}: {:?}",
            secret_rule(command)
        );
    }
}

#[test]
fn s11_high_entropy_strings() {
    // The example of the table.
    assert_eq!(
        secret_rule("export K=q7Xv2LmR9tKp4WzN8sBe"),
        Some(SecretRule::S11)
    );
    assert_eq!(
        secret_rule("curl -d q7Xv2LmR9tKp4WzN8sBe https://example.com"),
        Some(SecretRule::S11)
    );
    // Hex strings of 32+ characters with digits and letters: commit ids and
    // digests match by design (fail closed).
    assert_eq!(
        secret_rule("git checkout 3f786850e387550fdab836ed7e6dc881de23001b"),
        Some(SecretRule::S11)
    );
    assert_eq!(
        secret_rule(
            "docker pull alpine@sha256:4bcff63911fcb4448bd4fdacec207030997caf25e9bea4045fa6c8c44de311d1"
        ),
        Some(SecretRule::S11)
    );
    // Base64 with `+` and `/` is a candidate.
    assert_eq!(
        secret_rule("echo dGhpcyBpcyBhIHRlc3Q+/c2VjcmV0IGJsb2I0Mg"),
        Some(SecretRule::S11)
    );
    // Long strings with high entropy without all three classes.
    assert_eq!(
        secret_rule("echo abcdefghijklmnopqrstuvwxyz0123456789"),
        Some(SecretRule::S11)
    );
    // Split on quotes and `= : , ; @`.
    assert_eq!(
        secret_rule("x=\"q7Xv2LmR9tKp4WzN8sBe\""),
        Some(SecretRule::S11)
    );
    assert_eq!(
        secret_rule("x a,q7Xv2LmR9tKp4WzN8sBe;b"),
        Some(SecretRule::S11)
    );

    for command in [
        // Paths are not candidates (no `+`).
        "cd /var/lib/postgresql/16/main/base/Q7xV2lMr9TkP",
        "cat ~/projects/MyProject2024/src/main.rs",
        // Short tokens.
        "kubectl get pods -n kube-system",
        "q7Xv2LmR9tKp4WzN8sB",
        // 20+ characters, but low entropy or missing classes.
        "echo aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "echo abababababababababababab",
        "echo AAAAAAAAAA1111111111aaaaaaaaaa",
        "systemctl restart my-long-service-name-here",
        "echo some_snake_case_identifier",
        // A UUID: hex digits and dashes, entropy under 4.2.
        "journalctl --boot 0199f6a1-b2c3-7d4e-8f00-112233445566",
        // Hex of 31 characters.
        "echo 3f786850e387550fdab836ed7e6dc88",
        // Hex of 32 characters without letters.
        "echo 12345678901234567890123456789012",
        // Characters outside the candidate set split nothing but disqualify.
        "echo q7Xv2LmR9tKp4WzN8sBe!",
    ] {
        assert!(
            !looks_like_secret(command),
            "{command}: {:?}",
            secret_rule(command)
        );
    }
}

#[test]
fn s11_threshold_corpus() {
    // Ordinary commands of the test corpus: none may be dropped.
    let benign = [
        "ls -la",
        "cd /etc/nginx/sites-available",
        "sudo systemctl status nginx.service",
        "docker compose up -d --build",
        "docker ps --format '{{.Names}}'",
        "git commit -m 'Fix the connection timeout'",
        "git push origin feature/connection-retry",
        "kubectl -n production rollout restart deployment/api-server",
        "tail -f /var/log/syslog",
        "journalctl -u sshd --since today",
        "apt-get install -y build-essential",
        "pip install --upgrade requests",
        "npm run build -- --mode production",
        "find . -name '*.log' -mtime +7 -delete",
        "grep -rn TODO src/",
        "awk '{print $1}' access.log | sort | uniq -c",
        "terraform plan -out=tfplan",
        "ansible-playbook -i inventory/production site.yml",
        "psql -h db.example.com -U app app_production",
        "ssh-keygen -t ed25519 -C deploy@example.com",
        "curl -sS https://example.com/health",
        "export PATH=$PATH:/usr/local/go/bin",
        "echo $HOME",
        "vim /etc/hosts",
        "htop",
        "df -h",
        "du -sh * | sort -h",
        "chmod 600 ~/.ssh/authorized_keys",
        "tar -czf backup-2026-10-18.tar.gz /srv/www",
        "openssl x509 -in cert.pem -noout -dates",
        "python3 manage.py migrate --settings=config.settings.production",
        "SomeVeryLongCamelCaseName",
    ];
    for command in benign {
        assert!(
            !looks_like_secret(command),
            "{command}: {:?}",
            secret_rule(command)
        );
    }
    // Generated secrets of the corpus: every one is dropped.
    let secrets = [
        "Zx9vQ2mL8rT4kW7nB3sY",
        "G4hJ8kL2mN6pQ0rS3tU5vW7xY9zA1bC",
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "AbCdEfGh12345678IjKlMnOp",
        "vF3/kP9+qL2/xR8+mT5/zW1+",
    ];
    for secret in secrets {
        let command = format!("deploy --credential {secret}");
        assert!(looks_like_secret(&command), "{command}");
    }
}

#[test]
fn entropy_function() {
    assert_eq!(shannon_entropy(""), 0.0);
    assert_eq!(shannon_entropy("aaaa"), 0.0);
    assert!((shannon_entropy("ab") - 1.0).abs() < 1e-9);
    assert!((shannon_entropy("abcd") - 2.0).abs() < 1e-9);
    assert!((shannon_entropy("q7Xv2LmR9tKp4WzN8sBe") - 20f64.log2()).abs() < 1e-9);
}

#[test]
fn low_entropy_snippet_passwords() {
    // Short, dictionary-like passwords carry no entropy signal; they are
    // caught only by the rules that look at their context.
    for command in [
        "mysql -uroot -psummer2024",
        "sshpass -phunter2 ssh deploy@203.0.113.5",
        "PGPASSWORD=changeme psql -h db.example.com",
        "redis-cli --pass letmein --password letmein",
        "curl -u admin:admin@ https://example.com",
        "git clone https://deploy:qwerty@example.com/team/app.git",
        "export SECRET=abc123",
        "MYSQL_ROOT_PASSWORD=root docker compose up",
        "docker run -e POSTGRES_PASSWORD=postgres postgres",
        "htpasswd -b .htpasswd admin password=1",
        "vault login token=abc",
    ] {
        assert!(looks_like_secret(command), "{command}");
    }
    // Known limits (docs/security.md, section 10.1): a low-entropy password
    // given as a separate argument or in another variable name is not
    // recognized by any rule. The detector fails closed only where a rule
    // applies; these document the limit and must not silently change.
    for command in [
        "sshpass -p hunter2 ssh deploy@203.0.113.5",
        "MYSQL_PWD=hunter2 mysql",
        "echo hunter2 | sudo -S apt-get update",
    ] {
        assert!(!looks_like_secret(command), "{command}");
    }
}

// ----- password prompt pattern -----------------------------------------------

#[test]
fn password_prompts_in_every_listed_language() {
    for prompt in [
        "Password:",
        "[sudo] password for alice:",
        "alice@example.com's password:",
        "Enter passphrase for key '/home/a/.ssh/id_ed25519':",
        "New passwd:",
        "Passcode:",
        "Enter PIN:",
        "OTP:",
        "One-time code:",
        "one time password:",
        "Verification code:",
        "Token:",
        "2FA code:",
        "Authenticator code:",
        "Secret:",
        "Passwort:",
        "Kennwort:",
        "Mot de passe :",
        "Contraseña:",
        "CONTRASEÑA:",
        "Senha:",
        "Parola:",
        "Wachtwoord:",
        "Hasło:",
        "Heslo:",
        "Пароль:",
        "Введите пароль:",
        "密码：",
        "パスワード:",
        "비밀번호:",
    ] {
        assert!(prompt_looks_like_password(prompt), "{prompt}");
    }
    for prompt in [
        "alice@host:~$",
        "root@db:/var/lib#",
        "PS C:\\Users\\alice>",
        "mysql>",
        "Do you want to continue? [Y/n]",
        "spinning up",
        "tokens left:",
        "secretary@host:~$",
        "pinned>",
        "",
    ] {
        assert!(!prompt_looks_like_password(prompt), "{prompt}");
    }
}

/// A prompt printed after the boundary (input typed before the prompt
/// appeared) is part of the captured line; it drops the line too.
#[test]
fn a_password_prompt_inside_the_captured_line() {
    for line in [
        "Password:",
        "[sudo] password for alice:",
        "hun[sudo] password for alice:",
        // Typed ahead of `su`'s prompt, which starts with the word: no
        // word boundary between the input and the prompt.
        "hunter2Password:",
        "Ahead-pw-2Password:",
        "Ahead-pw-2Passwort:",
        "Enter passphrase (empty for no passphrase):",
        "Enter same passphrase again:",
        "Enter password:",
        "Current password:",
        "Mot de passe :",
        "密码：",
        "echo \"password: x\"",
    ] {
        assert!(line_holds_password_prompt(line), "{line}");
        let mut typed_ahead = candidate(line);
        typed_ahead.prompt_text = "alice@host:~$ sudo -v".to_owned();
        assert_eq!(
            evaluate(&typed_ahead, false, true),
            Err(HistoryDropReason::PasswordPrompt),
            "{line}"
        );
    }
    // The word must come before a colon: a commit message, a path or a
    // command that only names the tools stays a command.
    for line in [
        "passwd",
        "sudo passwd alice",
        "grep -i password /etc/app.conf",
        "git commit -m \"fix: token refresh\"",
        "ssh-keygen -t ed25519 -f /tmp/k",
        "mysql -u root -p",
        "sudo -S apt update",
        "scp notes.txt deploy@web:/tmp/",
        // The short words keep their word boundaries.
        "ping: unknown host",
        "curl -H \"tokens: 3\" http://localhost/",
        "",
    ] {
        assert!(!line_holds_password_prompt(line), "{line}");
    }
    assert_eq!(
        evaluate(&candidate("sudo passwd alice"), false, true),
        Ok("sudo passwd alice".to_owned())
    );
}

// ----- cleaning ----------------------------------------------------------------

#[test]
fn bracketed_paste_markers_are_stripped() {
    assert_eq!(clean_command("\u{1b}[200~ls -la\u{1b}[201~"), "ls -la");
    assert_eq!(
        clean_command("echo \u{1b}[200~a b\u{1b}[201~ c"),
        "echo a b c"
    );
    // 8-bit CSI form.
    assert_eq!(clean_command("\u{9b}200~ls\u{9b}201~"), "ls");
    // Caret notation echoed by a shell without bracketed paste support.
    assert_eq!(clean_command("^[[200~git status^[[201~"), "git status");
    // A secret pasted inside markers is still detected after cleaning.
    let command = clean_command("\u{1b}[200~mysql -phunter2\u{1b}[201~");
    assert_eq!(command, "mysql -phunter2");
    assert!(looks_like_secret(&command));
}

#[test]
fn control_characters_and_escape_sequences_are_removed() {
    assert_eq!(clean_command("ls\u{7}\u{8}\u{0} -l"), "ls -l");
    assert_eq!(clean_command("a\tb"), "a b");
    assert_eq!(clean_command("echo \u{1b}[31mred\u{1b}[0m"), "echo red");
    assert_eq!(clean_command("echo \u{1b}[?2004hx"), "echo x");
    assert_eq!(clean_command("\u{1b}]0;title\u{7}pwd"), "pwd");
    assert_eq!(clean_command("\u{1b}]0;title\u{1b}\\pwd"), "pwd");
    assert_eq!(clean_command("\u{1b}Pdcs data\u{1b}\\ls"), "ls");
    assert_eq!(clean_command("\u{1b}(Bls"), "ls");
    assert_eq!(clean_command("\u{1b}7ls\u{1b}8"), "ls");
    assert_eq!(clean_command("x\u{85}y\u{8a}z"), "xyz");
    // An 8-bit APC string is removed whole, up to its terminator.
    assert_eq!(clean_command("x\u{9f}hidden\u{9c}y"), "xy");
    assert_eq!(clean_command("ls   \t "), "ls");
    assert_eq!(clean_command("  ls"), "  ls");
    assert_eq!(clean_command("echo 日本語 ✓"), "echo 日本語 ✓");
    assert_eq!(clean_command("ls\r\n"), "ls");
    assert_eq!(clean_command("\u{1b}"), "");
    assert_eq!(clean_command("ls \u{1b}["), "ls");
}

// ----- capture rules, in order -------------------------------------------------

#[test]
fn every_drop_reason_of_evaluate() {
    use HistoryDropReason::*;
    assert_eq!(evaluate(&candidate("ls"), false, false), Err(Off));
    assert_eq!(evaluate(&candidate("ls"), true, true), Err(Injected));
    let mut alt = candidate("vim x");
    alt.alt_screen = true;
    assert_eq!(evaluate(&alt, false, true), Err(AltScreen));
    assert_eq!(evaluate(&candidate(""), false, true), Err(EchoOff));
    assert_eq!(evaluate(&candidate("****"), false, true), Err(EchoOff));
    assert_eq!(evaluate(&candidate("•••"), false, true), Err(EchoOff));
    assert_eq!(evaluate(&candidate("●●"), false, true), Err(EchoOff));
    let mut prompt = candidate("hunter2");
    prompt.prompt_text = "[sudo] password for test:".to_owned();
    assert_eq!(evaluate(&prompt, false, true), Err(PasswordPrompt));
    let mut nothing_typed = candidate("");
    nothing_typed.typed_printable = false;
    assert_eq!(evaluate(&nothing_typed, false, true), Err(Empty));
    assert_eq!(evaluate(&candidate(" ls"), false, true), Err(LeadingSpace));
    assert_eq!(evaluate(&candidate("\tls"), false, true), Err(LeadingSpace));
    assert_eq!(
        evaluate(&candidate(&"x".repeat(MAX_COMMAND_CHARS + 1)), false, true),
        Err(TooLong)
    );
    assert_eq!(
        evaluate(&candidate("mysql -phunter2"), false, true),
        Err(Secret)
    );
    assert_eq!(
        evaluate(&candidate("git status  "), false, true),
        Ok("git status".to_owned())
    );
    assert_eq!(
        evaluate(&candidate(&"x".repeat(MAX_COMMAND_CHARS)), false, true)
            .map(|command| command.len()),
        Ok(MAX_COMMAND_CHARS)
    );
}

#[test]
fn evaluate_checks_in_plan_order() {
    use HistoryDropReason::*;
    // Off wins over everything else.
    let mut all = candidate(" mysql -phunter2");
    all.alt_screen = true;
    all.prompt_text = "Password:".to_owned();
    assert_eq!(evaluate(&all, true, false), Err(Off));
    assert_eq!(evaluate(&all, true, true), Err(Injected));
    assert_eq!(evaluate(&all, false, true), Err(AltScreen));
    all.alt_screen = false;
    assert_eq!(evaluate(&all, false, true), Err(PasswordPrompt));
    all.prompt_text = "$".to_owned();
    assert_eq!(evaluate(&all, false, true), Err(LeadingSpace));
    // Echo off comes before the prompt check.
    let mut masked = candidate("***");
    masked.prompt_text = "Password:".to_owned();
    assert_eq!(evaluate(&masked, false, true), Err(EchoOff));
    // A prompt with escape sequences is cleaned before matching.
    let mut colored = candidate("hunter2");
    colored.prompt_text = "\u{1b}[1mPassword\u{1b}[0m:".to_owned();
    assert_eq!(evaluate(&colored, false, true), Err(PasswordPrompt));
    // The length counts characters, not bytes.
    assert!(evaluate(&candidate(&"ж".repeat(MAX_COMMAND_CHARS)), false, true).is_ok());
}

#[test]
fn imported_commands_follow_the_rules_a_stored_command_can_be_checked_by() {
    use HistoryDropReason::*;
    assert_eq!(imported_command("ls -la").as_deref(), Ok("ls -la"));
    // Control and escape sequences are removed as for a typed command.
    assert_eq!(
        imported_command("echo \u{1b}[1mhi\u{1b}[0m").as_deref(),
        Ok("echo hi")
    );
    let secret = format!("mysql -p{}", cycle("Kq7", 12));
    assert_eq!(imported_command(&secret), Err(Secret));
    assert_eq!(imported_command(" ls"), Err(LeadingSpace));
    assert_eq!(imported_command(""), Err(Empty));
    assert_eq!(imported_command("****"), Err(EchoOff));
    assert_eq!(
        imported_command("[sudo] password for deploy:"),
        Err(PasswordPrompt)
    );
    assert_eq!(
        imported_command(&"a".repeat(MAX_COMMAND_CHARS + 1)),
        Err(TooLong)
    );
}
