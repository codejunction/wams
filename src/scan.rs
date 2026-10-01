//! Disk scanning, drive info and cloud sync detection. No UI code here.

use rayon::prelude::*;
use std::collections::HashMap;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

/// Online-only cloud placeholder (Files On-Demand): counted in size, takes no space on this PC.
const CLOUD_ONLY: u32 = 0x0040_0000 | 0x0000_1000; // RECALL_ON_DATA_ACCESS | OFFLINE
const BIG_FILE: u64 = 64 << 20;

/// Only directories are kept in memory; files are listed on demand when a folder is opened.
pub struct Dir {
    pub name: String,
    pub size: u64,
    /// Bytes actually stored on this PC (size minus online-only cloud files).
    pub local: u64,
    pub files: u64,
    pub dirs: Vec<Dir>,
}

pub struct Drive {
    pub root: String,
    pub label: String,
    pub kind: Kind,
    pub total: u64,
    pub free: u64,
}

impl Drive {
    /// Only local disks are scanned; network and virtual cloud drives are listed but not walked.
    pub fn scanned(&self) -> bool {
        matches!(self.kind, Kind::System | Kind::Local | Kind::Usb)
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    System,
    Local,
    Usb,
    Network,
    Cloud,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::System => "System",
            Kind::Local => "Local",
            Kind::Usb => "USB",
            Kind::Network => "Network",
            Kind::Cloud => "Cloud",
        }
    }
}

pub struct SyncRoot {
    pub provider: String,
    pub path: PathBuf,
    key: String,
}

pub struct Scan {
    pub root: Dir,
    pub drives: Vec<Drive>,
    pub big: Vec<(u64, PathBuf)>,
    /// (extension, bytes, count), biggest first.
    pub exts: Vec<(String, u64, u64)>,
    pub sync: Vec<SyncRoot>,
    pub known: Vec<(&'static str, PathBuf)>,
}

impl Scan {
    /// The drive when `path` is a drive root like "C:\\".
    pub fn drive(&self, path: &Path) -> Option<&Drive> {
        self.drives.iter().find(|d| path.as_os_str().eq_ignore_ascii_case(&d.root))
    }

    pub fn find(&self, path: &Path) -> Option<&Dir> {
        find(&self.root, &names_of(path))
    }

    /// The sync root this path is inside of, if any.
    pub fn synced(&self, path: &Path) -> Option<&SyncRoot> {
        let s = path.to_string_lossy().to_lowercase();
        let s = s.trim_end_matches('\\');
        self.sync.iter().find(|r| s == r.key || s.strip_prefix(&r.key).is_some_and(|t| t.starts_with('\\')))
    }
}

#[derive(Default)]
struct Acc {
    big: Mutex<Vec<(u64, PathBuf)>>,
    exts: Mutex<HashMap<String, (u64, u64)>>,
}

fn scan(path: &Path, name: String, count: &AtomicU64, acc: &Acc) -> Dir {
    let (mut size, mut local, mut files) = (0, 0, 0);
    let mut subs = Vec::new();
    let mut exts: HashMap<String, (u64, u64)> = HashMap::new();
    for e in std::fs::read_dir(path).into_iter().flatten().flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue; // also skips junctions: no double counting, no loops
        }
        if ft.is_dir() {
            subs.push(e);
        } else if let Ok(m) = e.metadata() {
            let len = m.len();
            size += len;
            files += 1;
            if m.file_attributes() & CLOUD_ONLY == 0 {
                local += len;
            }
            let fname = e.file_name();
            let ext = Path::new(&fname).extension().map(|x| x.to_string_lossy()).unwrap_or_default();
            let x = match exts.get_mut(ext.as_ref()) {
                Some(x) => x, // hot path: no allocation for an extension already seen in this folder
                None => exts.entry(ext.to_lowercase()).or_default(),
            };
            x.0 += len;
            x.1 += 1;
            if len >= BIG_FILE {
                acc.big.lock().unwrap().push((len, e.path()));
            }
        }
    }
    count.fetch_add(files, Ordering::Relaxed);
    if !exts.is_empty() {
        let mut all = acc.exts.lock().unwrap();
        for (k, (b, n)) in exts {
            let x = all.entry(k).or_default();
            x.0 += b;
            x.1 += n;
        }
    }
    let mut dirs: Vec<Dir> = subs
        .par_iter()
        .map(|e| scan(&e.path(), e.file_name().to_string_lossy().into(), count, acc))
        .collect();
    dirs.sort_by(|a, b| b.size.cmp(&a.size));
    for d in &dirs {
        size += d.size;
        local += d.local;
        files += d.files;
    }
    Dir { name, size, local, files, dirs }
}

