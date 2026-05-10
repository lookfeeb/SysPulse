use crate::paths;
use std::time::{Duration, SystemTime};
use tracing_appender::rolling;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub const LOG_RETAIN_DAYS: u64 = 14;

pub fn init() {
    let _ = paths::ensure_dirs();
    cleanup_old_logs(LOG_RETAIN_DAYS);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,wry=info,tao=info"));

    let file_appender = rolling::daily(paths::logs_dir(), "app.log");

    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_target(false)
        .with_writer(file_appender);

    if cfg!(debug_assertions) {
        let stderr_layer = fmt::layer().with_target(false).with_writer(std::io::stderr);
        tracing_subscriber::registry()
            .with(filter)
            .with(file_layer)
            .with(stderr_layer)
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(file_layer)
            .init();
    }
}

pub fn cleanup_old_logs(retain_days: u64) {
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(retain_days.saturating_mul(24 * 60 * 60)));
    let Some(cutoff) = cutoff else {
        return;
    };

    let Ok(entries) = std::fs::read_dir(paths::logs_dir()) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("app.log") {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if modified < cutoff {
            let _ = std::fs::remove_file(path);
        }
    }
}
