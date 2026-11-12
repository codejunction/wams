//! Process sampler: CPU, memory, GPU and disk use per app. Runs on its own thread and only works when asked.

use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;

#[derive(Clone, Copy, Default)]
pub struct Use {
    /// Percent of all cores.
    pub cpu: f32,
    pub mem: u64,
    /// Percent of the busiest GPU engine.
    pub gpu: f32,
    /// Bytes read + written per second.
    pub disk: u64,
}

pub struct Proc {
    pub pid: u32,
    pub name: String,
    pub exe: PathBuf,
    pub used: Use,
}

/// One app: a root process, everything it started, and other running copies of the same app.
pub struct Group {
    pub name: String,
    pub system: bool,
    pub used: Use,
    pub procs: Vec<Proc>,
}

#[derive(Default)]
pub struct Snapshot {
    pub groups: Vec<Group>,
    pub total: Use,
    pub mem_total: u64,
    /// False where per-process GPU use is not available (macOS).
    pub has_gpu: bool,
    /// Result of the last kill request, if it failed.
    pub error: Option<String>,
}

pub enum Cmd {
    Sample,
    Kill(Vec<u32>),
}

/// Starts the sampler thread. It blocks until a `Cmd` arrives, so a hidden task view costs nothing.
pub fn spawn(ctx: egui::Context) -> (mpsc::Sender<Cmd>, mpsc::Receiver<Snapshot>) {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (snap_tx, snap_rx) = mpsc::channel();
    std::thread::spawn(move || {
        // on failure the channel closes and the view reports it
        let Some(os) = os::State::new() else { return };
        let mut s = Sampler { os, at: Instant::now(), names: HashMap::new() };
        for cmd in cmd_rx {
            let error = match cmd {
                Cmd::Kill(pids) => {
                    let failed = pids.iter().filter(|&&p| !s.os.kill(p)).count();
                    let how = if cfg!(windows) { "as administrator" } else { "with sudo" };
                    (failed > 0).then(|| format!("Could not end {failed} of {} processes: access denied. Run WAMS {how} to end them.", pids.len()))
                }
                Cmd::Sample => None,
            };
            if snap_tx.send(s.sample(error)).is_err() {
                break;
            }
            ctx.request_repaint();
        }
    });
    (cmd_tx, snap_rx)
}

/// One process as the OS reports it, rates already worked out.
struct Raw {
    pid: u32,
    parent: Option<u32>,
    /// Creation time in any unit, only compared between processes.
    start: u64,
    /// Image name, "chrome.exe".
    name: String,
    /// Empty when the process is protected.
    exe: PathBuf,
    /// Part of the OS, not something the user started.
    system: bool,
    /// Chromium/Electron "--type=" value ("renderer", "gpu-process"): tells sub-tasks of one app apart.
    hint: Option<String>,
    used: Use,
}

struct Sample {
    procs: Vec<Raw>,
    /// CPU, memory and GPU of the whole machine. Disk is summed from the processes.
    total: Use,
    mem_total: u64,
    has_gpu: bool,
}

struct Sampler {
    os: os::State,
    at: Instant,
    /// Executable path -> human name. Grows by one entry per distinct program seen.
    names: HashMap<PathBuf, String>,
}

impl Sampler {
    fn sample(&mut self, error: Option<String>) -> Snapshot {
        let dt = self.at.elapsed().as_secs_f64().max(0.001);
        self.at = Instant::now();
        let Sample { procs, mut total, mem_total, has_gpu } = self.os.sample(dt);
        let links: Vec<Link> = procs.iter().map(|p| Link { pid: p.pid, parent: p.parent, start: p.start, boundary: boundary(p) }).collect();
        let mut groups: HashMap<String, Group> = HashMap::new();
        for (p, root) in procs.iter().zip(roots(&links)) {
            let r = &procs[root];
            let name = if is_service_host(r) { "Service Host".to_string() } else { title(&mut self.names, r) };
            let mut sub = match self.os.service(p.pid).filter(|_| is_service_host(p)) {
                Some(s) => format!("Service Host: {s}"),
                None => title(&mut self.names, p),
            };
            if let Some(t) = &p.hint {
                sub = format!("{sub} ({t})");
            }
            let g = groups.entry(name).or_insert_with_key(|k| Group { name: k.clone(), system: r.system, used: Use::default(), procs: vec![] });
            g.used.cpu += p.used.cpu;
            g.used.mem += p.used.mem;
            g.used.gpu = (g.used.gpu + p.used.gpu).min(100.0);
            g.used.disk += p.used.disk;
            total.disk += p.used.disk;
            g.procs.push(Proc { pid: p.pid, name: sub, exe: p.exe.clone(), used: p.used });
        }
        Snapshot { groups: groups.into_values().collect(), total, mem_total, has_gpu, error }
    }
}

