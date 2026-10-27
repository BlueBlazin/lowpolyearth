//! `log` and panic output routed to the browser console.

use std::sync::{OnceLock, RwLock};

/// `RUST_LOG`-style filter: a default level plus `target=level` overrides,
/// most specific prefix first.
struct Filter {
    default: log::LevelFilter,
    targets: Vec<(String, log::LevelFilter)>,
}

impl Filter {
    fn parse(spec: &str) -> Self {
        let mut filter = Filter {
            default: log::LevelFilter::Info,
            targets: Vec::new(),
        };
        for directive in spec.split(',').map(str::trim).filter(|d| !d.is_empty()) {
            match directive.split_once('=') {
                Some((target, level)) => {
                    if let Ok(level) = level.parse() {
                        filter.targets.push((target.to_owned(), level));
                    }
                }
                None => {
                    if let Ok(level) = directive.parse() {
                        filter.default = level;
                    }
                }
            }
        }
        filter
            .targets
            .sort_by_key(|(target, _)| std::cmp::Reverse(target.len()));
        filter
    }

    fn level(&self, target: &str) -> log::LevelFilter {
        self.targets
            .iter()
            .find(|(prefix, _)| target.starts_with(prefix.as_str()))
            .map_or(self.default, |(_, level)| *level)
    }

    fn max(&self) -> log::LevelFilter {
        self.targets
            .iter()
            .map(|(_, level)| *level)
            .fold(self.default, std::cmp::max)
    }
}

struct Console {
    filter: RwLock<Filter>,
}

impl log::Log for Console {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= self.filter.read().unwrap().level(metadata.target())
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "[{}] {} {}: {}",
            thread_label(),
            record.level(),
            record.target(),
            record.args()
        );
        let line = line.into();
        match record.level() {
            log::Level::Error => web_sys::console::error_1(&line),
            log::Level::Warn => web_sys::console::warn_1(&line),
            log::Level::Info => web_sys::console::info_1(&line),
            // Chrome hides `console.debug` behind its Verbose level; a record
            // that passed the filter was asked for.
            log::Level::Debug | log::Level::Trace => web_sys::console::log_1(&line),
        }
    }

    fn flush(&self) {}
}

fn thread_label() -> String {
    super::thread::current_name().unwrap_or_else(|| "worker".into())
}

fn console() -> &'static Console {
    static LOGGER: OnceLock<Console> = OnceLock::new();
    LOGGER.get_or_init(|| Console {
        filter: RwLock::new(Filter::parse("info")),
    })
}

/// Installs the process-wide logger (at `info`) and panic hook. Both live in
/// shared memory, so one call from the root worker covers every thread.
pub fn install_console_logger() {
    let logger = console();
    if log::set_logger(logger).is_ok() {
        log::set_max_level(logger.filter.read().unwrap().max());
    }
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&format!("[{}] {info}", thread_label()).into());
    }));
}

/// Replaces the log filter with a `RUST_LOG`-style specification such as
/// `info,vearth_app::diagnostics=debug`.
pub fn set_log_filter(spec: &str) {
    let filter = Filter::parse(spec);
    log::set_max_level(filter.max());
    *console().filter.write().unwrap() = filter;
}
