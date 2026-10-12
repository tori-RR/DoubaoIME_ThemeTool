//! Process operations are restricted to verified paths and the current session.
use std::ffi::c_void;
use std::mem::size_of;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

type Handle = *mut c_void;
const ERROR_ACCESS_DENIED: u32 = 5;
const PROCESS_PROBE_ATTEMPTS: usize = 6;
const PROCESS_PROBE_RETRY_DELAY: Duration = Duration::from_millis(25);

struct OwnedHandle(Handle);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

#[repr(C)]
struct ProcessEntry {
    size: u32,
    usage: u32,
    pid: u32,
    heap: usize,
    module: u32,
    threads: u32,
    parent: u32,
    priority: i32,
    flags: u32,
    exe: [u16; 260],
}
#[derive(Clone, Debug)]
pub struct ImeProcess {
    pub pid: u32,
    pub session: u32,
    pub path: PathBuf,
}

pub fn session() -> Result<u32, String> {
    let mut id = 0;
    if unsafe { ProcessIdToSessionId(std::process::id(), &mut id) } == 0 {
        return Err("无法查询当前会话".into());
    }
    Ok(id)
}

enum ProbeError {
    Changed,
    Denied(String),
    Fatal(String),
}

fn probe_failure(permission_message: &str) -> ProbeError {
    let code = unsafe { GetLastError() };
    if code == ERROR_ACCESS_DENIED {
        ProbeError::Denied(permission_message.into())
    } else {
        ProbeError::Changed
    }
}

fn path_of_probe(handle: Handle) -> Result<PathBuf, ProbeError> {
    let mut buf = [0u16; 32768];
    let mut len = buf.len() as u32;
    if unsafe { QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len) } == 0 {
        return Err(probe_failure(
            "无法核验输入法进程路径，请以普通用户重新启动豆包输入法",
        ));
    }
    Ok(PathBuf::from(String::from_utf16_lossy(
        &buf[..len as usize],
    )))
}

fn path_of(handle: Handle) -> Result<PathBuf, String> {
    path_of_probe(handle).map_err(|err| match err {
        ProbeError::Changed => "输入法进程发生变化，请重试".into(),
        ProbeError::Denied(message) | ProbeError::Fatal(message) => message,
    })
}

fn processes_once() -> Result<Vec<ImeProcess>, ProbeError> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(2, 0) };
    if snapshot as isize == -1 {
        return Err(ProbeError::Fatal("无法枚举输入法进程".into()));
    }
    let snapshot = OwnedHandle(snapshot);
    let mut entry: ProcessEntry = unsafe { std::mem::zeroed() };
    entry.size = size_of::<ProcessEntry>() as u32;
    let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) };
    let mut list = Vec::new();
    while more != 0 {
        let n = entry
            .exe
            .iter()
            .position(|x| *x == 0)
            .unwrap_or(entry.exe.len());
        let name = String::from_utf16_lossy(&entry.exe[..n]);
        if name.eq_ignore_ascii_case("ImeService.exe")
            || name.eq_ignore_ascii_case("ImeWatchdog.exe")
        {
            let mut session_id = 0;
            if unsafe { ProcessIdToSessionId(entry.pid, &mut session_id) } == 0 {
                return Err(probe_failure("无法核验输入法进程所属会话"));
            }
            let h = unsafe { OpenProcess(0x1000, 0, entry.pid) };
            if h.is_null() {
                return Err(probe_failure(
                    "无法核验输入法进程，请先正常退出并重新启动豆包输入法",
                ));
            }
            let h = OwnedHandle(h);
            list.push(ImeProcess {
                pid: entry.pid,
                session: session_id,
                path: path_of_probe(h.0)?,
            });
        }
        more = unsafe { Process32NextW(snapshot.0, &mut entry) };
    }
    Ok(list)
}

pub fn processes() -> Result<Vec<ImeProcess>, String> {
    retry_probe(processes_once, || {
        std::thread::sleep(PROCESS_PROBE_RETRY_DELAY);
    })
}

