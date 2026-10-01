#![windows_subsystem = "windows"]
//! wams - who ate my storage. Folder/file sizes for every drive.

use eframe::egui;
use rayon::prelude::*;
use std::os::windows::process::CommandExt;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

const NO_WINDOW: u32 = 0x0800_0000;
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const UNINSTALL_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\wams";

/// Only directories are kept in memory; files are listed on demand when a folder is opened.
struct Dir {
    name: String,
    size: u64,
    dirs: Vec<Dir>,
}

fn scan(path: &Path, name: String, files: &AtomicU64) -> Dir {
    let mut size = 0;
    let mut subs = Vec::new();
    for e in std::fs::read_dir(path).into_iter().flatten().flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue; // also skips junctions: no double counting, no loops
        }
        if ft.is_dir() {
            subs.push(e);
        } else if let Ok(m) = e.metadata() {
            size += m.len();
            files.fetch_add(1, Ordering::Relaxed);
        }
    }
    let mut dirs: Vec<Dir> = subs
        .par_iter()
        .map(|e| scan(&e.path(), e.file_name().to_string_lossy().into(), files))
        .collect();
    dirs.sort_by(|a, b| b.size.cmp(&a.size));
    size += dirs.iter().map(|d| d.size).sum::<u64>();
    Dir { name, size, dirs }
}

fn scan_all(files: &AtomicU64) -> Dir {
    // ponytail: scans every existing drive letter incl. network/USB; filter with GetDriveTypeW if that gets slow
    let mut dirs: Vec<Dir> = (b'A'..=b'Z')
        .map(|c| format!("{}:\\", c as char))
        .filter(|d| Path::new(d).exists())
        .collect::<Vec<_>>()
        .into_par_iter()
        .map(|d| scan(Path::new(&d), d.clone(), files))
        .collect();
    dirs.sort_by(|a, b| b.size.cmp(&a.size));
    Dir { name: "This PC".into(), size: dirs.iter().map(|d| d.size).sum(), dirs }
}

fn human(b: u64) -> String {
    let mut v = b as f64;
    for u in ["B", "KB", "MB", "GB", "TB"] {
        if v < 1024.0 || u == "TB" {
            return if u == "B" { format!("{b} B") } else { format!("{v:.1} {u}") };
        }
        v /= 1024.0;
    }
    unreachable!()
}

struct Row {
    depth: f32,
    name: String,
    size: u64,
    parent: u64,
    path: PathBuf,
    dir: bool,
    open: bool,
}

fn list_files(p: &Path) -> Vec<(String, u64)> {
    std::fs::read_dir(p)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| (e.file_name().to_string_lossy().into(), e.metadata().map_or(0, |m| m.len())))
        .collect()
}

/// Flattens the expanded part of the tree into visible rows, children sorted by size.
fn walk(d: &Dir, path: &Path, depth: f32, open: &HashSet<PathBuf>, files: &mut HashMap<PathBuf, Vec<(String, u64)>>, out: &mut Vec<Row>) {
    let mut kids: Vec<(String, u64, Option<&Dir>)> = d.dirs.iter().map(|s| (s.name.clone(), s.size, Some(s))).collect();
    if !path.as_os_str().is_empty() {
        let f = files.entry(path.into()).or_insert_with(|| list_files(path));
        kids.extend(f.iter().map(|(n, s)| (n.clone(), *s, None)));
    }
    kids.sort_by(|a, b| b.1.cmp(&a.1));
    for (name, size, sub) in kids {
        let p = path.join(&name);
        let is_open = sub.is_some() && open.contains(&p);
        out.push(Row { depth, name, size, parent: d.size, path: p.clone(), dir: sub.is_some(), open: is_open });
        if let Some(s) = sub.filter(|_| is_open) {
            walk(s, &p, depth + 1.0, open, files, out);
        }
    }
}

#[derive(Default)]
struct App {
    root: Option<Dir>,
    focus: Vec<String>, // breadcrumb trail of folder names below "This PC"
    open: HashSet<PathBuf>,
    files: HashMap<PathBuf, Vec<(String, u64)>>,
    rows: Vec<Row>,
    dirty: bool,
    rx: Option<mpsc::Receiver<Dir>>,
    count: Arc<AtomicU64>,
}

impl App {
    fn refresh(&mut self) {
        let (tx, rx) = mpsc::channel();
        self.count = Arc::default();
        let count = self.count.clone();
        std::thread::spawn(move || tx.send(scan_all(&count)));
        self.rx = Some(rx);
    }

    fn focused(&self) -> Option<&Dir> {
        let mut d = self.root.as_ref()?;
        for n in &self.focus {
            d = d.dirs.iter().find(|c| &c.name == n)?;
        }
        Some(d)
    }
}

fn explorer(arg: String) {
    let _ = Command::new("explorer").raw_arg(arg).spawn();
}

const ROW_H: f32 = 26.0;

