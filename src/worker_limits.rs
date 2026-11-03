// SPDX-License-Identifier: AGPL-3.0-only
#[cfg(windows)]
pub(crate) fn configure(command: &mut tokio::process::Command) {
    // Workers only communicate through pipes; Windows must not create a new
    // console window for every render when the server has no attached console.
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
}

#[cfg(windows)]
pub(crate) fn attach(
    child: &tokio::process::Child,
) -> crate::Result<std::os::windows::io::OwnedHandle> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
        JOB_OBJECT_LIMIT_PROCESS_TIME, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    // SAFETY: null optional security/name arguments create an unnamed,
    // non-inheritable job. OwnedHandle closes the returned handle exactly once.
    let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if raw.is_null() {
        return Err("Cannot create render worker limits".into());
    }
    let job = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        | JOB_OBJECT_LIMIT_PROCESS_MEMORY
        | JOB_OBJECT_LIMIT_PROCESS_TIME
        | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
    limits.BasicLimitInformation.PerProcessUserTimeLimit = 12 * 10_000_000;
    limits.BasicLimitInformation.ActiveProcessLimit = 1;
    limits.ProcessMemoryLimit = 256 * 1024 * 1024;
    let process = child.raw_handle().ok_or("Render worker already exited")?;
    // SAFETY: both handles stay live throughout the calls. The struct layout and
    // size match the Win32 API. The child has not received chart data yet.
    let configured = unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
        )
    };
    if configured == 0 || unsafe { AssignProcessToJobObject(job.as_raw_handle(), process) } == 0 {
        return Err("Cannot apply render worker limits".into());
    }
    Ok(job)
}

#[cfg(all(test, windows))]
mod tests {
    #[tokio::test]
    async fn worker_process_starts_without_a_console_window() {
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        super::configure(&mut command);
        command
            .args([
                "--exact",
                "worker_limits::tests::console_probe",
                "--ignored",
                "--nocapture",
            ])
            .kill_on_drop(true);
        let output = tokio::time::timeout(std::time::Duration::from_secs(10), command.output())
            .await
            .unwrap()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    }

    #[test]
    #[ignore = "Runs in the child process launched by the console-window test"]
    fn console_probe() {
        // SAFETY: GetConsoleWindow takes no parameters and only queries the
        // calling process. A null handle means there is no attached console.
        assert!(unsafe { windows_sys::Win32::System::Console::GetConsoleWindow() }.is_null());
    }
}
