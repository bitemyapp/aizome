//! Bounded, cancellable workers. Credentials travel only through pipes,
//! never arguments, environment variables, files or diagnostic output.
use crate::{AgentError, Result};
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

pub const MAX_BYTES: usize = 1024 * 1024;

#[derive(Clone, Default)]
pub struct Cancel(pub Arc<AtomicBool>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn check(&self) -> Result<()> {
        if self.0.load(Ordering::Acquire) {
            Err(AgentError::AgentCanceled("Network authentication was cancelled".into()))
        } else {
            Ok(())
        }
    }
}

pub fn run(command: &mut Command, input: Vec<u8>, cancel: &Cancel) -> Result<Vec<u8>> {
    cancel.check()?;
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    // A plugin's descendants must not outlive cancellation either.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|_| AgentError::Failed("Could not start credential helper".into()))?;
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let write = std::thread::spawn(move || stdin.write_all(&input));
    let read = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(120);
    let status = loop {
        if cancel.check().is_err() || Instant::now() >= deadline {
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.wait();
            let _ = write.join();
            let _ = read.join();
            cancel.check()?;
            return Err(AgentError::AgentCanceled("Network authentication timed out".into()));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => {
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.wait();
                let _ = write.join();
                let _ = read.join();
                return Err(AgentError::Failed("Credential helper failed".into()));
            }
        }
    };
    // Helpers must not leave inherited pipe descriptors in descendants.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = write.join();
    let output = read
        .join()
        .ok()
        .and_then(|r| r.ok())
        .filter(|b| b.len() <= MAX_BYTES)
        .ok_or_else(|| AgentError::Failed("Invalid credential helper response".into()))?;
    cancel.check()?;
    if !status.success() {
        return Err(AgentError::UserCanceled("Authentication was cancelled or the helper could not complete".into()));
    }
    Ok(output)
}
