use std::collections::VecDeque;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Outcome of running a child process to completion.
pub struct RunResult {
    pub success: bool,
    /// Tail of stderr/stdout, useful for surfacing failure reasons to the UI.
    pub error_tail: String,
}

impl RunResult {
    /// The captured output as an error message, or `fallback` if the tool said nothing.
    pub fn error_or(self, fallback: &str) -> String {
        if self.error_tail.trim().is_empty() {
            fallback.to_string()
        } else {
            self.error_tail
        }
    }
}

const ERROR_TAIL_LINES: usize = 25;

/// Run a command, forwarding every line of stdout *and* stderr to `on_line`
/// as it is produced. The closure is the place to parse progress.
///
/// Returns once the child exits. The last handful of output lines are kept so
/// a meaningful error can be shown when the process fails.
pub async fn run_with_progress<F>(cmd: &mut Command, mut on_line: F) -> std::io::Result<RunResult>
where
    F: FnMut(&str),
{
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    let mut out_lines = BufReader::new(stdout).lines();
    let mut err_lines = BufReader::new(stderr).lines();

    let mut tail: VecDeque<String> = VecDeque::with_capacity(ERROR_TAIL_LINES);
    let mut push_tail = |line: &str| {
        if tail.len() == ERROR_TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(line.to_string());
    };

    let mut out_done = false;
    let mut err_done = false;
    loop {
        tokio::select! {
            res = out_lines.next_line(), if !out_done => match res {
                Ok(Some(line)) => { on_line(&line); push_tail(&line); }
                _ => out_done = true,
            },
            res = err_lines.next_line(), if !err_done => match res {
                Ok(Some(line)) => { on_line(&line); push_tail(&line); }
                _ => err_done = true,
            },
            else => break,
        }
    }

    let status = child.wait().await?;
    Ok(RunResult {
        success: status.success(),
        error_tail: tail.into_iter().collect::<Vec<_>>().join("\n"),
    })
}

/// Seconds of output written so far, from one line of ffmpeg's `-progress` output.
pub fn out_time(line: &str) -> Option<f64> {
    parse_ffmpeg_time(line.strip_prefix("out_time=")?)
}

/// Parse ffmpeg's `out_time` value ("HH:MM:SS.microseconds") into seconds.
fn parse_ffmpeg_time(s: &str) -> Option<f64> {
    let mut parts = s.trim().split(':');
    let (h, m, sec) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let h: f64 = h.parse().ok()?;
    let m: f64 = m.parse().ok()?;
    let sec: f64 = sec.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ffmpeg_progress_lines() {
        assert_eq!(out_time("out_time=00:01:02.500000"), Some(62.5));
        assert_eq!(out_time("out_time=01:00:00.000000"), Some(3600.0));
        // ffmpeg prints N/A before the first frame is written.
        assert_eq!(out_time("out_time=N/A"), None);
        assert_eq!(out_time("out_time_us=62500000"), None);
        assert_eq!(out_time("frame=12"), None);
    }
}
