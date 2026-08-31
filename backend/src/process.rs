use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub(crate) struct ExecutionResult {
    pub(crate) output: Output,
    pub(crate) timed_out: bool,
    pub(crate) stdout_truncated: bool,
    pub(crate) stderr_truncated: bool,
}

pub(crate) fn execute_output(
    command: &mut Command,
    timeout: Duration,
    output_limit: usize,
) -> std::io::Result<ExecutionResult> {
    execute(command, None, timeout, output_limit, None)
}

pub(crate) fn execute_stdin(
    command: &mut Command,
    input: &str,
    timeout: Duration,
    output_limit: usize,
) -> std::io::Result<ExecutionResult> {
    execute(command, Some(input.as_bytes()), timeout, output_limit, None)
}

pub(crate) fn execute_stdin_with_lease(
    command: &mut Command,
    input: &str,
    timeout: Duration,
    output_limit: usize,
    lease: File,
) -> std::io::Result<ExecutionResult> {
    execute(
        command,
        Some(input.as_bytes()),
        timeout,
        output_limit,
        Some(lease),
    )
}

fn execute(
    command: &mut Command,
    input: Option<&[u8]>,
    timeout: Duration,
    output_limit: usize,
    mut lease: Option<File>,
) -> std::io::Result<ExecutionResult> {
    if let Some(lease) = &lease {
        make_inheritable(lease)?;
    }
    command.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(input) = input {
        if let Some(mut stdin) = child.stdin.take() {
            if let Err(error) = stdin.write_all(input) {
                let _ = child.wait();
                return Err(error);
            }
        }
    }

    let stdout_reader = child
        .stdout
        .take()
        .map(|reader| spawn_reader(reader, output_limit));
    let stderr_reader = child
        .stderr
        .take()
        .map(|reader| spawn_reader(reader, output_limit));

    let started = Instant::now();
    let mut timed_out = false;
    let mut child_exited = false;
    loop {
        if !child_exited {
            child_exited = child.try_wait()?.is_some();
        }
        let stdout_finished = stdout_reader
            .as_ref()
            .map(|(_, handle)| handle.is_finished())
            .unwrap_or(true);
        let stderr_finished = stderr_reader
            .as_ref()
            .map(|(_, handle)| handle.is_finished())
            .unwrap_or(true);
        if child_exited && stdout_finished && stderr_finished {
            break;
        }
        if started.elapsed() >= timeout {
            timed_out = true;
            let (stdout, stdout_truncated) = snapshot_reader(&stdout_reader);
            let (stderr, stderr_truncated) = snapshot_reader(&stderr_reader);
            // A timeout is an observation, not authorization to terminate the
            // command tree. The Android mutation may already be in progress,
            // so leave it running and return an uncertain timeout result. A
            // separate process retains the flock lease until the direct
            // runner exits; unlike a thread, it survives usbsrctl's
            // process::exit. The runner itself also inherited the descriptor.
            if let Some(lease) = lease.take() {
                spawn_lock_keeper(child.id(), lease)?;
            }
            std::thread::spawn(move || {
                let _ = child.wait();
                join_reader(stdout_reader);
                join_reader(stderr_reader);
            });
            return Ok(ExecutionResult {
                output: Output {
                    status: std::process::ExitStatus::from_raw(124 << 8),
                    stdout,
                    stderr,
                },
                timed_out,
                stdout_truncated,
                stderr_truncated,
            });
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let status = child.wait()?;
    let (stdout, stdout_truncated) = finish_reader(stdout_reader);
    let (stderr, stderr_truncated) = finish_reader(stderr_reader);
    Ok(ExecutionResult {
        output: Output {
            status,
            stdout,
            stderr,
        },
        timed_out,
        stdout_truncated,
        stderr_truncated,
    })
}

fn spawn_lock_keeper(pid: u32, lease: File) -> std::io::Result<()> {
    let mut keeper = Command::new("sh");
    keeper
        .args([
            "-c",
            "while kill -0 \"$1\" 2>/dev/null; do sleep 0.1; done",
            "lock-keeper",
            &pid.to_string(),
        ])
        // Install the lease as fd 0 so shells that proactively close unknown
        // inherited descriptors cannot discard the flock.
        .stdin(Stdio::from(lease))
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    keeper.spawn()?;
    Ok(())
}

fn make_inheritable(file: &File) -> std::io::Result<()> {
    const F_GETFD: i32 = 1;
    const F_SETFD: i32 = 2;
    const FD_CLOEXEC: i32 = 1;
    let descriptor = file.as_raw_fd();
    let flags = unsafe { fcntl(descriptor, F_GETFD, 0) };
    if flags == -1 {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe { fcntl(descriptor, F_SETFD, flags & !FD_CLOEXEC) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

unsafe extern "C" {
    fn fcntl(fd: i32, command: i32, argument: i32) -> i32;
}

#[derive(Default)]
struct CapturedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

type Reader = (Arc<Mutex<CapturedOutput>>, JoinHandle<()>);

fn spawn_reader<R: Read + Send + 'static>(mut reader: R, limit: usize) -> Reader {
    let captured = Arc::new(Mutex::new(CapturedOutput {
        bytes: Vec::with_capacity(limit.min(8192)),
        truncated: false,
    }));
    let writer = Arc::clone(&captured);
    let handle = std::thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(size) => {
                    let mut output = writer.lock().unwrap_or_else(|error| error.into_inner());
                    let available = limit.saturating_sub(output.bytes.len());
                    if size > available {
                        output.truncated = true;
                    }
                    output
                        .bytes
                        .extend_from_slice(&buffer[..size.min(available)]);
                }
                Err(_) => break,
            }
        }
    });
    (captured, handle)
}

fn snapshot_reader(reader: &Option<Reader>) -> (Vec<u8>, bool) {
    reader
        .as_ref()
        .map(|(captured, _)| {
            let output = captured.lock().unwrap_or_else(|error| error.into_inner());
            (output.bytes.clone(), output.truncated)
        })
        .unwrap_or_default()
}

fn finish_reader(reader: Option<Reader>) -> (Vec<u8>, bool) {
    if let Some((captured, handle)) = reader {
        let _ = handle.join();
        let output = captured.lock().unwrap_or_else(|error| error.into_inner());
        (output.bytes.clone(), output.truncated)
    } else {
        (Vec::new(), false)
    }
}

fn join_reader(reader: Option<Reader>) {
    if let Some((_, handle)) = reader {
        let _ = handle.join();
    }
}