/// Paints the name / size / share columns of one row inside `rect`.
fn columns(ui: &egui::Ui, rect: egui::Rect, name: &str, size: &str, share: Option<f32>, strong: bool) {
    let v = ui.visuals();
    let p = ui.painter();
    let font = egui::FontId::proportional(14.0);
    let text = if strong { v.strong_text_color() } else { v.text_color() };
    let r = rect.right();
    let y = rect.center().y;
    let name_rect = egui::Rect::from_x_y_ranges(rect.left()..=r - 240.0, rect.y_range());
    p.with_clip_rect(name_rect)
        .text(egui::pos2(rect.left(), y), egui::Align2::LEFT_CENTER, name, font.clone(), text);
    p.text(egui::pos2(r - 140.0, y), egui::Align2::RIGHT_CENTER, size, font.clone(), text);
    match share {
        Some(s) => {
            let track = egui::Rect::from_min_max(egui::pos2(r - 126.0, y - 4.0), egui::pos2(r - 52.0, y + 4.0));
            p.rect_filled(track, 4.0, v.extreme_bg_color);
            let fill = egui::Color32::from_rgb(70, 150, 245).lerp_to_gamma(egui::Color32::from_rgb(240, 90, 60), s);
            let mut bar = track;
            bar.set_width((track.width() * s).max(2.0));
            p.rect_filled(bar, 4.0, fill);
            let pct = format!("{:.0}%", s * 100.0);
            p.text(egui::pos2(r - 8.0, y), egui::Align2::RIGHT_CENTER, pct, font, v.weak_text_color());
        }
        None => {
            p.text(egui::pos2(r - 89.0, y), egui::Align2::CENTER_CENTER, "Share", font, text);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, root_ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = root_ui.ctx().clone();
        if let Some(Ok(root)) = self.rx.as_ref().map(|rx| rx.try_recv()) {
            self.rx = None;
            self.root = Some(root);
            self.files.clear(); // expanded folders stay expanded
            self.dirty = true;
        }
        if self.root.is_some() && self.focused().is_none() {
            self.focus.clear(); // focused folder vanished after refresh
        }
        if self.dirty {
            self.dirty = false;
            let base: PathBuf = self.focus.iter().collect();
            let (mut rows, mut files) = (vec![], std::mem::take(&mut self.files));
            if let Some(d) = self.focused() {
                walk(d, &base, 0.0, &self.open, &mut files, &mut rows);
            }
            (self.rows, self.files) = (rows, files);
        }

        egui::Panel::top("bar").show(root_ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading("Who Ate My Storage");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(self.rx.is_none(), egui::Button::new("⟳  Refresh")).clicked() {
                        self.refresh();
                    }
                });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let mut go = None;
                let last = self.focus.len();
                for i in 0..=last {
                    if i > 0 {
                        ui.weak("›");
                    }
                    let name = if i == 0 { "🖥 This PC" } else { self.focus[i - 1].trim_end_matches('\\') };
                    let text = egui::RichText::new(name);
                    let text = if i == last { text.strong() } else { text };
                    if ui.add(egui::Button::new(text).frame(i == last)).clicked() {
                        go = Some(i);
                    }
                }
                if let Some(i) = go {
                    self.focus.truncate(i);
                    self.dirty = true;
                }
                if let Some(d) = self.focused() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.strong(human(d.size));
                    });
                }
            });
            ui.add_space(6.0);
        });

        egui::Panel::bottom("status").show(root_ui, |ui| {
            ui.horizontal(|ui| {
                let n = self.count.load(Ordering::Relaxed);
                if self.rx.is_some() {
                    ui.spinner();
                    ui.label(format!("Scanning… {n} files"));
                    ctx.request_repaint_after(std::time::Duration::from_millis(200));
                } else {
                    ui.weak(format!(
                        "{n} files scanned  ·  click to expand  ·  double-click to open  ·  right-click for Explorer"
                    ));
                }
            });
        });

        egui::CentralPanel::default().show(root_ui, |ui| {
            let (head, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), egui::Sense::hover());
            columns(ui, head.shrink2(egui::vec2(8.0, 0.0)), "Name", "Size", None, true);
            ui.separator();
            let mut toggle = None;
            let mut drill = None;
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW_H, self.rows.len(), |ui, range| {
                for (i, r) in self.rows[range.clone()].iter().enumerate() {
                    let size = egui::vec2(ui.available_width(), ROW_H);
                    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
                    let v = ui.visuals();
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, 4.0, v.widgets.hovered.weak_bg_fill);
                    } else if (range.start + i) % 2 == 1 {
                        ui.painter().rect_filled(rect, 0.0, v.faint_bg_color);
                    }
                    let (arrow, icon, tint) = match (r.dir, r.open) {
                        (true, true) => ("⏷", "🗁", egui::Color32::from_rgb(240, 190, 70)),
                        (true, false) => ("⏵", "🗀", egui::Color32::from_rgb(240, 190, 70)),
                        _ => ("", "🗋", v.weak_text_color()),
                    };
                    let mut inner = rect.shrink2(egui::vec2(8.0, 0.0));
                    inner.min.x += r.depth * 20.0;
                    let (y, x, f) = (inner.center().y, inner.min.x, egui::FontId::proportional(15.0));
                    let p = ui.painter();
                    p.text(egui::pos2(x, y), egui::Align2::LEFT_CENTER, arrow, f.clone(), v.weak_text_color());
                    p.text(egui::pos2(x + 18.0, y), egui::Align2::LEFT_CENTER, icon, f, tint);
                    inner.min.x += 42.0;
                    let share = r.size as f32 / r.parent.max(1) as f32;
                    columns(ui, inner, &r.name, &human(r.size), Some(share), false);
                    let show = || {
                        let p = r.path.display();
                        explorer(if r.dir { format!("\"{p}\"") } else { format!("/select,\"{p}\"") })
                    };
                    if resp.double_clicked() {
                        if r.dir {
                            drill = Some(r.path.clone())
                        } else {
                            show()
                        }
                    } else if resp.clicked() && r.dir {
                        toggle = Some(r.path.clone());
                    }
                    resp.context_menu(|ui| {
                        if r.dir && ui.button("📂  Open here").clicked() {
                            drill = Some(r.path.clone());
                        }
                        if ui.button("🗔  Show in Explorer").clicked() {
                            show();
                        }
                    });
                }
            });
            if let Some(p) = toggle {
                if !self.open.remove(&p) {
                    self.open.insert(p);
                }
                self.dirty = true;
            }
            if let Some(p) = drill {
                let mut f: Vec<String> = p.iter().map(|c| c.to_string_lossy().into()).collect();
                let sep = f.remove(1); // "C:" + "\\" back into the drive name "C:\\"
                f[0].push_str(&sep);
                self.focus = f;
                self.dirty = true;
            }
        });
    }
}