struct Link {
    pid: u32,
    parent: Option<u32>,
    start: u64,
    /// Shells and service launchers: their children are separate apps.
    boundary: bool,
}

/// For each process, the index of its app root: the oldest ancestor below a boundary process.
fn roots(ps: &[Link]) -> Vec<usize> {
    let by_pid: HashMap<u32, usize> = ps.iter().enumerate().map(|(i, p)| (p.pid, i)).collect();
    (0..ps.len())
        .map(|mut i| {
            // depth limit: guards against parent cycles from reused pids
            for _ in 0..32 {
                let Some(&up) = ps[i].parent.and_then(|pp| by_pid.get(&pp)) else { break };
                // a "parent" that started after its child is a reused pid, not a parent
                if up == i || ps[up].boundary || ps[up].start > ps[i].start {
                    break;
                }
                i = up;
            }
            i
        })
        .collect()
}

fn boundary(p: &Raw) -> bool {
    // Linux names are cut to 15 characters by the kernel ("gnome-session-b")
    const SHELLS: [&str; 18] = [
        "explorer.exe", "services.exe", "svchost.exe", "wininit.exe", "winlogon.exe", "smss.exe", "csrss.exe", "userinit.exe", "sihost.exe", "runtimebroker.exe", "launchd",
        "systemd", "init", "gnome-shell", "gnome-session-b", "plasmashell", "xfce4-session", "cinnamon",
    ];
    // Windows: 0 idle, 4 System. Unix: 0 kernel, 1 init. Linux pid 2 stays a normal parent, so its kernel threads form one group.
    p.pid <= if cfg!(windows) { 4 } else { 1 } || SHELLS.contains(&p.name.to_lowercase().as_str())
}

fn is_service_host(p: &Raw) -> bool {
    p.name.eq_ignore_ascii_case("svchost.exe")
}

/// Human name: the program's own description, else its file name.
fn title(names: &mut HashMap<PathBuf, String>, p: &Raw) -> String {
    let fallback = || p.name.strip_suffix(".exe").unwrap_or(&p.name).to_string();
    if p.exe.as_os_str().is_empty() {
        return fallback();
    }
    if let Some(n) = names.get(&p.exe) {
        return n.clone();
    }
    let n = os::describe(&p.exe).filter(|s| !s.trim().is_empty()).unwrap_or_else(fallback);
    names.insert(p.exe.clone(), n.clone());
    n
}

