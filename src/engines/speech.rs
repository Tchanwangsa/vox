//! A one-shot speech helper that can load its model before it receives audio.

use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::process::CommandExt as _;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

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
            .process_group(0)
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

    pub(super) fn transcribe(
        mut self,
        wav: &[u8],
        stage: &Arc<AtomicU8>,
        cancel: &AtomicBool,
    ) -> io::Result<Output> {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        {
            let mut startup = self.startup.lock().unwrap();
            if !matches!(*startup, Startup::Ready) {
                stage.store(LOADING_SPEECH, Ordering::Relaxed);
                *startup = Startup::Waiting(stage.clone());
            }
        }
        let mut child = self.child.take().expect("speech helper");
        // Write WAV on a helper thread so cancellation can kill the child even
        // while the model is still loading and hasn't begun reading stdin yet.
        let mut stdin = child.stdin.take().expect("piped stdin");
        let wav = wav.to_vec();
        let writer = thread::spawn(move || {
            let result = stdin.write_all(&wav);
            drop(stdin);
            result
        });
        let status = loop {
            if cancel.load(Ordering::Relaxed) {
                kill_group(&mut child);
                writer.join().ok();
                if let Some(log) = self.log.take() {
                    log.join().ok();
                }
                if let Some(stdout) = self.stdout.take() {
                    stdout.join().ok();
                }
                stage.store(HEARING, Ordering::Relaxed);
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
            if let Some(status) = child.try_wait()? {
                break status;
            }
            thread::sleep(Duration::from_millis(25));
        };
        let sent = writer
            .join()
            .map_err(|_| io::Error::other("speech input writer failed"))?;
        let stderr = self.log.take().unwrap().join().unwrap_or_default();
        stage.store(HEARING, Ordering::Relaxed);
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

fn kill_group(child: &mut Child) {
    unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
    child.kill().ok();
    child.wait().ok();
}

impl Drop for Process {
    fn drop(&mut self) {
        // A cancelled or very short recording never sends audio.
        if let Some(mut child) = self.child.take() {
            kill_group(&mut child);
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
        let cancel = AtomicBool::new(false);
        let output = helper.transcribe(b"audio", &stage, &cancel).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"audio");
        assert_eq!(stage.load(Ordering::Relaxed), HEARING);
    }

    #[test]
    fn cancelling_transcription_stops_the_helper_promptly() {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "printf 'ready\n' >&2; /bin/sleep 5"])
            .stdout(Stdio::piped());
        let helper = Process::start(&mut command, |line| line == b"ready\n").unwrap();
        let stage = Arc::new(AtomicU8::new(HEARING));
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let trigger = thread::spawn(move || {
            thread::sleep(Duration::from_millis(80));
            flag.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();
        let err = helper.transcribe(b"audio", &stage, &cancel).unwrap_err();
        trigger.join().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::Interrupted);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
