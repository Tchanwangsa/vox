//! A one-shot speech helper that can load its model before it receives audio.

use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use super::{HEARING, LOADING_SPEECH};

enum Startup {
    Loading,
    Waiting(Arc<AtomicU8>),
    Ready,
}

pub(super) struct Process {
    child: Option<Child>,
    log: Option<JoinHandle<Vec<u8>>>,
    stdout: Option<JoinHandle<io::Result<Vec<u8>>>>,
    startup: Arc<Mutex<Startup>>,
}

impl Process {
    pub(super) fn start(command: &mut Command, ready: fn(&[u8]) -> bool) -> io::Result<Self> {
        let mut child = command
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stderr = child.stderr.take().expect("piped stderr");
        let stdout = child.stdout.take().map(|mut pipe| {
            thread::spawn(move || {
                let mut output = Vec::new();
                pipe.read_to_end(&mut output)?;
                Ok(output)
            })
        });
        let startup = Arc::new(Mutex::new(Startup::Loading));
        let status = startup.clone();
        let log = thread::spawn(move || {
            let mut stderr = BufReader::new(stderr);
            let mut log = Vec::new();
            let mut line = Vec::new();
            while stderr.read_until(b'\n', &mut line).is_ok_and(|n| n > 0) {
                if ready(&line) {
                    let mut status = status.lock().unwrap();
                    if let Startup::Waiting(stage) = &*status {
                        stage.store(HEARING, Ordering::Relaxed);
                    }
                    *status = Startup::Ready;
                }
                log.extend_from_slice(&line);
                line.clear();
            }
            log
        });
        Ok(Self {
            child: Some(child),
            log: Some(log),
            stdout,
            startup,
        })
    }

    pub(super) fn transcribe(mut self, wav: &[u8], stage: &Arc<AtomicU8>) -> io::Result<Output> {
        {
            let mut startup = self.startup.lock().unwrap();
            if !matches!(*startup, Startup::Ready) {
                stage.store(LOADING_SPEECH, Ordering::Relaxed);
                *startup = Startup::Waiting(stage.clone());
            }
        }
        let mut child = self.child.take().expect("speech helper");
        // Closing stdin tells the helper that the recording is complete.
        let sent = child.stdin.take().expect("piped stdin").write_all(wav);
        let result = child.wait();
        let stderr = self.log.take().unwrap().join().unwrap_or_default();
        stage.store(HEARING, Ordering::Relaxed);
        let status = result?;
        if status.success() && !matches!(sent, Err(ref e) if e.kind() == io::ErrorKind::BrokenPipe)
        {
            sent?;
        }
        let stdout = self
            .stdout
            .take()
            .map(|reader| {
                reader
                    .join()
                    .map_err(|_| io::Error::other("speech output reader failed"))?
            })
            .transpose()?
            .unwrap_or_default();
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // A cancelled or very short recording never sends audio.
        if let Some(mut child) = self.child.take() {
            child.kill().ok();
            child.wait().ok();
        }
        if let Some(log) = self.log.take() {
            log.join().ok();
        }
        if let Some(stdout) = self.stdout.take() {
            stdout.join().ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn the_helper_can_be_ready_before_receiving_audio() {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "printf 'ready\n' >&2; exec /bin/cat"])
            .stdout(Stdio::piped());
        let helper = Process::start(&mut command, |line| line == b"ready\n").unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        while !matches!(*helper.startup.lock().unwrap(), Startup::Ready) {
            assert!(Instant::now() < deadline, "helper never reported ready");
            thread::sleep(Duration::from_millis(5));
        }

        let stage = Arc::new(AtomicU8::new(HEARING));
        let output = helper.transcribe(b"audio", &stage).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"audio");
        assert_eq!(stage.load(Ordering::Relaxed), HEARING);
    }
}