pub fn scan_all(count: &AtomicU64) -> Scan {
    let drives = drives();
    let acc = Acc::default();
    let mut dirs: Vec<Dir> = drives.par_iter().filter(|d| d.scanned()).map(|d| scan(Path::new(&d.root), d.root.clone(), count, &acc)).collect();
    dirs.sort_by(|a, b| b.size.cmp(&a.size));
    let sum = |f: fn(&Dir) -> u64| dirs.iter().map(f).sum();
    let root = Dir { name: "This PC".into(), size: sum(|d| d.size), local: sum(|d| d.local), files: sum(|d| d.files), dirs };
    let mut big = acc.big.into_inner().unwrap();
    big.sort_by(|a, b| b.0.cmp(&a.0));
    big.truncate(100);
    let mut exts: Vec<_> = acc.exts.into_inner().unwrap().into_iter().map(|(k, (b, n))| (k, b, n)).collect();
    exts.sort_by(|a, b| b.1.cmp(&a.1));
    Scan { root, drives, big, exts, sync: sync_roots(), known: known_folders() }
}

/// (name, size, online-only) of the files directly inside `p`.
pub fn list_files(p: &Path) -> Vec<(String, u64, bool)> {
    std::fs::read_dir(p)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((e.file_name().to_string_lossy().into(), m.len(), m.file_attributes() & CLOUD_ONLY != 0))
        })
        .collect()
}

/// Path to tree names. The drive comes back as one name ("C:" + "\" = "C:\").
pub fn names_of(p: &Path) -> Vec<String> {
    let mut f: Vec<String> = p.iter().map(|c| c.to_string_lossy().into()).collect();
    if f.len() >= 2 && f[1] == "\\" {
        let sep = f.remove(1);
        f[0].push_str(&sep);
    }
    f
}

pub fn find<'a>(root: &'a Dir, names: &[String]) -> Option<&'a Dir> {
    let mut d = root;
    for n in names {
        d = d.dirs.iter().find(|c| c.name.eq_ignore_ascii_case(n))?;
    }
    Some(d)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// Every drive letter except optical drives, tagged System / Local / USB / Network / Cloud.
// ponytail: a disconnected network drive can make this call slow; it runs on the scan thread so the UI stays live
fn drives() -> Vec<Drive> {
    use windows_sys::Win32::Storage::FileSystem::*;
    let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()) + "\\";
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .filter_map(|i| {
            let root = format!("{}:\\", (b'A' + i) as char);
            let w = wide(&root);
            let (mut label, mut fs) = ([0u16; 261], [0u16; 261]);
            let ok = unsafe {
                GetVolumeInformationW(w.as_ptr(), label.as_mut_ptr(), 261, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), fs.as_mut_ptr(), 261)
            };
            let s = |b: &[u16]| String::from_utf16_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(0)]);
            let local_fs = ok != 0 && ["NTFS", "ReFS", "FAT32", "exFAT", "FAT"].contains(&s(&fs).as_str());
            let kind = match unsafe { GetDriveTypeW(w.as_ptr()) } {
                4 => Kind::Network,
                2 if local_fs => Kind::Usb,
                3 | 6 if !local_fs => Kind::Cloud, // virtual drives such as Google Drive's G:
                3 | 6 if root.eq_ignore_ascii_case(&system) => Kind::System,
                3 | 6 => Kind::Local,
                _ => return None, // optical, unknown, or removable with no media
            };
            let (mut free, mut total) = (0u64, 0u64);
            unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut free, &mut total, std::ptr::null_mut()) };
            let label = match s(&label) {
                l if l.is_empty() => match kind {
                    Kind::Usb => "USB Drive",
                    Kind::Network => "Network Drive",
                    Kind::Cloud => "Cloud Drive",
                    _ => "Local Disk",
                }
                .to_string(),
                l => l,
            };
            Some(Drive { root, label, kind, total, free })
        })
        .collect()
}

