//! Rolling log files with a level that can change at run time.

use std::path::Path;
use std::sync::OnceLock;

use bento_core::settings::LogLevel;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::Registry;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::reload;
use tracing_subscriber::util::SubscriberInitExt;

static HANDLE: OnceLock<reload::Handle<LevelFilter, Registry>> = OnceLock::new();

fn filter(l: LogLevel) -> LevelFilter {
    match l {
        LogLevel::Error => LevelFilter::ERROR,
        LogLevel::Warn => LevelFilter::WARN,
        LogLevel::Info => LevelFilter::INFO,
        LogLevel::Debug => LevelFilter::DEBUG,
        LogLevel::Trace => LevelFilter::TRACE,
    }
}

/// Starts logging to `dir\bento-desk.<date>.log`, 7 files kept.
/// The returned guard flushes the writer thread when dropped.
pub fn init(dir: &Path, level: LogLevel) -> Option<WorkerGuard> {
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("bento-desk")
        .filename_suffix("log")
        .max_log_files(7)
        .build(dir)
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let (level_layer, handle) = reload::Layer::new(filter(level));
    let fmt = tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .with_target(false);
    if tracing_subscriber::registry()
        .with(level_layer)
        .with(fmt)
        .try_init()
        .is_err()
    {
        return None;
    }
    let _ = HANDLE.set(handle);
    std::panic::set_hook(Box::new(|info| {
        let bt = std::backtrace::Backtrace::force_capture();
        tracing::error!("panic: {info}\n{bt}");
    }));
    Some(guard)
}

/// Changes the log level without restarting.
pub fn set_level(level: LogLevel) {
    if let Some(h) = HANDLE.get() {
        let _ = h.modify(|f| *f = filter(level));
    }
}
