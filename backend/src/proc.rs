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