fn retry_probe(
    mut probe: impl FnMut() -> Result<Vec<ImeProcess>, ProbeError>,
    mut pause: impl FnMut(),
) -> Result<Vec<ImeProcess>, String> {
    for attempt in 0..PROCESS_PROBE_ATTEMPTS {
        match probe() {
            Ok(processes) => return Ok(processes),
            Err(ProbeError::Fatal(message)) => return Err(message),
            // A restarting process can briefly reject a path query. Retry the
            // entire snapshot, but never accept a process with an unknown path.
            Err(_) if attempt + 1 < PROCESS_PROBE_ATTEMPTS => {
                pause();
            }
            Err(ProbeError::Denied(message)) => return Err(message),
            Err(ProbeError::Changed) => {
                return Err("输入法进程持续变化，请稍后重试".into());
            }
        }
    }
    unreachable!()
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy()
        .eq_ignore_ascii_case(&b.to_string_lossy())
}

pub fn stop(root: &Path, version: &str) -> Result<(), String> {
    let current = session()?;
    let watchdog = root.join("bootstrap/ImeWatchdog.exe");
    let service = root.join("versions").join(version).join("ImeService.exe");
    if processes()?.iter().any(|p| {
        p.session != current && (same_path(&p.path, &watchdog) || same_path(&p.path, &service))
    }) {
        return Err("其他 Windows 会话正在使用此输入法，请先退出该会话的输入法".into());
    }
    // Recheck through the opened handle before termination to avoid PID reuse.
    for target in [&watchdog, &service] {
        for p in processes()? {
            if !same_path(&p.path, target) {
                continue;
            }
            if p.session != current {
                return Err("其他 Windows 会话正在使用此输入法，请先退出该会话的输入法".into());
            }
            let h = unsafe { OpenProcess(0x1000 | 0x100000 | 1, 0, p.pid) };
            if h.is_null() {
                return Err("无法停止输入法，未写入皮肤".into());
            }
            let h = OwnedHandle(h);
            if !same_path(&path_of(h.0)?, target) {
                return Err("进程身份已变化，请重试".into());
            }
            if unsafe { TerminateProcess(h.0, 0) } == 0
                || unsafe { WaitForSingleObject(h.0, 5000) } != 0
            {
                return Err("输入法尚未退出，未写入皮肤".into());
            }
        }
    }
    if processes()?
        .iter()
        .any(|p| same_path(&p.path, &service) || same_path(&p.path, &watchdog))
    {
        return Err("输入法被再次启动，未写入皮肤".into());
    }
    Ok(())
}

/// Must be called by the unelevated UI process, including after helper failure.
pub fn restart(root: &Path, version: &str) -> Result<(), String> {
    if crate::elevate::is_admin() {
        return Err("请用普通权限打开助手以恢复输入法".into());
    }
    let watchdog = root.join("bootstrap/ImeWatchdog.exe");
    let service = root.join("versions").join(version).join("ImeService.exe");
    let _guards = crate::safe_fs::guard_path(&watchdog)?;
    let current = session()?;
    let deadline = Instant::now() + Duration::from_secs(8);
    wait_for_restart(
        &watchdog,
        &service,
        current,
        processes,
        || {
            std::process::Command::new(&watchdog)
                .creation_flags(0x0800_0000 | 0x8)
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("启动输入法失败：{e}"))
        },
        || {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(150));
            Instant::now() < deadline
        },
    )
}

