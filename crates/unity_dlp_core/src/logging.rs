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
    let _ = log::set_logger(&LOGGER).map(|()| log::set_max_level(LevelFilter::Debug));
}