fn reg(args: &[&str]) {
    let _ = Command::new("reg").args(args).creation_flags(NO_WINDOW).status();
}

/// Per-user install: copy to %LOCALAPPDATA%\Programs\wams, run at login, list in Apps & features.
/// Returns true when this process should exit.
fn install() -> bool {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--portable") {
        return false;
    }
    let dir = PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join(r"Programs\wams");
    let target = dir.join("wams.exe");
    let t = target.display().to_string();
    if args.iter().any(|a| a == "--uninstall") {
        reg(&["delete", RUN_KEY, "/v", "wams", "/f"]);
        reg(&["delete", UNINSTALL_KEY, "/f"]);
        let _ = Command::new("cmd")
            .raw_arg(format!("/c timeout /t 2 >nul & rmdir /s /q \"{}\"", dir.display()))
            .creation_flags(NO_WINDOW)
            .spawn();
        return true;
    }
    let exe = std::env::current_exe().unwrap_or_default();
    if exe.display().to_string().eq_ignore_ascii_case(&t) {
        return false;
    }
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::copy(&exe, &target); // fails if an old copy is running; that copy keeps working
    let quoted = format!("\"{t}\"");
    let uninstall = format!("{quoted} --uninstall");
    reg(&["add", RUN_KEY, "/v", "wams", "/t", "REG_SZ", "/d", &quoted, "/f"]);
    for (k, v) in [
        ("DisplayName", "WAMS - Who Ate My Storage"),
        ("DisplayIcon", t.as_str()),
        ("UninstallString", uninstall.as_str()),
    ] {
        reg(&["add", UNINSTALL_KEY, "/v", k, "/t", "REG_SZ", "/d", v, "/f"]);
    }
    reg(&["add", UNINSTALL_KEY, "/v", "NoModify", "/t", "REG_DWORD", "/d", "1", "/f"]);
    Command::new(&target).spawn().is_ok()
}

fn main() -> eframe::Result {
    if install() {
        return Ok(());
    }
    eframe::run_native(
        "WAMS - Who Ate My Storage",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([960.0, 680.0]),
            ..Default::default()
        },
        Box::new(|_| {
            let mut app = App::default();
            app.refresh();
            Ok(Box::new(app))
        }),
    )
}

#[test]
fn scan_sums_nested_sizes() {
    let root = std::env::temp_dir().join("wams_test");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("a/b")).unwrap();
    std::fs::write(root.join("x"), [0; 10]).unwrap();
    std::fs::write(root.join("a/y"), [0; 20]).unwrap();
    std::fs::write(root.join("a/b/z"), [0; 30]).unwrap();
    let n = AtomicU64::new(0);
    let d = scan(&root, "t".into(), &n);
    assert_eq!((d.size, d.dirs[0].size, d.dirs[0].dirs[0].size), (60, 50, 30));
    assert_eq!(n.load(Ordering::Relaxed), 3);
    assert_eq!(human(1536), "1.5 KB");
}
