//! Making failures observable: where log lines go, and the one door an
//! [`AppError`] passes through on its way to the UI.
//!
//! Two problems live here, because they are the same problem — a masked
//! `Internal error` banner with no way to find out what actually failed:
//!
//! 1. **A bundled app has nowhere to log.** `tracing_subscriber::fmt()` writes
//!    to stderr, and macOS launchd sends a Finder-launched `.app`'s stderr to
//!    `/dev/null`. [`init`] therefore adds a rolling *file* layer beside the
//!    stderr one, so a shipped build is diagnosable without a terminal.
//! 2. **The cause is discarded before it is logged.** `AppError`'s `Display`
//!    masks internal errors by design, so `error.to_string()` straight into a
//!    UI signal throws away the operation, context, and source chain that
//!    infrastructure carefully attached. [`surface_error`] is the funnel that
//!    logs the full `Debug` tree first and hands back only the client-safe
//!    message.

use std::path::{Path, PathBuf};

use domain::AppError;
use tracing::warn;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

/// Directory name for the app's logs, in the OS log location.
const LOG_DIR_NAME: &str = "Zfirot";
/// Directory name under a non-macOS local data dir, matching the config dir
/// `infrastructure` already uses for the board cache and other app data.
const APP_DIR_NAME: &str = "zfirot";
/// Base name of a rolling log file (`zfirot.2026-08-03.log`).
const LOG_FILE_PREFIX: &str = "zfirot";
/// How many daily files to keep, so logs cannot grow without bound.
const MAX_LOG_FILES: usize = 7;

/// Install the global tracing subscriber: stderr (for `cargo run` / `make dev`)
/// plus, when a log directory is available, a daily-rolling file (for bundled
/// builds). `RUST_LOG` controls the level for both; the default is `info`.
///
/// Returns the file being written to, or `None` when file logging could not be
/// set up — a missing or unwritable log directory degrades to stderr-only
/// rather than blocking startup, since losing logs must never cost the user
/// their app.
pub fn init() -> Option<PathBuf> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let dir = log_dir();
    let appender = dir.as_deref().and_then(appender_in);
    // A `None` layer is a no-op layer, which is exactly the stderr-only
    // degradation we want.
    let file_layer = appender.map(|appender| fmt::layer().with_ansi(false).with_writer(appender));
    let logging_to = file_layer.is_some().then(|| dir).flatten();

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(file_layer)
        .init();

    logging_to
}

/// Log an error's full diagnostic tree and return the client-safe message the
/// UI should show.
///
/// Every place that turns an [`AppError`] into UI text goes through here, so a
/// cause can never be silently dropped: the banner still shows only `Display`
/// (a generic "Internal error" for `AppErrorKind::Internal`), while the log
/// keeps the `Debug` tree — operation, context fields, and source chain.
/// `surface` names where in the UI the error landed (for example
/// `"board_refresh"` or `"save_token"`), so a log line can be traced back to
/// the screen that showed it.
pub fn surface_error(surface: &str, error: &AppError) -> String {
    warn!(surface = %surface, error = ?error, "surfacing an error to the UI");
    error.to_string()
}

/// The directory a bundled build writes its rolling log files to.
fn log_dir() -> Option<PathBuf> {
    log_dir_from(dirs::home_dir(), dirs::data_local_dir())
}

/// Choose the log directory from the OS directories available.
///
/// macOS keeps app logs in `~/Library/Logs/<App>`, which is where Console.app
/// looks and where a user can be asked to find them. Elsewhere there is no such
/// convention, so logs sit beside the app's other local data.
fn log_dir_from(home: Option<PathBuf>, data_local: Option<PathBuf>) -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        if let Some(home) = home {
            return Some(home.join("Library").join("Logs").join(LOG_DIR_NAME));
        }
    }
    data_local.map(|base| base.join(APP_DIR_NAME).join("logs"))
}

/// A daily-rolling appender writing into `dir`, or `None` if the directory
/// cannot be created or opened.
///
/// Deliberately a *blocking* appender rather than `tracing_appender::non_blocking`:
/// the non-blocking writer needs its `WorkerGuard` to flush on drop, and the
/// lines that matter most here — the panic logger's, written while the stack is
/// already unwinding out of the windowing FFI — are exactly the ones an
/// unflushed background worker would lose.
fn appender_in(dir: &Path) -> Option<RollingFileAppender> {
    std::fs::create_dir_all(dir).ok()?;
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILE_PREFIX)
        .filename_suffix("log")
        .max_log_files(MAX_LOG_FILES)
        .build(dir)
        .ok()
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn unique_temp_path(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("zfirot-{label}-{unique}"))
    }

    #[test]
    fn log_dir_uses_the_os_log_location() {
        let dir = log_dir_from(Some(PathBuf::from("/home/u")), Some(PathBuf::from("/data")))
            .expect("a log directory should be chosen when both bases exist");

        if cfg!(target_os = "macos") {
            assert_eq!(dir, PathBuf::from("/home/u/Library/Logs/Zfirot"));
        } else {
            assert_eq!(dir, PathBuf::from("/data/zfirot/logs"));
        }
    }

    #[test]
    fn log_dir_falls_back_to_local_data_without_a_home() {
        let dir = log_dir_from(None, Some(PathBuf::from("/data")))
            .expect("local data should be enough to choose a log directory");

        assert_eq!(dir, PathBuf::from("/data/zfirot/logs"));
    }

    #[test]
    fn log_dir_is_none_when_the_os_offers_no_directory() {
        assert_eq!(log_dir_from(None, None), None);
    }

    #[test]
    fn appender_creates_a_missing_log_directory_and_writes_there() {
        let dir = unique_temp_path("logs");

        let mut appender = appender_in(&dir).expect("a fresh temp directory should be usable");
        writeln!(appender, "hello").expect("the appender should accept a line");
        appender.flush().expect("the appender should flush");

        let written: Vec<_> = std::fs::read_dir(&dir)
            .expect("the log directory should have been created")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            written.iter().any(|name| name.starts_with("zfirot")
                && name.ends_with(".log")
                && name.len() > "zfirot.log".len()),
            "expected a dated zfirot log file, found {written:?}",
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn appender_is_none_when_the_log_directory_cannot_be_created() {
        // A regular file where the directory should be: `create_dir_all` fails,
        // and the app must still start with stderr-only logging.
        let path = unique_temp_path("not-a-dir");
        std::fs::write(&path, b"blocking file").expect("temp file should be writable");

        assert!(appender_in(&path).is_none());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn surfacing_an_internal_error_returns_the_masked_message() {
        let error = AppError::internal("connection pool exhausted")
            .with_operation("LoadBoard::run")
            .with_context("repo", "funkode-io/zfirot");

        assert_eq!(surface_error("board_load", &error), "Internal error");
    }

    #[test]
    fn surfacing_a_client_safe_error_returns_its_message() {
        let error = AppError::not_found("Slice does not exist");

        assert_eq!(surface_error("board_load", &error), "Slice does not exist");
    }
}