/// Windows: one NtQuerySystemInformation call lists every process with its counters, protected ones
/// included, without opening a handle per process. This is what Task Manager reads.
#[cfg(windows)]
mod os {
    use super::{Raw, Sample, Use};
    use crate::scan::wide;
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::path::{Path, PathBuf};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, FILETIME, GetLastError, HANDLE};
    use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    use windows_sys::Win32::System::Performance::*;
    use windows_sys::Win32::System::Services::*;
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, QueryFullProcessImageNameW, TerminateProcess,
    };

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtQuerySystemInformation(class: u32, info: *mut c_void, len: u32, needed: *mut u32) -> i32;
        fn NtQueryInformationProcess(process: HANDLE, class: u32, info: *mut c_void, len: u32, needed: *mut u32) -> i32;
    }
    const SYSTEM_PROCESS_INFORMATION: u32 = 5;
    const PROCESS_COMMAND_LINE_INFORMATION: u32 = 60;

    #[repr(C)]
    struct UnicodeString {
        /// Bytes, not characters.
        len: u16,
        max: u16,
        buf: *const u16,
    }

    /// SYSTEM_PROCESS_INFORMATION. The layout is unchanged since Windows Vista.
    #[repr(C)]
    struct ProcessInfo {
        next: u32,
        threads: u32,
        /// Private working set: the "Memory" column of Task Manager.
        private_working_set: i64,
        hard_faults: u32,
        threads_high: u32,
        cycle_time: u64,
        create_time: i64,
        /// CPU times in 100 ns units.
        user_time: i64,
        kernel_time: i64,
        image_name: UnicodeString,
        base_priority: i32,
        pid: usize,
        parent: usize,
        handles: u32,
        session: u32,
        key: usize,
        vm: [usize; 2],
        page_faults: u32,
        working_set: [usize; 2],
        quota: [usize; 4],
        pagefile: [usize; 2],
        private_pages: usize,
        ops: [i64; 3],
        read_bytes: i64,
        write_bytes: i64,
        other_bytes: i64,
    }

    unsafe fn wstr(p: *const u16) -> String {
        if p.is_null() {
            return String::new();
        }
        unsafe {
            let len = (0..).take_while(|&i| *p.add(i) != 0).count();
            String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
        }
    }

    unsafe fn ustr(s: &UnicodeString) -> String {
        if s.buf.is_null() {
            return String::new();
        }
        unsafe { String::from_utf16_lossy(std::slice::from_raw_parts(s.buf, s.len as usize / 2)) }
    }

    /// FileDescription from the executable's version resource, the name Task Manager shows.
    pub fn describe(exe: &Path) -> Option<String> {
        let file = wide(&exe.to_string_lossy());
        unsafe {
            let size = GetFileVersionInfoSizeW(file.as_ptr(), null_mut());
            if size == 0 {
                return None;
            }
            let mut data = vec![0u8; size as usize];
            if GetFileVersionInfoW(file.as_ptr(), 0, size, data.as_mut_ptr().cast()) == 0 {
                return None;
            }
            let (mut p, mut len) = (null_mut(), 0u32);
            let query = |key: &str, p: &mut *mut c_void, len: &mut u32| VerQueryValueW(data.as_ptr().cast(), wide(key).as_ptr(), p, len) != 0 && !p.is_null();
            // language + code page of the first translation, else US English Unicode
            let (lang, page) = if query(r"\VarFileInfo\Translation", &mut p, &mut len) && len >= 4 {
                let t = p.cast::<u16>();
                (t.read_unaligned(), t.add(1).read_unaligned())
            } else {
                (0x0409, 0x04B0)
            };
            if !query(&format!(r"\StringFileInfo\{lang:04x}{page:04x}\FileDescription"), &mut p, &mut len) {
                return None;
            }
            Some(wstr(p.cast()))
        }
    }

    /// Executable path and "--type=" hint. Opens the process once, when it is first seen.
    fn identify(pid: u32) -> (PathBuf, Option<String>) {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return Default::default();
            }
            let mut path = [0u16; 1024];
            let mut n = path.len() as u32;
            let exe = if QueryFullProcessImageNameW(h, 0, path.as_mut_ptr(), &mut n) != 0 { String::from_utf16_lossy(&path[..n as usize]).into() } else { PathBuf::new() };
            // ponytail: command lines over 8 KB get no hint
            let mut line = [0u64; 1024];
            let mut hint = None;
            if NtQueryInformationProcess(h, PROCESS_COMMAND_LINE_INFORMATION, line.as_mut_ptr().cast(), 8192, &mut n) >= 0 {
                let line = ustr(&*line.as_ptr().cast::<UnicodeString>());
                hint = line.split_whitespace().find_map(|a| a.strip_prefix("--type=")).map(str::to_string);
            }
            CloseHandle(h);
            (exe, hint)
        }
    }

    /// What the last sample knew about a pid.
    struct Seen {
        start: i64,
        cpu: i64,
        io: i64,
        exe: PathBuf,
        hint: Option<String>,
    }

    pub struct State {
        /// PDH query + "GPU Engine" counter; None when the counter set does not exist.
        gpu: Option<(PDH_HQUERY, PDH_HCOUNTER)>,
        services: HashMap<u32, String>,
        seen: HashMap<u32, Seen>,
        /// CPU time of the idle process at the last sample.
        idle: i64,
        /// Process list buffer, kept between samples. u64: aligned for the entries.
        buf: Vec<u64>,
        tick: u32,
        /// Lower-case Windows folder: what runs from there is part of the OS.
        windir: String,
    }

    impl State {
        pub fn new() -> Option<Self> {
            let gpu = unsafe {
                let (mut q, mut c) = (null_mut(), null_mut());
                if PdhOpenQueryW(null(), 0, &mut q) != 0 {
                    None
                } else if PdhAddEnglishCounterW(q, wide(r"\GPU Engine(*)\Utilization Percentage").as_ptr(), 0, &mut c) != 0 {
                    PdhCloseQuery(q);
                    None
                } else {
                    PdhCollectQueryData(q); // rates need two samples; this is the first
                    Some((q, c))
                }
            };
            let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into()).to_lowercase();
            Some(State { gpu, services: HashMap::new(), seen: HashMap::new(), idle: 0, buf: vec![0; 1 << 16], tick: 0, windir })
        }

        pub fn sample(&mut self, dt: f64) -> Sample {
            if self.tick % 8 == 0 {
                self.refresh_services();
            }
            self.tick += 1;
            let (gpu, gpu_total) = self.gpu();
            let mut mem = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
            unsafe { GlobalMemoryStatusEx(&mut mem) };
            let mut out = Sample {
                procs: vec![],
                total: Use { cpu: 0.0, mem: mem.ullTotalPhys - mem.ullAvailPhys, gpu: gpu_total, disk: 0 },
                mem_total: mem.ullTotalPhys,
                has_gpu: self.gpu.is_some(),
            };
            // the list grows between the size query and the call: retry with room to spare
            let mut need = 0u32;
            while unsafe { NtQuerySystemInformation(SYSTEM_PROCESS_INFORMATION, self.buf.as_mut_ptr().cast(), (self.buf.len() * 8) as u32, &mut need) } < 0 {
                if need as usize <= self.buf.len() * 8 {
                    return out; // failed for a reason other than size
                }
                self.buf = vec![0; need as usize / 8 + (1 << 13)];
            }
            // 100 ns units of CPU time available to all cores in this interval
            let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
            let budget = dt * 1e7 * cores;
            let mut seen = HashMap::with_capacity(self.seen.len() + 16);
            let mut at = self.buf.as_ptr().cast::<u8>();
            loop {
                let e = unsafe { &*at.cast::<ProcessInfo>() };
                let (pid, cpu, io) = (e.pid as u32, e.user_time + e.kernel_time, e.read_bytes + e.write_bytes);
                if pid == 0 {
                    // the idle process: everything it did not get is work
                    if self.idle != 0 {
                        out.total.cpu = (100.0 - (cpu - self.idle) as f64 / budget * 100.0).clamp(0.0, 100.0) as f32;
                    }
                    self.idle = cpu;
                } else {
                    // same pid with another creation time is a new process
                    let old = self.seen.remove(&pid).filter(|s| s.start == e.create_time);
                    let (d_cpu, d_io) = old.as_ref().map_or((0, 0), |s| (cpu - s.cpu, io - s.io));
                    let (exe, hint) = old.map_or_else(|| identify(pid), |s| (s.exe, s.hint));
                    let name = unsafe { ustr(&e.image_name) };
                    out.procs.push(Raw {
                        pid,
                        parent: Some(e.parent as u32),
                        start: e.create_time as u64,
                        name: if name.is_empty() { "System".into() } else { name },
                        // protected processes hide their path; all of them belong to the OS
                        system: exe.as_os_str().is_empty() || exe.to_string_lossy().to_lowercase().starts_with(&self.windir),
                        exe: exe.clone(),
                        hint: hint.clone(),
                        used: Use {
                            cpu: (d_cpu.max(0) as f64 / budget * 100.0) as f32,
                            mem: e.private_working_set.max(0) as u64,
                            gpu: gpu.get(&pid).copied().unwrap_or(0.0),
                            // ponytail: file, pipe and socket bytes together, as the kernel counts them; ETW disk events if exact disk bytes matter
                            disk: (d_io.max(0) as f64 / dt) as u64,
                        },
                    });
                    seen.insert(pid, Seen { start: e.create_time, cpu, io, exe, hint });
                }
                if e.next == 0 {
                    break;
                }
                at = unsafe { at.add(e.next as usize) };
            }
            self.seen = seen;
            out
        }

        /// True when the process is gone afterwards.
        pub fn kill(&self, pid: u32) -> bool {
            let Some(seen) = self.seen.get(&pid) else { return true };
            unsafe {
                let h = OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                if h.is_null() {
                    return GetLastError() == ERROR_INVALID_PARAMETER; // no such pid: it already ended
                }
                // the pid may belong to a new process since the last sample: never end that one
                let mut t = [FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 }; 4];
                let [created, exited, kernel, user] = &mut t;
                let same = GetProcessTimes(h, created, exited, kernel, user) != 0 && (((created.dwHighDateTime as i64) << 32) | created.dwLowDateTime as i64) == seen.start;
                let ok = !same || TerminateProcess(h, 1) != 0;
                CloseHandle(h);
                ok
            }
        }

        /// (GPU percent per pid, total GPU percent).
        fn gpu(&self) -> (HashMap<u32, f32>, f32) {
            let Some((q, c)) = self.gpu else { return Default::default() };
            let mut values = vec![];
            unsafe {
                PdhCollectQueryData(q);
                let (mut size, mut n) = (0u32, 0u32);
                PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, &mut size, &mut n, null_mut());
                let mut buf = vec![0u64; size as usize / 8 + 1]; // u64: aligned for the item structs
                if size > 0 && PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, &mut size, &mut n, buf.as_mut_ptr().cast()) == 0 {
                    for it in std::slice::from_raw_parts(buf.as_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>(), n as usize) {
                        // status 0 = valid, 1 = new data; anything else is an engine with no second sample yet
                        if it.FmtValue.CStatus <= 1 {
                            values.push((wstr(it.szName), it.FmtValue.Anonymous.doubleValue as f32));
                        }
                    }
                }
            }
            fold_gpu(&values)
        }

        fn refresh_services(&mut self) {
            unsafe {
                let scm = OpenSCManagerW(null(), null(), SC_MANAGER_ENUMERATE_SERVICE);
                if scm.is_null() {
                    return;
                }
                let (mut need, mut n, mut resume) = (0u32, 0u32, 0u32);
                EnumServicesStatusExW(scm, SC_ENUM_PROCESS_INFO, SERVICE_WIN32, SERVICE_ACTIVE, null_mut(), 0, &mut need, &mut n, &mut resume, null());
                let mut buf = vec![0u64; need as usize / 8 + 1];
                resume = 0;
                let ok = EnumServicesStatusExW(scm, SC_ENUM_PROCESS_INFO, SERVICE_WIN32, SERVICE_ACTIVE, buf.as_mut_ptr().cast(), (buf.len() * 8) as u32, &mut need, &mut n, &mut resume, null());
                CloseServiceHandle(scm);
                if ok == 0 {
                    return; // service list grew between the two calls; keep the old map until the next refresh
                }
                self.services.clear();
                for s in std::slice::from_raw_parts(buf.as_ptr().cast::<ENUM_SERVICE_STATUS_PROCESSW>(), n as usize) {
                    let names = self.services.entry(s.ServiceStatusProcess.dwProcessId).or_default();
                    if !names.is_empty() {
                        names.push_str(", ");
                    }
                    names.push_str(&wstr(s.lpDisplayName));
                }
            }
        }

        /// Display names of the services hosted by this pid.
        pub fn service(&self, pid: u32) -> Option<&str> {
            self.services.get(&pid).map(String::as_str)
        }
    }

    /// Counter instances are named "pid_<pid>_luid_..._engtype_<type>", one per process per GPU engine.
    /// A process uses as much GPU as its busiest engine; the GPU is as busy as its busiest engine overall.
    fn fold_gpu(values: &[(String, f32)]) -> (HashMap<u32, f32>, f32) {
        let mut by_pid: HashMap<u32, f32> = HashMap::new();
        let mut by_engine: HashMap<&str, f32> = HashMap::new();
        for (name, v) in values {
            let Some((pid, engine)) = name.strip_prefix("pid_").and_then(|r| r.split_once('_')) else { continue };
            let Ok(pid) = pid.parse() else { continue };
            let e = by_pid.entry(pid).or_default();
            *e = e.max(*v);
            *by_engine.entry(engine).or_default() += v;
        }
        (by_pid, by_engine.values().fold(0.0f32, |a, b| a.max(*b)).min(100.0))
    }

    #[test]
    fn gpu_takes_busiest_engine() {
        let v = |s: &str, x| (s.to_string(), x);
        let (by_pid, total) = fold_gpu(&[
            v("pid_10_luid_0x0_0x1_phys_0_eng_0_engtype_3D", 30.0),
            v("pid_10_luid_0x0_0x1_phys_0_eng_1_engtype_VideoDecode", 5.0),
            v("pid_20_luid_0x0_0x1_phys_0_eng_0_engtype_3D", 25.0),
            v("garbage", 99.0),
        ]);
        assert_eq!(by_pid[&10], 30.0);
        assert_eq!(by_pid[&20], 25.0);
        assert_eq!(total, 55.0);
    }

    /// Live check against this very process: the hand-written struct layout must match the kernel's.
    #[test]
    fn sample_finds_this_process() {
        let mut s = State::new().unwrap();
        s.sample(1.0);
        std::hint::black_box((0..20_000_000u64).sum::<u64>());
        let out = s.sample(1.0);
        let me = out.procs.iter().find(|p| p.pid == std::process::id()).expect("own pid listed");
        assert_eq!(me.exe, std::env::current_exe().unwrap());
        assert!(me.name.to_lowercase().starts_with("wams"), "{}", me.name);
        assert!(me.used.mem > 100 << 10 && me.used.mem < 4 << 30, "{}", me.used.mem);
        assert!(out.mem_total > out.total.mem && out.total.mem > 0);
        assert!(out.procs.iter().any(|p| p.pid == 4 && p.exe.as_os_str().is_empty()), "System process listed");
        assert!(out.procs.len() > 20);
    }

    #[test]
    fn kill_ends_only_a_sampled_process() {
        use std::process::{Command, Stdio};
        let mut child = Command::new("cmd").args(["/c", "pause"]).stdin(Stdio::piped()).stdout(Stdio::null()).spawn().unwrap();
        let mut s = State::new().unwrap();
        assert!(s.kill(child.id())); // never sampled: treated as gone, left alone
        assert!(child.try_wait().unwrap().is_none());
        s.sample(1.0);
        assert!(s.kill(child.id()));
        assert_eq!(child.wait().unwrap().code(), Some(1));
    }
}

