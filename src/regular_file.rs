//! Nonblocking, descriptor-validated regular-file reads, including symlink targets.
use std::{fs::File, io, path::Path};

/// Open without waiting for a FIFO peer, then validate the actual opened target.
/// A path metadata check before open would race replacement. Symlinks to regular
/// files are intentional (in particular, native cache snapshot entries).
pub(crate) fn open(path: &Path) -> io::Result<File> {
    use rustix::fs::{Mode, OFlags};
    let file = File::from(rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )?);
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("Expected a regular file"));
    }
    Ok(file)
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::{path::Path, process::Command, time::Duration};

    pub(crate) fn child_root() -> Option<std::path::PathBuf> {
        std::env::var_os("HERD_REGULAR_FILE_TEST_ROOT").map(Into::into)
    }

    // A regression must not leave a stuck FIFO open or blocking worker in the
    // test runner. Run the real consumer in a bounded, reaped child instead.
    pub(crate) fn run(test: &str, root: &Path) {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture"])
            .env("HERD_REGULAR_FILE_TEST_ROOT", root)
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "Regular-file consumer failed: {test}");
                return;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("Regular-file consumer timed out: {test}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
