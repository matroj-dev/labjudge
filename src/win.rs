use std::io::{self, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct LimitedOutput {
    pub status: Option<ExitStatus>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
}

#[cfg(windows)]
pub struct JobHandle {
    handle: usize,
}

#[cfg(windows)]
impl JobHandle {
    pub fn new(memory_limit: Option<u64>) -> io::Result<Self> {
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_JOB_MEMORY,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if let Some(limit) = memory_limit {
            info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
            info.JobMemoryLimit = limit as usize;
        }

        let ok = unsafe {
            SetInformationJobObject(
                raw,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            unsafe { windows_sys::Win32::Foundation::CloseHandle(raw) };
            return Err(error);
        }

        Ok(Self {
            handle: raw as usize,
        })
    }

    pub fn assign(&self, child: &Child) -> io::Result<()> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;

        let process = child.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE;
        let ok = unsafe {
            AssignProcessToJobObject(
                self.handle as windows_sys::Win32::Foundation::HANDLE,
                process,
            )
        };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn terminate(&self) {
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(
                self.handle as windows_sys::Win32::Foundation::HANDLE,
                1,
            );
        }
    }
}

#[cfg(windows)]
impl Drop for JobHandle {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(
                self.handle as windows_sys::Win32::Foundation::HANDLE,
            );
        }
    }
}

#[cfg(not(windows))]
pub struct JobHandle;

#[cfg(not(windows))]
impl JobHandle {
    pub fn new(_memory_limit: Option<u64>) -> io::Result<Self> {
        Ok(Self)
    }

    pub fn assign(&self, _child: &Child) -> io::Result<()> {
        Ok(())
    }

    pub fn terminate(&self) {}
}

pub fn run_limited(
    command: &mut Command,
    timeout: Duration,
    memory_limit: Option<u64>,
) -> io::Result<LimitedOutput> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }

    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let job = JobHandle::new(memory_limit)?;
    let mut child = command.spawn()?;
    if let Err(error) = job.assign(&child) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }

    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let stdout_reader = thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stdout.read_to_end(&mut buffer);
        buffer
    });
    let stderr_reader = thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stderr.read_to_end(&mut buffer);
        buffer
    });

    let start = Instant::now();
    let (status, timed_out) = loop {
        if let Some(found) = child.try_wait()? {
            break (Some(found), false);
        }
        if start.elapsed() >= timeout {
            job.terminate();
            let _ = child.kill();
            break (child.wait().ok(), true);
        }
        thread::sleep(Duration::from_millis(20));
    };

    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    Ok(LimitedOutput {
        status,
        stdout,
        stderr,
        timed_out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn captures_output_and_times_out() {
        let mut command = Command::new("cmd");
        command.args(["/c", "echo", "hello"]);
        let output = run_limited(&mut command, Duration::from_secs(5), None).unwrap();
        assert!(!output.timed_out);
        assert!(output.status.unwrap().success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("hello"));

        let mut command = Command::new("ping");
        command.args(["-n", "10", "127.0.0.1"]);
        let started = Instant::now();
        let output = run_limited(&mut command, Duration::from_millis(200), None).unwrap();
        assert!(output.timed_out);
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