fn wait_for_restart(
    watchdog: &Path,
    service: &Path,
    current: u32,
    mut probe: impl FnMut() -> Result<Vec<ImeProcess>, String>,
    mut launch: impl FnMut() -> Result<(), String>,
    mut pause: impl FnMut() -> bool,
) -> Result<(), String> {
    let mut launched = false;
    let mut previous_pid = None;
    loop {
        let error = match probe() {
            Ok(list) => {
                // Launch only after a complete, verified snapshot confirms the
                // watchdog is absent. A failed probe must not start duplicates.
                if !launched
                    && !list
                        .iter()
                        .any(|p| p.session == current && same_path(&p.path, watchdog))
                {
                    launch()?;
                    launched = true;
                }
                let pid = list
                    .iter()
                    .find(|p| p.session == current && same_path(&p.path, service))
                    .map(|p| p.pid);
                // Startup can replace an initial service process. Require two
                // successive verified snapshots of the same service instance.
                if pid.is_some() && pid == previous_pid {
                    return Ok(());
                }
                previous_pid = pid;
                None
            }
            Err(error) => {
                previous_pid = None;
                Some(error)
            }
        };
        if !pause() {
            return Err(match error {
                Some(error) => format!("皮肤操作已结束，但输入法恢复超时：{error}"),
                None => "皮肤操作已结束，但输入法未恢复；请手动打开豆包输入法".into(),
            });
        }
    }
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
    fn Process32FirstW(h: Handle, entry: *mut ProcessEntry) -> i32;
    fn Process32NextW(h: Handle, entry: *mut ProcessEntry) -> i32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    fn CloseHandle(h: Handle) -> i32;
    fn ProcessIdToSessionId(pid: u32, session: *mut u32) -> i32;
    fn QueryFullProcessImageNameW(h: Handle, flags: u32, name: *mut u16, size: *mut u32) -> i32;
    fn GetLastError() -> u32;
    fn TerminateProcess(h: Handle, code: u32) -> i32;
    fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn process(pid: u32, path: &str) -> ImeProcess {
        ImeProcess {
            pid,
            session: 1,
            path: PathBuf::from(path),
        }
    }

    #[test]
    fn temporary_access_denial_retries_but_persistent_denial_stays_an_error() {
        let mut attempts = 0;
        let list = retry_probe(
            || {
                attempts += 1;
                if attempts < 3 {
                    Err(ProbeError::Denied("path denied".into()))
                } else {
                    Ok(vec![process(10, "service")])
                }
            },
            || {},
        )
        .unwrap();
        assert_eq!(list[0].pid, 10);
        assert_eq!(attempts, 3);
        attempts = 0;
        let error = retry_probe(
            || {
                attempts += 1;
                Err(ProbeError::Denied("path denied".into()))
            },
            || {},
        )
        .unwrap_err();
        assert_eq!(error, "path denied");
        assert_eq!(attempts, PROCESS_PROBE_ATTEMPTS);
    }

    #[test]
    fn restart_survives_probe_failures_and_waits_for_a_stable_service() {
        let watchdog = process(10, "watchdog");
        let service = process(20, "service");
        let replacement = process(21, "service");
        let mut snapshots = VecDeque::from([
            Err("path denied".into()),
            Ok(vec![]),
            Ok(vec![watchdog.clone(), service]),
            Err("process changed".into()),
            Ok(vec![watchdog.clone(), replacement.clone()]),
            Ok(vec![watchdog, replacement]),
        ]);
        let mut launches = 0;
        let mut waits = 0;
        wait_for_restart(
            Path::new("watchdog"),
            Path::new("service"),
            1,
            || snapshots.pop_front().expect("restart should finish"),
            || {
                launches += 1;
                Ok(())
            },
            || {
                waits += 1;
                waits < 8
            },
        )
        .unwrap();
        assert!(snapshots.is_empty());
        assert_eq!(launches, 1);
        assert_eq!(waits, 5);
    }

    #[test]
    fn restart_never_launches_when_process_identity_cannot_be_verified() {
        let mut waits = 0;
        let error = wait_for_restart(
            Path::new("watchdog"),
            Path::new("service"),
            1,
            || Err("path denied".into()),
            || panic!("unknown paths must not launch a duplicate watchdog"),
            || {
                waits += 1;
                waits < 4
            },
        )
        .unwrap_err();
        assert!(error.contains("恢复超时") && error.contains("path denied"));
        assert_eq!(waits, 4);
    }

    #[test]
    fn restart_rejects_other_versions_and_sessions_and_does_not_spawn_repeatedly() {
        let mut foreign_session = process(30, "service");
        foreign_session.session = 2;
        let other_version = process(31, "another-version/service");
        let mut launches = 0;
        let mut waits = 0;
        let result = wait_for_restart(
            Path::new("watchdog"),
            Path::new("service"),
            1,
            || Ok(vec![foreign_session.clone(), other_version.clone()]),
            || {
                launches += 1;
                Ok(())
            },
            || {
                waits += 1;
                waits < 4
            },
        );
        assert!(result.is_err());
        assert_eq!(launches, 1);
        assert_eq!(waits, 4);
    }

    #[test]
    fn a_failed_probe_or_changed_pid_breaks_the_stability_check() {
        let watchdog = process(10, "watchdog");
        let mut snapshots = VecDeque::from([
            Ok(vec![watchdog.clone(), process(20, "service")]),
            Ok(vec![watchdog.clone(), process(21, "service")]),
            Err("process changed".into()),
            Ok(vec![watchdog.clone(), process(21, "service")]),
            Ok(vec![watchdog, process(21, "service")]),
        ]);
        wait_for_restart(
            Path::new("watchdog"),
            Path::new("service"),
            1,
            || snapshots.pop_front().expect("restart should finish"),
            || panic!("watchdog already running"),
            || true,
        )
        .unwrap();
        assert!(snapshots.is_empty());
    }
}