/// macOS and Linux: sysinfo reads the process table.
/// No per-process GPU counters that cover every vendor, and no service hosts.
#[cfg(not(windows))]
mod os {
    use super::{Raw, Sample, Use};
    use std::path::{Path, PathBuf};
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

    /// macOS: the name of the .app bundle the program is in.
    #[cfg(target_os = "macos")]
    pub fn describe(exe: &Path) -> Option<String> {
        let app = exe.ancestors().find(|a| a.extension().is_some_and(|e| e == "app"))?;
        Some(app.file_stem()?.to_string_lossy().into())
    }

    /// Linux: the name the app menu shows for this program, else the full file name
    /// (the kernel's own process name stops at 15 characters).
    #[cfg(not(target_os = "macos"))]
    pub fn describe(exe: &Path) -> Option<String> {
        static NAMES: std::sync::OnceLock<std::collections::HashMap<String, String>> = std::sync::OnceLock::new();
        let file = exe.file_name()?.to_string_lossy();
        Some(NAMES.get_or_init(desktop_names).get(file.as_ref()).cloned().unwrap_or_else(|| file.into()))
    }

    /// Program file name -> app name, from every installed .desktop file. Read once.
    // ponytail: matched on the file name of Exec; an app started through a wrapper script (google-chrome-stable -> chrome) keeps its file name
    #[cfg(not(target_os = "macos"))]
    fn desktop_names() -> std::collections::HashMap<String, String> {
        let data = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
        let user = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share");
        let mut names = std::collections::HashMap::new();
        for dir in std::iter::once(user).chain(data.split(':').map(PathBuf::from)) {
            for e in std::fs::read_dir(dir.join("applications")).into_iter().flatten().flatten() {
                if let Some((file, name)) = std::fs::read_to_string(e.path()).ok().as_deref().and_then(desktop_entry) {
                    names.entry(file).or_insert(name);
                }
            }
        }
        names
    }

