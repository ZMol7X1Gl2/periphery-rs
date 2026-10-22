//! An opt-in record of the reports exchanged with open devices, for bug
//! reports. Off until [`start`]; [`stop`] returns what was exchanged since.
//! Only interfaces an app opens are recorded, which for Periphery are
//! configuration interfaces, never typing.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// Lines kept per recording; later exchanges are only counted.
const MAX_LINES: usize = 400;
/// Bytes shown per report.
const MAX_BYTES: usize = 64;

static ON: AtomicBool = AtomicBool::new(false);
static LOG: Mutex<Log> = Mutex::new(Log { lines: Vec::new(), dropped: 0 });

struct Log {
    lines: Vec<String>,
    dropped: usize,
}

/// Which way a report went.
#[derive(Clone, Copy)]
pub(crate) enum Dir {
    Write,
    Read,
    FeatureSet,
    FeatureGet,
}

/// Clears the record and starts recording.
pub fn start() {
    if let Ok(mut log) = LOG.lock() {
        log.lines.clear();
        log.dropped = 0;
    }
    ON.store(true, Ordering::Relaxed);
}

/// Stops recording and returns the lines recorded since [`start`].
pub fn stop() -> Vec<String> {
    ON.store(false, Ordering::Relaxed);
    let Ok(mut log) = LOG.lock() else { return Vec::new() };
    let mut lines = std::mem::take(&mut log.lines);
    if log.dropped > 0 {
        lines.push(format!("… {} more reports not shown", log.dropped));
    }
    lines
}

/// Notes one report on device `id` (vid:pid) while recording.
pub(crate) fn note(id: &str, dir: Dir, bytes: &[u8]) {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    let Ok(mut log) = LOG.lock() else { return };
    if log.lines.len() >= MAX_LINES {
        log.dropped += 1;
        return;
    }
    let arrow = match dir {
        Dir::Write => "->",
        Dir::Read => "<-",
        Dir::FeatureSet => "=> feature",
        Dir::FeatureGet => "<= feature",
    };
    let shown: Vec<String> = bytes.iter().take(MAX_BYTES).map(|b| format!("{b:02x}")).collect();
    let more = if bytes.len() > MAX_BYTES { format!(" …(+{})", bytes.len() - MAX_BYTES) } else { String::new() };
    log.lines.push(format!("{id} {arrow} {}{more}", shown.join(" ")));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_only_between_start_and_stop() {
        note("1234:5678", Dir::Write, &[1]);
        start();
        note("1234:5678", Dir::Write, &[0x00, 0x01, 0xff]);
        note("1234:5678", Dir::Read, &[0x01; 70]);
        let lines = stop();
        note("1234:5678", Dir::Write, &[2]);
        assert_eq!(lines[0], "1234:5678 -> 00 01 ff");
        assert!(lines[1].starts_with("1234:5678 <- 01 01") && lines[1].ends_with("…(+6)"));
        assert_eq!(lines.len(), 2);
        assert!(stop().is_empty());
    }
}
