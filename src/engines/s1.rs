//! Tidying on this Mac: S1-mini by Superwhisper, a 0.6B text normaliser for
//! speech-to-text output, served by llama.cpp's `llama-server` with Metal.
//!
//! One server, started on first use (or warmed when a dictation starts) on a
//! port the OS picks, stopped after five idle minutes and when Vox quits. It
//! cannot outlive Vox: it runs under a `/bin/sh` watchdog that holds the read
//! end of a pipe whose write end is ours, so when Vox closes it or dies, the
//! read returns and the watchdog kills the server.
//!
//! S1-mini cleans English only, a piece of about 1,000 tokens at a time, each
//! sent with the system prompt and control line it was trained on.

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::os::unix::process::CommandExt as _;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::{LOADING_TIDY, TIDYING};

const IDLE: Duration = Duration::from_secs(5 * 60);
const IDLE_CHECK: Duration = Duration::from_secs(15);
const STARTUP: Duration = Duration::from_secs(60);
const STOP_GRACE: Duration = Duration::from_secs(3);
const ALIAS: &str = "vox-s1-mini";

/// The input format S1-mini was trained on, word for word; anything else and
/// it garbles.
const SYSTEM: &str = "You are a text normalizer for speech-to-text transcripts. The input begins with a control line specifying the styling, structure, and context settings; clean the transcript to match those settings and output only the cleaned text.";
const CONTROL_LINE: &str = "[Styling: semi-formal] [Structure: lists] [Context: general]";
const CHUNK_TOKENS: usize = 1_000;

const WATCHDOG: &str = r#"exec 3<&0
"$0" "$@" </dev/null 3<&- >&2 &
child=$!
echo "$child"
exec >/dev/null
(read -r _ <&3; kill "$child" 2>/dev/null) &
exec 3<&- </dev/null
wait "$child""#;

#[derive(Debug, Clone, PartialEq)]
pub struct Launch {
    pub helper: PathBuf,
    pub model: PathBuf,
}

struct Running {
    launch: Launch,
    shell: Child,
    /// Dropping our end of the watchdog's pipe stops the server.
    pipe: Option<ChildStdin>,
    server_pid: u32,
    port: u16,
    generation: u64,
    busy: usize,
    last_used: Instant,
}

static SERVER: Mutex<Option<Running>> = Mutex::new(None);

/// The transcript tidied, or as heard when it is not English.
pub fn tidy(
    launch: &Launch,
    transcript: &str,
    language: &str,
    stage: &AtomicU8,
    cancel: &AtomicBool,
) -> Result<String, String> {
    if !english(transcript, language) {
        return Ok(transcript.to_string());
    }
    let cleaned = chunks(transcript, CHUNK_TOKENS)
        .iter()
        .map(|piece| {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".into());
            }
            complete(launch, &request(piece), stage, cancel).and_then(|value| reply(&value))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(join(&cleaned))
}

/// Load the model now, so the tidy at the end of a dictation finds it ready.
pub fn warm(launch: &Launch) {
    if acquire(launch).is_ok() {
        release();
    }
}

pub fn stop() {
    let taken = lock().take();
    if let Some(running) = taken {
        shut(running);
    }
}

fn complete(
    launch: &Launch,
    body: &Value,
    stage: &AtomicU8,
    cancel: &AtomicBool,
) -> Result<Value, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled".into());
    }
    stage.store(LOADING_TIDY, Ordering::Relaxed);
    let port = acquire(launch)?;
    if cancel.load(Ordering::Relaxed) {
        release();
        return Err("Cancelled".into());
    }
    stage.store(TIDYING, Ordering::Relaxed);
    let sent = ureq::post(&format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .set("Content-Type", "application/json")
        .timeout(Duration::from_secs(60))
        .send_string(&body.to_string());
    release();
    match sent {
        Ok(response) => response
            .into_string()
            .map_err(|e| e.to_string())
            .and_then(|text| serde_json::from_str(&text).map_err(|e| e.to_string()))
            .map_err(|e| format!("S1-mini's reply could not be read: {e}")),
        Err(ureq::Error::Status(status, _)) => Err(format!("S1-mini refused the request ({status})")),
        Err(e) => Err(format!("Could not reach S1-mini: {e}")),
    }
}

