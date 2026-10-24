//! `shellpp-server`: `serve`, `migrate` and `admin ...` (docs/architecture.md,
//! section 10.2). Configuration through `SHELLPP_*` environment variables.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use shellpp_server::{Config, Db, admin, migrate};
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "shellpp-server", version, about = "Shell++ sync server")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serves the API (refuses a database schema older than this binary).
    Serve,
    /// Applies the pending schema migrations.
    Migrate,
    /// Administration on the database.
    Admin {
        #[command(subcommand)]
        command: AdminCommand,
    },
}

#[derive(Subcommand)]
enum AdminCommand {
    /// Accounts and registration codes.
    User {
        #[command(subcommand)]
        command: UserCommand,
    },
    /// Devices.
    Device {
        #[command(subcommand)]
        command: DeviceCommand,
    },
    /// Vaults.
    Vault {
        #[command(subcommand)]
        command: VaultCommand,
    },
    /// Marks the end of a restore from a backup: a new server generation,
    /// so every client checks its vaults against the server.
    Restored,
    /// Purges expired tombstones, tokens, tickets, codes and invitations.
    Purge,
}

#[derive(Subcommand)]
enum UserCommand {
    /// Creates a registration code (shown once).
    Invite {
        #[arg(long, default_value_t = 1)]
        uses: u32,
        #[arg(long, default_value_t = 7)]
        days: u32,
        #[arg(long)]
        note: Option<String>,
    },
    /// Lists the accounts.
    List,
    /// Deletes an account (UUID or email).
    Delete { user: String },
    /// Sets or clears an account's storage limits.
    Quota {
        user: String,
        #[arg(long)]
        records: Option<u64>,
        #[arg(long)]
        bytes: Option<u64>,
        /// Clears both limits (the server defaults apply again).
        #[arg(long)]
        clear: bool,
    },
    /// Resets TOTP of an account that lost its authenticator.
    ResetTotp { user: String },
    /// Disables an account (signs every device out).
    Disable { user: String },
    /// Enables a disabled account.
    Enable { user: String },
}

#[derive(Subcommand)]
enum DeviceCommand {
    /// Revokes a device's tokens.
    Revoke { device_id: Uuid },
}

#[derive(Subcommand)]
enum VaultCommand {
    /// Deletes a vault and its records.
    Delete { vault_id: Uuid },
}

fn init_logging() {
    let filter = std::env::var("SHELLPP_LOG").unwrap_or_else(|_| "info".to_owned());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .with_writer(std::io::stderr)
        .try_init();
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// Prints a line for the operator (the CLI's output, not a log).
fn say(line: &str) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stdout(), "{line}");
}

fn main() -> ExitCode {
    init_logging();
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            tracing::error!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Serve => {
            let config = Config::from_env(env).map_err(|e| e.to_string())?;
            for warning in &config.warnings {
                tracing::warn!("{warning}");
            }
            let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
            runtime
                .block_on(shellpp_server::serve(config, async {
                    let _ = tokio::signal::ctrl_c().await;
                }))
                .map_err(|e| e.to_string())
        }
        Command::Migrate => {
            let url = env("SHELLPP_DATABASE_URL").ok_or("SHELLPP_DATABASE_URL is required")?;
            let db = Db::open(&url).map_err(|e| e.to_string())?;
            let applied = migrate::migrate(&db).map_err(|e| e.to_string())?;
            say(&format!(
                "schema at version {} ({} step(s) applied)",
                migrate::CURRENT,
                applied.len()
            ));
            Ok(())
        }
        Command::Admin { command } => {
            let url = env("SHELLPP_DATABASE_URL").ok_or("SHELLPP_DATABASE_URL is required")?;
            let db = Db::open(&url).map_err(|e| e.to_string())?;
            let fail = |e: shellpp_server::error::ApiError| e.code.to_owned();
            match command {
                AdminCommand::User { command } => match command {
                    UserCommand::Invite { uses, days, note } => {
                        let code = admin::invite(&db, uses, days, note).map_err(fail)?;
                        say(&code);
                    }
                    UserCommand::List => {
                        for (id, email, created) in admin::user_list(&db).map_err(fail)? {
                            say(&format!("{id}  {created}  {email}"));
                        }
                    }
                    UserCommand::Delete { user } => {
                        let id = admin::user_delete(&db, &user).map_err(fail)?;
                        say(&format!("deleted {id}"));
                    }
                    UserCommand::Quota {
                        user,
                        records,
                        bytes,
                        clear,
                    } => {
                        let (records, bytes) = if clear {
                            (None, None)
                        } else {
                            (records, bytes)
                        };
                        let id = admin::user_quota(&db, &user, records, bytes).map_err(fail)?;
                        say(&format!(
                            "quota of {id}: records {records:?}, bytes {bytes:?}"
                        ));
                    }
                    UserCommand::ResetTotp { user } => {
                        let id = admin::user_reset_totp(&db, &user).map_err(fail)?;
                        say(&format!("TOTP reset for {id}"));
                    }
                    UserCommand::Disable { user } => {
                        let id = admin::user_disable(&db, &user, true).map_err(fail)?;
                        say(&format!("disabled {id}"));
                    }
                    UserCommand::Enable { user } => {
                        let id = admin::user_disable(&db, &user, false).map_err(fail)?;
                        say(&format!("enabled {id}"));
                    }
                },
                AdminCommand::Device {
                    command: DeviceCommand::Revoke { device_id },
                } => {
                    admin::device_revoke(&db, device_id).map_err(fail)?;
                    say(&format!("revoked {device_id}"));
                }
                AdminCommand::Vault {
                    command: VaultCommand::Delete { vault_id },
                } => {
                    admin::vault_delete(&db, vault_id).map_err(fail)?;
                    say(&format!("deleted {vault_id}"));
                }
                AdminCommand::Restored => {
                    admin::restored(&db).map_err(fail)?;
                    say("server generation replaced");
                }
                AdminCommand::Purge => {
                    let days = env("SHELLPP_TOMBSTONE_RETENTION_DAYS")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(90);
                    let count = admin::purge(&db, days).map_err(fail)?;
                    say(&format!("purged {count} tombstone(s)"));
                }
            }
            Ok(())
        }
    }
}
