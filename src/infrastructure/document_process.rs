use crate::{
    config::DocumentConfig,
    error::{Error, Result},
};
use std::process::Stdio;
use tokio::{io::AsyncReadExt, process::Command};

pub async fn bounded(
    config: &DocumentConfig,
    workspace: &std::path::Path,
    binary: &str,
    args: &[&str],
    cap: usize,
) -> Result<Vec<u8>> {
    let path = config.poppler_dir.join(binary);
    execute(config, workspace, &path, args, cap).await
}
pub async fn execute(
    config: &DocumentConfig,
    workspace: &std::path::Path,
    path: &std::path::Path,
    args: &[&str],
    cap: usize,
) -> Result<Vec<u8>> {
    if !path.is_absolute() {
        return Err(Error::unavailable());
    }
    let mut command = Command::new(path);
    command
        .args(args)
        .current_dir(workspace)
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let cpu = config.timeout_seconds;
    let memory = config.memory_mb * 1024 * 1024;
    // Only async-signal-safe setrlimit calls execute between fork and exec.
    unsafe {
        command.pre_exec(move || resource_limits(cpu, memory, cap as u64));
    }
    let mut child = command
        .spawn()
        .map_err(|_| Error::bad("Configured PDF extraction service is unavailable"))?;
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(config.timeout_seconds),
        async {
            let mut bytes = Vec::new();
            child
                .stdout
                .take()
                .ok_or_else(Error::unavailable)?
                .take(cap as u64 + 1)
                .read_to_end(&mut bytes)
                .await
                .map_err(|_| Error::unavailable())?;
            if bytes.len() > cap {
                return Err(Error::bad("PDF extraction exceeded its output limit"));
            }
            if !child
                .wait()
                .await
                .map_err(|_| Error::unavailable())?
                .success()
            {
                return Err(Error::bad(
                    "PDF extraction failed or exceeded its resource budget",
                ));
            }
            Ok(bytes)
        },
    )
    .await
    .map_err(|_| Error::bad("PDF extraction timed out"))?;
    output
}
fn resource_limits(cpu: u64, memory: u64, file_size: u64) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let limit = libc::rlimit {
            rlim_cur: memory as libc::rlim_t,
            rlim_max: memory as libc::rlim_t,
        };
        if unsafe { libc::setrlimit(libc::RLIMIT_AS, &limit) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = memory; // macOS rejects address-space limits; the Linux runtime enforces this cap.
    for (resource, value) in [
        (libc::RLIMIT_CPU, cpu),
        (libc::RLIMIT_FSIZE, file_size),
        (libc::RLIMIT_NOFILE, 64),
    ] {
        let limit = libc::rlimit {
            rlim_cur: value as libc::rlim_t,
            rlim_max: value as libc::rlim_t,
        };
        if unsafe { libc::setrlimit(resource, &limit) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}