/// Output that hit its cap is a loop, not a cleanup.
fn reply(value: &Value) -> Result<String, String> {
    let choice = &value["choices"][0];
    if choice["finish_reason"] == "length" {
        return Err("S1-mini's output ran past its length cap".into());
    }
    choice["message"]["content"]
        .as_str()
        .map(|text| text.trim().to_string())
        .ok_or_else(|| "S1-mini's reply held no message".into())
}

fn lock() -> std::sync::MutexGuard<'static, Option<Running>> {
    SERVER.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn alive(running: &mut Running) -> bool {
    matches!(running.shell.try_wait(), Ok(None))
}

/// The server's port, counted busy. Starting holds the lock, so a second
/// caller waits for the same server rather than starting another.
fn acquire(launch: &Launch) -> Result<u16, String> {
    let mut held = lock();
    if let Some(running) = held.as_mut() {
        if alive(running) && running.launch == *launch {
            running.busy += 1;
            running.last_used = Instant::now();
            return Ok(running.port);
        }
    }
    if let Some(old) = held.take() {
        shut(old);
    }
    let mut running = start(launch)?;
    running.busy = 1;
    let (port, generation) = (running.port, running.generation);
    *held = Some(running);
    drop(held);
    std::thread::spawn(move || watch_idle(generation));
    Ok(port)
}

fn release() {
    if let Some(running) = lock().as_mut() {
        running.busy = running.busy.saturating_sub(1);
        running.last_used = Instant::now();
    }
}

fn watch_idle(generation: u64) {
    loop {
        std::thread::sleep(IDLE_CHECK);
        let mut held = lock();
        let idle = match held.as_ref() {
            Some(r) if r.generation == generation => r.busy == 0 && r.last_used.elapsed() >= IDLE,
            _ => return,
        };
        if idle {
            let running = held.take();
            drop(held);
            running.map(shut);
            return;
        }
    }
}

