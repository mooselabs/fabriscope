//! Runs the `bd` and `scrolls` command-line tools.

use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;

/// One external program, run with fixed leading arguments from a directory.
#[derive(Clone, Debug)]
pub struct Tool {
    pub name: &'static str,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

/// Runs the tool and returns its standard output. A non-zero exit, a timeout
/// or non-UTF-8 output is an error carrying the tool's own message.
pub async fn run(tool: &Tool, args: &[&str], timeout_secs: u64) -> Result<String, String> {
    let output = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        Command::new(&tool.program)
            .args(&tool.args)
            .args(args)
            .current_dir(&tool.cwd)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| {
        format!(
            "`{} {}` took longer than {timeout_secs}s",
            tool.name,
            args.join(" ")
        )
    })?
    .map_err(|e| format!("could not run {}: {e}", tool.program.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = [stderr.trim(), stdout.trim()]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or("no output");
        return Err(format!(
            "`{} {}` failed: {}",
            tool.name,
            args.join(" "),
            detail.lines().take(6).collect::<Vec<_>>().join(" ")
        ));
    }
    String::from_utf8(output.stdout).map_err(|_| {
        format!(
            "`{} {}` printed non-UTF-8 output",
            tool.name,
            args.join(" ")
        )
    })
}