    /// (program file name, app name) of one .desktop file.
    #[cfg(not(target_os = "macos"))]
    fn desktop_entry(text: &str) -> Option<(String, String)> {
        let get = |key: &str| text.lines().find_map(|l| l.strip_prefix(key));
        let program = get("Exec=")?.split_whitespace().next()?.trim_matches('"');
        let file = Path::new(program).file_name()?.to_str()?;
        // launchers start many different apps: their own name says nothing
        if ["env", "sh", "bash", "flatpak", "snap", "python3"].contains(&file) {
            return None;
        }
        Some((file.to_string(), get("Name=")?.to_string()))
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn desktop_entry_gives_program_and_name() {
        let text = "[Desktop Entry]\nName=Visual Studio Code\nName[de]=VS Code\nExec=/usr/share/code/code --unity-launch %F\n";
        assert_eq!(desktop_entry(text), Some(("code".into(), "Visual Studio Code".into())));
        assert_eq!(desktop_entry("Name=X\nExec=flatpak run org.x.X\n"), None);
    }

    /// Live check: this test process is in the sample, is not a system process, and has a size.
    #[test]
    fn sample_finds_this_process() {
        let mut s = State::new().unwrap();
        s.sample(1.0);
        let out = s.sample(1.0);
        let me = out.procs.iter().find(|p| p.pid == std::process::id()).expect("own pid listed");
        assert_eq!(me.exe, std::env::current_exe().unwrap());
        assert!(!me.system && me.used.mem > 100 << 10, "{} {}", me.system, me.used.mem);
        assert!(out.mem_total > out.total.mem && out.total.mem > 0);
        assert!(out.procs.iter().any(|p| p.pid == 1));
        // one row per process, not per thread
        let mut pids: Vec<u32> = out.procs.iter().map(|p| p.pid).collect();
        pids.dedup();
        assert_eq!(pids.len(), out.procs.len());
        assert_eq!(out.procs.iter().filter(|p| p.pid == std::process::id()).count(), 1);
    }

    #[test]
    fn kill_ends_a_child() {
        let mut child = std::process::Command::new("sleep").arg("60").spawn().unwrap();
        let mut s = State::new().unwrap();
        s.sample(1.0);
        assert!(s.kill(child.id()));
        assert!(!child.wait().unwrap().success());
    }

    pub struct State(System);

    impl State {
        pub fn new() -> Option<Self> {
            System::new().ok().map(State)
        }

        pub fn sample(&mut self, dt: f64) -> Sample {
            let sys = &mut self.0;
            // without_tasks: on Linux a thread is a task; one row per process is wanted
            let kind = ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_disk_usage()
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_cmd(UpdateKind::OnlyIfNotSet)
                .with_user(UpdateKind::OnlyIfNotSet)
                .without_tasks();
            sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
            let me = unsafe { libc::getuid() };
            sys.refresh_cpu_usage();
            sys.refresh_memory();
            let cores = sys.cpus().len().max(1) as f32;
            let procs = sys
                .processes()
                .values()
                .map(|p| {
                    let io = p.disk_usage();
                    let exe = p.exe().map(PathBuf::from).unwrap_or_default();
                    // macOS keeps the OS in fixed folders. Linux keeps everything in /usr, so there the owner decides:
                    // what another user (root, a service account) runs is the system's.
                    let system = if cfg!(target_os = "macos") {
                        ["/System", "/usr", "/sbin", "/bin"].iter().any(|d| exe.starts_with(d))
                    } else {
                        p.user_id().is_none_or(|u| **u != me)
                    };
                    Raw {
                        pid: p.pid().as_u32(),
                        parent: p.parent().map(|x| x.as_u32()),
                        start: p.start_time(),
                        name: p.name().to_string_lossy().into(),
                        // kernel threads and other users' processes hide their path
                        system: system || exe.as_os_str().is_empty(),
                        exe,
                        hint: p.cmd().iter().find_map(|a| a.to_str()?.strip_prefix("--type=")).map(str::to_string),
                        used: Use { cpu: p.cpu_usage() / cores, mem: p.memory(), gpu: 0.0, disk: ((io.read_bytes + io.written_bytes) as f64 / dt) as u64 },
                    }
                })
                .collect();
            Sample { procs, total: Use { cpu: sys.global_cpu_usage(), mem: sys.used_memory(), gpu: 0.0, disk: 0 }, mem_total: sys.total_memory(), has_gpu: false }
        }

        /// True when the process is gone afterwards.
        pub fn kill(&self, pid: u32) -> bool {
            self.0.process(Pid::from_u32(pid)).is_none_or(|p| p.kill())
        }

        pub fn service(&self, _: u32) -> Option<&str> {
            None
        }
    }
}

#[test]
fn roots_stop_at_shell_and_reused_pids() {
    let l = |pid, parent, start, boundary| Link { pid, parent, start, boundary };
    let ps = [
        l(100, None, 1, true),        // 0 shell
        l(200, Some(100), 10, false), // 1 app started by the shell
        l(300, Some(200), 20, false), // 2 helper of the app
        l(400, Some(300), 30, false), // 3 helper of the helper
        l(500, Some(600), 5, false),  // 4 "parent" 600 is younger: reused pid
        l(600, Some(100), 50, false), // 5
    ];
    assert_eq!(roots(&ps), [0, 1, 1, 1, 4, 5]);
}