fn start(launch: &Launch) -> Result<Running, String> {
    static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| format!("Could not find a free port for S1-mini: {e}"))?;
    let log_path = crate::config::data_dir().join("llama-server.log");
    let log = std::fs::File::create(&log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
    // Greedy, one slot, a 4,096-token context; thinking off through the chat
    // template, the empty think block S1-mini was trained on.
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(WATCHDOG)
        .arg(&launch.helper)
        .arg("-m")
        .arg(&launch.model)
        .args(["-c", "4096", "-np", "1", "--temp", "0", "--jinja", "--reasoning", "off"])
        .args(["--host", "127.0.0.1", "--port", &port.to_string(), "--alias", ALIAS])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(log);
    // macOS's Metal shader cache (`libCoreFSCache`) leaves its `flock`-locked
    // descriptors without `FD_CLOEXEC`; closing inherited fds >= 3 prevents
    // `llama-server` from holding Vox's Metal cache locks across restarts.
    unsafe {
        cmd.pre_exec(|| {
            for fd in 3..1024 {
                libc::close(fd);
            }
            Ok(())
        });
    }
    let mut shell = cmd
        .spawn()
        .map_err(|e| format!("Could not start S1-mini's server: {e}"))?;
    let pipe = shell.stdin.take();
    let mut line = String::new();
    if let Some(stdout) = shell.stdout.take() {
        BufReader::new(stdout).read_line(&mut line).ok();
    }
    let mut running = Running {
        launch: launch.clone(),
        shell,
        pipe,
        server_pid: line.trim().parse().unwrap_or(0),
        port,
        generation: GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1,
        busy: 0,
        last_used: Instant::now(),
    };
    let started = Instant::now();
    loop {
        if !alive(&mut running) {
            shut(running);
            return Err(format!(
                "S1-mini's server stopped while loading. Its log is at {}",
                log_path.display()
            ));
        }
        // `/health` answers 503 while the model loads and 200 once it serves.
        let healthy = ureq::get(&format!("http://127.0.0.1:{port}/health"))
            .timeout(Duration::from_secs(2))
            .call()
            .is_ok();
        if healthy {
            return Ok(running);
        }
        if started.elapsed() >= STARTUP {
            shut(running);
            return Err("S1-mini's server did not load the model within a minute".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Close the watchdog's pipe so it kills the server; past the grace, kill
/// both outright.
fn shut(mut running: Running) {
    drop(running.pipe.take());
    let deadline = Instant::now() + STOP_GRACE;
    while Instant::now() < deadline {
        if !matches!(running.shell.try_wait(), Ok(None)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    if running.server_pid > 0 {
        unsafe { libc::kill(running.server_pid as libc::pid_t, libc::SIGKILL) };
    }
    running.shell.kill().ok();
    running.shell.wait().ok();
}

/// About four characters a token for English through Qwen's tokenizer.
fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// One chat completion request for one piece. The cap is 1.3 × the input and
/// a margin: a cleanup is never much longer than what was said, and an
/// uncapped run can loop for thousands of tokens.
fn request(piece: &str) -> Value {
    json!({
        "messages": [
            {"role": "system", "content": SYSTEM},
            {"role": "user", "content": format!("{CONTROL_LINE}\n{piece}")},
        ],
        "temperature": 0,
        "max_tokens": estimate_tokens(piece) * 13 / 10 + 32,
    })
}

/// S1-mini knows English only: a transcript with letters from another script,
/// or speech set to another language, is left as it was heard.
pub fn english(transcript: &str, language: &str) -> bool {
    let latin = |c: char| {
        matches!(c as u32, 0x41..=0x5A | 0x61..=0x7A | 0xAA | 0xB5 | 0xBA | 0xC0..=0x24F | 0x1E00..=0x1EFF)
    };
    matches!(language, "en" | "auto") && !transcript.chars().any(|c| c.is_alphabetic() && !latin(c))
}

/// `transcript` in pieces of at most `limit` estimated tokens, cut after a
/// sentence's end; a sentence longer than `limit` is cut between words.
fn chunks(transcript: &str, limit: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current = String::new();
    let flush = |current: &mut String, pieces: &mut Vec<String>| {
        if !current.trim().is_empty() {
            pieces.push(std::mem::take(current));
        }
        current.clear();
    };
    for sentence in sentences(transcript) {
        for part in by_words(sentence, limit) {
            let joined = if current.is_empty() { part.clone() } else { format!("{current} {part}") };
            if estimate_tokens(&joined) > limit {
                flush(&mut current, &mut pieces);
                current = part;
            } else {
                current = joined;
            }
        }
    }
    flush(&mut current, &mut pieces);
    pieces
}

fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let next = chars.peek().map(|&(_, n)| n);
        let ends = c == '\n'
            || (matches!(c, '.' | '?' | '!' | '…') && next.is_none_or(char::is_whitespace));
        if ends {
            let end = i + c.len_utf8();
            out.push(text[start..end].trim());
            start = end;
        }
    }
    out.push(text[start..].trim());
    out.retain(|s| !s.is_empty());
    out
}

fn by_words(sentence: &str, limit: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current = String::new();
    for word in sentence.split_whitespace() {
        let joined = if current.is_empty() { word.to_string() } else { format!("{current} {word}") };
        if !current.is_empty() && estimate_tokens(&joined) > limit {
            pieces.push(std::mem::replace(&mut current, word.to_string()));
        } else {
            current = joined;
        }
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces
}

/// The pieces' cleanups as one message: prose runs on with a space, and a
/// piece holding a list or paragraphs is set apart by a blank line.
fn join(cleaned: &[String]) -> String {
    let mut out = String::new();
    let mut last_multiline = false;
    for piece in cleaned.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
        let multiline = piece.contains('\n');
        if !out.is_empty() {
            out.push_str(if multiline || last_multiline { "\n\n" } else { " " });
        }
        out.push_str(piece);
        last_multiline = multiline;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_english_is_tidied() {
        assert!(english("send the report on Friday", "auto"));
        assert!(english("café déjà vu", "en"));
        assert!(!english("สวัสดี hello", "auto"));
        assert!(!english("hello", "th"));
    }

    #[test]
    fn long_transcripts_are_cut_at_sentences() {
        let text = "One two three. Four five six. Seven eight nine.";
        let pieces = chunks(text, 5);
        assert_eq!(pieces, ["One two three.", "Four five six.", "Seven eight nine."]);
        assert_eq!(chunks(text, 1_000), [text]);
        assert!(chunks("a b c d e f g h i j k l", 2).len() > 1);
    }

    #[test]
    fn pieces_join_as_prose_or_set_apart_lists() {
        let pieces = ["First.".to_string(), String::new(), "- a\n- b".to_string(), "Last.".to_string()];
        assert_eq!(join(&pieces), "First.\n\n- a\n- b\n\nLast.");
    }
}
