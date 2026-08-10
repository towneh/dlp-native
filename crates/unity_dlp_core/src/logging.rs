use log::{Level, LevelFilter, Metadata, Record};

struct SimpleLogger;

impl log::Log for SimpleLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Debug
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            // Phase-0: write to stderr. Phase-6 will route through a Unity
            // Debug.Log callback registered via a C function pointer.
            eprintln!("[unity_dlp][{}] {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

static LOGGER: SimpleLogger = SimpleLogger;

pub fn init() {
    match log::set_logger(&LOGGER) {
        Ok(()) => log::set_max_level(LevelFilter::Debug),
        // A logger is already installed: either ours from an earlier init, in
        // which case the level below was already applied, or the host's. The
        // global max level is deliberately left alone here rather than
        // overriding a host that configured its own.
        Err(_) => {}
    }
}