/// Cloud sync folders registered with Windows (OneDrive, Google Drive, Dropbox, iCloud, Box, ...),
/// plus OneDrive's environment variables as a fallback.
fn sync_roots() -> Vec<SyncRoot> {
    let mut out: Vec<SyncRoot> = vec![];
    let mut add = |provider: String, path: PathBuf| {
        let key = path.to_string_lossy().to_lowercase().trim_end_matches('\\').to_string();
        if path.is_dir() && !out.iter().any(|r| r.key == key) {
            out.push(SyncRoot { provider, path, key });
        }
    };
    let mgr = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\SyncRootManager");
    for id in mgr.iter().flat_map(|k| k.enum_keys().flatten().collect::<Vec<_>>()) {
        let parts: Vec<&str> = id.split('!').collect();
        let provider = pretty_provider(parts[0], parts.get(2).copied());
        let Ok(roots) = mgr.as_ref().unwrap().open_subkey(format!(r"{id}\UserSyncRoots")) else { continue };
        for (_, v) in roots.enum_values().flatten() {
            add(provider.clone(), PathBuf::from(v.to_string().trim_matches('"')));
        }
    }
    for (var, name) in [("OneDriveConsumer", "OneDrive · Personal"), ("OneDriveCommercial", "OneDrive · Work or school"), ("OneDrive", "OneDrive")] {
        if let Ok(p) = std::env::var(var) {
            add(name.into(), p.into());
        }
    }
    out
}

fn pretty_provider(id: &str, account: Option<&str>) -> String {
    let l = id.to_lowercase();
    if l.contains("onedrive") {
        return match account {
            Some(a) if a.eq_ignore_ascii_case("personal") => "OneDrive · Personal".into(),
            Some(a) if a.to_lowercase().starts_with("business") => "OneDrive · Work or school".into(),
            _ => "OneDrive".into(),
        };
    }
    for (k, v) in [("google", "Google Drive"), ("dropbox", "Dropbox"), ("icloud", "iCloud Drive"), ("box", "Box")] {
        if l.contains(k) {
            return v.into();
        }
    }
    id.into()
}

fn known_folders() -> Vec<(&'static str, PathBuf)> {
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Shell Folders");
    let mut out = vec![("Home", PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default()))];
    for (label, value) in [
        ("Desktop", "Desktop"),
        ("Documents", "Personal"),
        ("Downloads", "{374DE290-123F-4565-9164-39C4925E467B}"),
        ("Pictures", "My Pictures"),
        ("Music", "My Music"),
        ("Videos", "My Video"),
    ] {
        if let Some(p) = key.as_ref().ok().and_then(|k| k.get_value::<String, _>(value).ok()) {
            out.push((label, p.into()));
        }
    }
    out
}

#[test]
fn scan_sums_nested_sizes() {
    let root = std::env::temp_dir().join("wams_test");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("a/b")).unwrap();
    std::fs::write(root.join("x.txt"), [0; 10]).unwrap();
    std::fs::write(root.join("a/y"), [0; 20]).unwrap();
    std::fs::write(root.join("a/b/z.txt"), [0; 30]).unwrap();
    let (n, acc) = (AtomicU64::new(0), Acc::default());
    let d = scan(&root, "t".into(), &n, &acc);
    assert_eq!((d.size, d.dirs[0].size, d.dirs[0].dirs[0].size), (60, 50, 30));
    assert_eq!((d.local, d.files, n.load(Ordering::Relaxed)), (60, 3, 3));
    assert_eq!(acc.exts.lock().unwrap()["txt"], (40, 2));
    assert_eq!(names_of(Path::new(r"C:\Users\me")), ["C:\\", "Users", "me"]);
}
