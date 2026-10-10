//! Theme, layout and the three views (Explorer, Disk analysis, Cloud backup).

use crate::scan::{self, Dir, Kind, Scan};
use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontFamily, FontId, Margin, Rect, RichText, Sense, Stroke, Ui, pos2, vec2,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

/// Segoe Fluent Icons / MDL2 Assets code points (loaded from C:\Windows\Fonts).
#[cfg(windows)]
mod icon {
    pub const FOLDER: &str = "\u{E8B7}";
    pub const FOLDER_OPEN: &str = "\u{E838}";
    pub const FILE: &str = "\u{E7C3}";
    pub const DRIVE: &str = "\u{EDA2}";
    pub const CLOUD: &str = "\u{E753}";
    pub const REFRESH: &str = "\u{E72C}";
    pub const HOME: &str = "\u{E80F}";
    pub const DESKTOP: &str = "\u{E7F4}";
    pub const DOCS: &str = "\u{E8A5}";
    pub const DOWNLOAD: &str = "\u{E896}";
    pub const PICTURES: &str = "\u{EB9F}";
    pub const MUSIC: &str = "\u{EC4F}";
    pub const VIDEO: &str = "\u{E714}";
    pub const PIE: &str = "\u{EB05}";
    pub const EXPLORER: &str = "\u{EC50}";
    pub const PC: &str = "\u{E977}";
    pub const RIGHT: &str = "\u{E76C}";
    pub const DOWN: &str = "\u{E70D}";
    pub const CHECK: &str = "\u{E930}";
    pub const WARN: &str = "\u{E7BA}";
    pub const COPY: &str = "\u{E8C8}";
    pub const OPEN: &str = "\u{E8A7}";
    pub const UP: &str = "\u{E74A}";
}

/// macOS: glyphs from egui's bundled emoji-icon-font / Noto Emoji.
#[cfg(not(windows))]
mod icon {
    pub const FOLDER: &str = "🗀";
    pub const FOLDER_OPEN: &str = "🗁";
    pub const FILE: &str = "🗋";
    pub const DRIVE: &str = "🖴";
    pub const CLOUD: &str = "☁";
    pub const REFRESH: &str = "⟳";
    pub const HOME: &str = "🏠";
    pub const DESKTOP: &str = "🖵";
    pub const DOCS: &str = "🗐";
    pub const DOWNLOAD: &str = "⬇";
    pub const PICTURES: &str = "🖼";
    pub const MUSIC: &str = "🎵";
    pub const VIDEO: &str = "🎞";
    pub const PIE: &str = "◔";
    pub const EXPLORER: &str = "🗄";
    pub const PC: &str = "🖥";
    pub const RIGHT: &str = "⏵";
    pub const DOWN: &str = "⏷";
    pub const CHECK: &str = "✔";
    pub const WARN: &str = "⚠";
    pub const COPY: &str = "🗐";
    pub const OPEN: &str = "⮫";
    pub const UP: &str = "⬆";
}

const ROW_H: f32 = 24.0;
const FOLDER_GREY: Color32 = Color32::from_rgb(128, 128, 128);
const ACCENT: Color32 = Color32::from_rgb(10, 132, 255);
const TILE_COLORS: [Color32; 8] = [
    Color32::from_rgb(56, 132, 255),
    Color32::from_rgb(139, 92, 246),
    Color32::from_rgb(16, 185, 129),
    Color32::from_rgb(245, 158, 11),
    Color32::from_rgb(236, 72, 153),
    Color32::from_rgb(6, 182, 212),
    Color32::from_rgb(239, 68, 68),
    Color32::from_rgb(132, 204, 22),
];

/// Atlas-style palette for one theme.
struct Pal {
    nav: Color32,
    content: Color32,
    card: Color32,
    stroke: Color32,
    hover: Color32,
    track: Color32,
    text: Color32,
    weak: Color32,
    accent: Color32,
    ok: Color32,
    warn: Color32,
}

fn pal(dark: bool, accent: Color32) -> Pal {
    let c = Color32::from_rgb;
    if dark {
        Pal {
            nav: c(19, 19, 19),
            content: c(28, 28, 30),
            card: c(28, 28, 30),
            stroke: c(44, 44, 46),
            hover: c(44, 44, 46),
            track: c(44, 44, 46),
            text: c(229, 229, 234),
            weak: c(142, 142, 147),
            accent,
            ok: c(108, 203, 95),
            warn: c(252, 166, 72),
        }
    } else {
        Pal {
            nav: c(238, 240, 243),
            content: c(249, 249, 249),
            card: c(255, 255, 255),
            stroke: c(229, 229, 229),
            hover: c(232, 234, 237),
            track: c(222, 222, 222),
            text: c(26, 26, 26),
            weak: c(96, 96, 96),
            accent,
            ok: c(15, 123, 15),
            warn: c(172, 92, 0),
        }
    }
}

fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

/// System UI font (Segoe UI / SF Pro) + Segoe Fluent Icons on Windows, egui's bundled fonts as fallback.
fn load_fonts(ctx: &egui::Context) {
    #[cfg(windows)]
    let (dir, ui, icons, semi): (PathBuf, &[&str], &[&str], &[&str]) = (
        PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into())).join("Fonts"),
        &["segoeui.ttf"],
        &["SegoeIcons.ttf", "segmdl2.ttf"],
        &["seguisb.ttf"],
    );
    #[cfg(not(windows))]
    let (dir, ui, icons, semi): (PathBuf, &[&str], &[&str], &[&str]) = ("/System/Library/Fonts".into(), &["SFNS.ttf"], &[], &[]);
    let mut f = egui::FontDefinitions::default();
    let mut put = |key: &str, files: &[&str]| {
        let bytes = files.iter().find_map(|n| std::fs::read(dir.join(n)).ok());
        bytes.map(|b| f.font_data.insert(key.into(), Arc::new(egui::FontData::from_owned(b)))).is_some()
    };
    let (seg, ico, sb) = (put("segoe", ui), put("icons", icons), put("semibold", semi));
    let prop = f.families.get_mut(&FontFamily::Proportional).unwrap();
    if ico {
        prop.insert(0, "icons".into());
    }
    if seg {
        prop.insert(0, "segoe".into());
    }
    let mut bold = prop.clone();
    if sb {
        bold.insert(0, "semibold".into());
    }
    f.families.insert(FontFamily::Name("semibold".into()), bold);
    ctx.set_fonts(f);
}

fn apply_theme(ctx: &egui::Context, accent: Color32) {
    for (theme, dark) in [(egui::Theme::Dark, true), (egui::Theme::Light, false)] {
        let p = pal(dark, accent);
        let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        v.panel_fill = p.content;
        v.window_fill = p.card;
        v.window_stroke = Stroke::new(1.0, p.stroke);
        v.extreme_bg_color = p.content;
        v.faint_bg_color = p.hover;
        v.hyperlink_color = p.accent;
        v.selection.bg_fill = p.accent.gamma_multiply(0.4);
        v.selection.stroke = Stroke::new(1.0, p.accent);
        v.window_corner_radius = 8.into();
        v.menu_corner_radius = 8.into();
        for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = 6.into();
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.stroke);
        v.widgets.noninteractive.fg_stroke.color = p.text;
        v.widgets.inactive.weak_bg_fill = p.card;
        v.widgets.inactive.bg_fill = p.card;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.stroke);
        v.widgets.inactive.fg_stroke.color = p.text;
        v.widgets.hovered.weak_bg_fill = p.hover;
        v.widgets.hovered.bg_fill = p.hover;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.stroke);
        v.widgets.active.weak_bg_fill = p.stroke;
        v.widgets.active.bg_fill = p.stroke;
        ctx.set_visuals_of(theme, v);
    }
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = vec2(8.0, 6.0);
        s.spacing.button_padding = vec2(12.0, 6.0);
        s.spacing.interact_size.y = 26.0;
        use egui::TextStyle::*;
        s.text_styles = [
            (Small, FontId::proportional(12.0)),
            (Body, FontId::proportional(13.0)),
            (Button, FontId::proportional(13.0)),
            (Monospace, FontId::monospace(13.0)),
            (Heading, semibold(26.0)),
        ]
        .into();
    });
}

pub fn human(b: u64) -> String {
    let mut v = b as f64;
    for u in ["B", "KB", "MB", "GB", "TB"] {
        if v < 1024.0 || u == "TB" {
            return if u == "B" { format!("{b} B") } else { format!("{v:.1} {u}") };
        }
        v /= 1024.0;
    }
    unreachable!()
}

fn num(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Squarified treemap: `values` sorted biggest first, returns one rect per value.
fn squarify(values: &[f64], rect: Rect) -> Vec<Rect> {
    let total: f64 = values.iter().sum();
    let scale = if total > 0.0 { (rect.width() * rect.height()) as f64 / total } else { 0.0 };
    let a: Vec<f64> = values.iter().map(|v| v * scale).collect();
    let mut out = Vec::with_capacity(a.len());
    let mut rest = rect;
    let mut i = 0;
    while i < a.len() {
        if a[i] <= 0.0 || rest.width() <= 0.5 || rest.height() <= 0.5 {
            out.push(Rect::NOTHING);
            i += 1;
            continue;
        }
        let short = rest.width().min(rest.height()) as f64;
        let worst = |row: &[f64]| {
            let s: f64 = row.iter().sum();
            (short * short * row[0] / (s * s)).max(s * s / (short * short * row[row.len() - 1]))
        };
        let mut j = i + 1;
        while j < a.len() && a[j] > 0.0 && worst(&a[i..=j]) <= worst(&a[i..j]) {
            j += 1;
        }
        let s: f64 = a[i..j].iter().sum();
        if rest.width() >= rest.height() {
            let w = (s / rest.height() as f64) as f32;
            let mut y = rest.top();
            for &v in &a[i..j] {
                let h = (v / w as f64) as f32;
                out.push(Rect::from_min_size(pos2(rest.left(), y), vec2(w, h)));
                y += h;
            }
            rest.min.x += w;
        } else {
            let h = (s / rest.width() as f64) as f32;
            let mut x = rest.left();
            for &v in &a[i..j] {
                let w = (v / h as f64) as f32;
                out.push(Rect::from_min_size(pos2(x, rest.top()), vec2(w, h)));
                x += w;
            }
            rest.min.y += h;
        }
        i = j;
    }
    out
}

#[derive(PartialEq, Clone, Copy)]
enum View {
    Explorer,
    Analysis,
    Cloud,
}

struct Row {
    depth: f32,
    name: String,
    size: u64,
    local: u64,
    files: u64,
    parent: u64,
    path: PathBuf,
    dir: bool,
    open: bool,
    synced: bool,
}

/// UI events, applied after drawing so views only need `&self`.
enum Act {
    Focus(PathBuf),
    Toggle(PathBuf),
    Select(PathBuf),
    View(View),
    Refresh,
    Show(PathBuf, bool),
    Copy(String),
}

type FileCache = HashMap<PathBuf, Vec<(String, u64, bool)>>;

/// Flattens the expanded part of the tree into visible rows, children sorted by size.
fn walk(d: &Dir, path: &Path, depth: f32, open: &HashSet<PathBuf>, files: &mut FileCache, s: &Scan, out: &mut Vec<Row>) {
    let mut kids: Vec<(String, u64, u64, Option<&Dir>)> = d.dirs.iter().map(|c| (c.name.clone(), c.size, c.local, Some(c))).collect();
    if !path.as_os_str().is_empty() {
        let f = files.entry(path.into()).or_insert_with(|| scan::list_files(path));
        kids.extend(f.iter().map(|(n, sz, cloud)| (n.clone(), *sz, if *cloud { 0 } else { *sz }, None)));
    }
    kids.sort_by(|a, b| b.1.cmp(&a.1));
    for (name, size, local, sub) in kids {
        let p = path.join(&name);
        let is_open = sub.is_some() && open.contains(&p);
        let synced = s.synced(&p).is_some();
        let nfiles = sub.map_or(0, |c| c.files);
        out.push(Row { depth, name, size, local, files: nfiles, parent: d.size, path: p.clone(), dir: sub.is_some(), open: is_open, synced });
        if let Some(c) = sub.filter(|_| is_open) {
            walk(c, &p, depth + 1.0, open, files, s, out);
        }
    }
}

#[cfg(windows)]
fn show_in_explorer(p: &Path, dir: bool) {
    use std::os::windows::process::CommandExt;
    let p = p.display();
    let arg = if dir { format!("\"{p}\"") } else { format!("/select,\"{p}\"") };
    let _ = std::process::Command::new("explorer").raw_arg(arg).spawn();
}

/// Finder: open the folder, or reveal the file.
#[cfg(not(windows))]
fn show_in_explorer(p: &Path, dir: bool) {
    let _ = std::process::Command::new("open").args(if dir { &[][..] } else { &["-R"][..] }).arg(p).spawn();
}

fn card(p: &Pal) -> egui::Frame {
    egui::Frame::new().fill(p.card).stroke(Stroke::new(1.0, p.stroke)).corner_radius(8.0).inner_margin(16.0)
}

fn badge(ui: &mut Ui, text: &str, color: Color32) {
    egui::Frame::new().fill(color.gamma_multiply(0.15)).corner_radius(10.0).inner_margin(Margin::symmetric(8, 3)).show(ui, |ui| {
        ui.label(RichText::new(text).color(color).size(12.0));
    });
}

/// Rounded tag drawn with the painter; returns its rect.
fn pill(ui: &Ui, left_center: egui::Pos2, s: &str, color: Color32) -> Rect {
    let g = ui.painter().layout_no_wrap(s.to_string(), FontId::proportional(11.5), color);
    let r = Rect::from_min_size(pos2(left_center.x, left_center.y - 10.0), vec2(g.size().x + 14.0, 20.0));
    ui.painter().rect_filled(r, 10.0, color.gamma_multiply(0.18));
    ui.painter().galley(r.left_top() + vec2(7.0, (20.0 - g.size().y) / 2.0), g, color);
    r
}

fn kind_color(p: &Pal, k: Kind) -> Option<Color32> {
    match k {
        Kind::System => Some(p.accent),
        Kind::Network => Some(TILE_COLORS[1]),
        Kind::Usb => Some(TILE_COLORS[2]),
        Kind::Cloud => Some(TILE_COLORS[5]),
        Kind::Local => None,
    }
}

fn drive_name(d: &scan::Drive) -> String {
    format!("{} ({})", d.label, d.root.trim_end_matches('\\'))
}

/// Scanned drives open in the tree; network and cloud drives open in Windows Explorer.
fn open_drive(d: &scan::Drive) -> Act {
    if d.scanned() { Act::Focus(d.root.clone().into()) } else { Act::Show(d.root.clone().into(), true) }
}

fn bar(ui: &Ui, rect: Rect, frac: f32, fill: Color32, track: Color32) {
    let r = rect.height() / 2.0;
    ui.painter().rect_filled(rect, r, track);
    let mut b = rect;
    b.set_width((rect.width() * frac.clamp(0.0, 1.0)).max(rect.height()));
    ui.painter().rect_filled(b, r, fill);
}

fn text(ui: &Ui, pos: egui::Pos2, align: Align2, s: &str, font: FontId, color: Color32) -> Rect {
    ui.painter().text(pos, align, s, font, color)
}

fn section(ui: &mut Ui, p: &Pal, title: &str) {
    ui.add_space(14.0);
    ui.label(RichText::new(title).font(semibold(12.0)).color(p.weak));
    ui.add_space(2.0);
}

/// Win11 navigation item: icon + label, accent pill when active.
fn nav_item(ui: &mut Ui, p: &Pal, icon: &str, label: &str, active: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 27.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let bg = if active || resp.hovered() { p.hover } else { Color32::TRANSPARENT };
    ui.painter().rect_filled(rect, 6.0, bg);
    let y = rect.center().y;
    text(ui, pos2(rect.left() + 10.0, y), Align2::LEFT_CENTER, icon, FontId::proportional(14.0), p.text);
    text(ui, pos2(rect.left() + 34.0, y), Align2::LEFT_CENTER, label, FontId::proportional(13.0), p.text);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn known_icon(label: &str) -> &'static str {
    match label {
        "Home" => icon::HOME,
        "Desktop" => icon::DESKTOP,
        "Documents" => icon::DOCS,
        "Downloads" => icon::DOWNLOAD,
        "Pictures" => icon::PICTURES,
        "Music" => icon::MUSIC,
        "Videos" => icon::VIDEO,
        _ => icon::FOLDER,
    }
}

fn display_name(p: &Path) -> String {
    p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into())
}

pub struct App {
    scan: Option<Scan>,
    rx: Option<mpsc::Receiver<Scan>>,
    count: Arc<AtomicU64>,
    view: View,
    focus: PathBuf, // empty = This PC
    open: HashSet<PathBuf>,
    files: FileCache,
    rows: Vec<Row>,
    dirty: bool,
    selected: Option<PathBuf>,
    accent: Color32,
}

impl App {
    pub fn new(ctx: &egui::Context) -> Self {
        load_fonts(ctx);
        let accent = ACCENT;
        apply_theme(ctx, accent);
        ctx.set_theme(egui::Theme::Dark);
        let mut app = App {
            scan: None,
            rx: None,
            count: Arc::default(),
            view: View::Explorer,
            focus: PathBuf::new(),
            open: HashSet::new(),
            files: HashMap::new(),
            rows: vec![],
            dirty: false,
            selected: None,
            accent,
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        let (tx, rx) = mpsc::channel();
        self.count = Arc::default();
        let count = self.count.clone();
        std::thread::spawn(move || tx.send(scan::scan_all(&count)));
        self.rx = Some(rx);
    }

    fn nav(&self, ui: &mut Ui, p: &Pal, acts: &mut Vec<Act>) {
        egui::Panel::bottom("nav_foot").frame(egui::Frame::new().inner_margin(Margin { top: 8, ..Default::default() })).show_separator_line(false).show(ui, |ui| {
            let n = num(self.count.load(Ordering::Relaxed));
            if self.scan.is_none() {
                // first scan: the central loader already shows progress
            } else if self.rx.is_some() {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(14.0).color(p.accent));
                    ui.label(RichText::new(format!("Scanning… {n} files")).color(p.weak).size(12.0));
                });
            } else {
                ui.label(RichText::new(format!("{n} files indexed")).color(p.weak).size(12.0));
                let b = egui::Button::new(format!("{}   Rescan all drives", icon::REFRESH)).min_size(vec2(ui.available_width(), 32.0));
                if ui.add(b).clicked() {
                    acts.push(Act::Refresh);
                }
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(icon::PIE).size(26.0).color(p.accent));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new("WAMS").font(semibold(18.0)).color(p.text));
                ui.label(RichText::new("Who Ate My Storage").size(12.0).color(p.weak));
            });
        });
        ui.add_space(14.0);
        for (v, ic, label) in [(View::Explorer, icon::EXPLORER, "Explorer"), (View::Analysis, icon::PIE, "Disk analysis"), (View::Cloud, icon::CLOUD, "Cloud backup")] {
            if nav_item(ui, p, ic, label, self.view == v).clicked() {
                acts.push(Act::View(v));
            }
        }
        let Some(s) = &self.scan else { return };
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            section(ui, p, "Drives");
            for d in &s.drives {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
                let name = drive_name(d);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{name}, {} drive", d.kind.name())));
                if resp.hovered() || self.focus.as_os_str().eq_ignore_ascii_case(&d.root) {
                    ui.painter().rect_filled(rect, 6.0, p.hover);
                }
                let x = rect.left() + 14.0;
                text(ui, pos2(x, rect.top() + 17.0), Align2::LEFT_CENTER, icon::DRIVE, FontId::proportional(16.0), p.text);
                let nr = text(ui, pos2(x + 30.0, rect.top() + 15.0), Align2::LEFT_CENTER, &name, FontId::proportional(13.0), p.text);
                if let Some(c) = kind_color(p, d.kind) {
                    pill(ui, pos2(nr.right() + 6.0, nr.center().y), d.kind.name(), c);
                }
                let used = d.total.saturating_sub(d.free);
                let frac = used as f32 / d.total.max(1) as f32;
                let fill = if frac > 0.9 { p.warn } else { p.accent };
                bar(ui, Rect::from_min_size(pos2(x + 30.0, rect.top() + 26.0), vec2(rect.width() - 58.0, 4.0)), frac, fill, p.track);
                let free = match (d.scanned(), d.total) {
                    (true, _) => format!("{} free of {}", human(d.free), human(d.total)),
                    (false, 0) => "Not scanned".to_string(),
                    (false, _) => format!("{} free of {} · not scanned", human(d.free), human(d.total)),
                };
                text(ui, pos2(x + 30.0, rect.top() + 41.0), Align2::LEFT_CENTER, &free, FontId::proportional(11.5), p.weak);
                if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    acts.push(open_drive(d));
                }
            }
            section(ui, p, "Quick access");
            for (label, path) in &s.known {
                if nav_item(ui, p, known_icon(label), label, false).clicked() {
                    acts.push(Act::Focus(path.clone()));
                    acts.push(Act::View(View::Explorer));
                }
            }
            for r in &s.sync {
                if nav_item(ui, p, icon::CLOUD, &r.provider, false).on_hover_text(r.path.display().to_string()).clicked() {
                    acts.push(Act::Focus(r.path.clone()));
                    acts.push(Act::View(View::Explorer));
                }
            }
        });
    }

    /// Up button + breadcrumbs + size of the focused folder.
    fn header(&self, ui: &mut Ui, p: &Pal, acts: &mut Vec<Act>) {
        let s = self.scan.as_ref().unwrap();
        ui.horizontal(|ui| {
            let names = scan::names_of(&self.focus);
            let up = egui::Button::new(RichText::new(icon::UP).size(14.0)).min_size(vec2(32.0, 32.0));
            if ui.add_enabled(!names.is_empty(), up).on_hover_text("Up one level").clicked() {
                acts.push(Act::Focus(names[..names.len() - 1].iter().collect()));
            }
            ui.add_space(4.0);
            ui.spacing_mut().item_spacing.x = 2.0;
            let crumbs = std::iter::once(format!("{}  This PC", icon::PC)).chain(names.iter().map(|n| n.trim_end_matches('\\').to_string()));
            for (i, c) in crumbs.enumerate() {
                if i > 0 {
                    ui.label(RichText::new(icon::RIGHT).size(10.0).color(p.weak));
                }
                let last = i == names.len();
                let t = RichText::new(c).size(15.0).color(if last { p.text } else { p.weak });
                let t = if last { t.font(semibold(15.0)) } else { t };
                let b = egui::Button::new(t).frame(false).min_size(vec2(0.0, 30.0));
                if ui.add(b).on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    acts.push(Act::Focus(names[..i].iter().collect()));
                }
            }
            if let Some(d) = s.find(&self.focus) {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(RichText::new(human(d.size)).font(semibold(16.0)).color(p.text));
                    ui.label(RichText::new(format!("{} files  ·", num(d.files))).color(p.weak));
                });
            }
        });
    }

    fn explorer_view(&self, ui: &mut Ui, p: &Pal, acts: &mut Vec<Act>) {
        self.header(ui, p, acts);
        ui.add_space(10.0);
        egui::Frame::new().show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
            let (y, r, l) = (head.center().y, head.right(), head.left() + 16.0);
            let f = FontId::proportional(12.5);
            text(ui, pos2(l + 44.0, y), Align2::LEFT_CENTER, "Name", f.clone(), p.weak);
            text(ui, pos2(r - 300.0, y), Align2::CENTER_CENTER, "Backup", f.clone(), p.weak);
            text(ui, pos2(r - 236.0, y), Align2::RIGHT_CENTER, "Files", f.clone(), p.weak);
            text(ui, pos2(r - 148.0, y), Align2::RIGHT_CENTER, "Size", f.clone(), p.weak);
            text(ui, pos2(r - 96.0, y), Align2::CENTER_CENTER, "Share", f, p.weak);
            ui.painter().hline(head.x_range(), head.bottom(), Stroke::new(1.0, p.stroke));
            if self.rows.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| ui.label(RichText::new("This folder is empty").color(p.weak)));
                return;
            }
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW_H, self.rows.len(), |ui, range| {
                for (i, row) in self.rows[range.clone()].iter().enumerate() {
                    self.row(ui, p, row, range.start + i, acts);
                }
            });
        });
    }

    fn row(&self, ui: &mut Ui, p: &Pal, r: &Row, idx: usize, acts: &mut Vec<Act>) {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{}, {}", r.name, human(r.size))));
        let sel = self.selected.as_ref() == Some(&r.path);
        let bg = if sel { p.accent } else if resp.hovered() { p.hover } else if idx % 2 == 1 { p.nav.gamma_multiply(0.6) } else { Color32::TRANSPARENT };
        ui.painter().rect_filled(rect, 0.0, bg);
        let (fg, weak) = if sel { (Color32::WHITE, Color32::WHITE) } else { (p.text, p.weak) };
        let (y, rr) = (rect.center().y, rect.right());
        let x = rect.left() + 16.0 + r.depth * 20.0;
        let arrow = Rect::from_min_max(pos2(x - 6.0, rect.top()), pos2(x + 18.0, rect.bottom()));
        if r.dir {
            let a = if r.open { icon::DOWN } else { icon::RIGHT };
            text(ui, pos2(x + 6.0, y), Align2::CENTER_CENTER, a, FontId::proportional(10.0), weak);
        }
        let drive = self.scan.as_ref().and_then(|s| s.drive(&r.path));
        let (ic, tint) = match (r.dir, r.open) {
            _ if drive.is_some() => (icon::DRIVE, fg),
            (true, true) => (icon::FOLDER_OPEN, FOLDER_GREY),
            (true, false) => (icon::FOLDER, FOLDER_GREY),
            _ => (icon::FILE, weak),
        };
        let tint = if sel { fg } else { tint };
        text(ui, pos2(x + 22.0, y), Align2::LEFT_CENTER, ic, FontId::proportional(14.0), tint);
        let name_clip = Rect::from_x_y_ranges(x + 44.0..=rr - 330.0, rect.y_range());
        let label = drive.map_or_else(|| r.name.clone(), drive_name);
        let nr = ui.painter().with_clip_rect(name_clip).text(pos2(x + 46.0, y), Align2::LEFT_CENTER, label, FontId::proportional(13.0), fg);
        if let Some(d) = drive {
            if let Some(c) = kind_color(p, d.kind) {
                pill(ui, pos2(nr.right() + 8.0, y), &format!("{} drive", d.kind.name()), c);
            }
        }
        if r.synced {
            let online = r.size - r.local.min(r.size);
            let (ic, c, tip) = if online > 0 {
                (icon::CLOUD, p.accent, format!("Backed up · {} online-only (not using space on this PC)", human(online)))
            } else {
                (icon::CHECK, p.ok, "Backed up · available on this PC".to_string())
            };
            let sr = text(ui, pos2(rr - 300.0, y), Align2::CENTER_CENTER, ic, FontId::proportional(15.0), c);
            ui.interact(sr.expand(4.0), ui.id().with(("status", idx)), Sense::hover()).on_hover_text(tip);
        }
        if r.dir {
            text(ui, pos2(rr - 236.0, y), Align2::RIGHT_CENTER, &num(r.files), FontId::proportional(13.0), weak);
        }
        text(ui, pos2(rr - 148.0, y), Align2::RIGHT_CENTER, &human(r.size), FontId::proportional(13.0), fg);
        let share = r.size as f32 / r.parent.max(1) as f32;
        bar(ui, Rect::from_min_max(pos2(rr - 132.0, y - 3.0), pos2(rr - 62.0, y + 3.0)), share, if sel { fg } else { p.accent }, p.track);
        text(ui, pos2(rr - 16.0, y), Align2::RIGHT_CENTER, &format!("{:.0}%", share * 100.0), FontId::proportional(12.5), weak);

        if resp.double_clicked() {
            if r.dir {
                acts.push(Act::Focus(r.path.clone()));
            } else {
                acts.push(Act::Show(r.path.clone(), false));
            }
        } else if resp.clicked() {
            let on_arrow = resp.interact_pointer_pos().is_some_and(|q| arrow.contains(q));
            acts.push(if r.dir && on_arrow { Act::Toggle(r.path.clone()) } else { Act::Select(r.path.clone()) });
        }
        resp.context_menu(|ui| self.item_menu(ui, &r.path, r.dir, acts));
    }

    fn item_menu(&self, ui: &mut Ui, path: &Path, dir: bool, acts: &mut Vec<Act>) {
        if dir && ui.button(format!("{}   Open", icon::FOLDER_OPEN)).clicked() {
            acts.push(Act::Focus(path.into()));
        }
        if ui.button(format!("{}   Show in Explorer", icon::OPEN)).clicked() {
            acts.push(Act::Show(path.into(), dir));
        }
        if ui.button(format!("{}   Copy path", icon::COPY)).clicked() {
            acts.push(Act::Copy(path.display().to_string()));
        }
    }

    fn analysis_view(&self, ui: &mut Ui, p: &Pal, acts: &mut Vec<Act>) {
        let s = self.scan.as_ref().unwrap();
        self.header(ui, p, acts);
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.add_space(10.0);
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.columns(s.drives.len().max(1), |cols| {
                for (ui, d) in cols.iter_mut().zip(&s.drives) {
                    let resp = card(p).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        let used = d.total.saturating_sub(d.free);
                        let frac = used as f32 / d.total.max(1) as f32;
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(icon::DRIVE).size(26.0).color(p.accent));
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                ui.label(RichText::new(&d.label).font(semibold(15.0)).color(p.text));
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(d.root.trim_end_matches('\\')).color(p.weak).size(12.0));
                                    if let Some(c) = kind_color(p, d.kind) {
                                        ui.label(RichText::new(d.kind.name()).size(11.5).color(c).background_color(c.gamma_multiply(0.18)));
                                    }
                                });
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(RichText::new(format!("{:.0}%", frac * 100.0)).font(semibold(22.0)).color(p.text));
                            });
                        });
                        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::hover());
                        bar(ui, rect, frac, if frac > 0.9 { p.warn } else { p.accent }, p.track);
                        ui.label(RichText::new(format!("{} used · {} free · {} total", human(used), human(d.free), human(d.total))).size(12.0).color(p.weak));
                        let seen = s.root.dirs.iter().find(|x| x.name == d.root).map_or(0, |x| x.local);
                        let hidden = used.saturating_sub(seen);
                        if !d.scanned() {
                            ui.label(RichText::new(format!("{} drive · not scanned, opens in Explorer", d.kind.name())).size(12.0).color(p.weak));
                        } else if hidden > 1 << 30 {
                            ui.label(RichText::new(format!("{} is system or protected data", human(hidden))).size(12.0).color(p.weak))
                                .on_hover_text("Used space WAMS can't read without admin rights: other users, System Volume Information, shadow copies, hibernation, and similar.");
                        }
                    });
                    let resp = ui.interact(resp.response.rect, ui.id().with(("drive", &d.root)), Sense::click());
                    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        acts.push(open_drive(d));
                    }
                }
            });
            ui.add_space(14.0);
            let title = if self.focus.as_os_str().is_empty() { "This PC".into() } else { display_name(&self.focus) };
            card(p).show(ui, |ui| {
                ui.label(RichText::new(format!("What's inside {title}")).font(semibold(16.0)).color(p.text));
                ui.label(RichText::new("Bigger box = more space. Click a folder to go into it.").size(12.0).color(p.weak));
                ui.add_space(8.0);
                self.treemap(ui, p, acts);
            });
            ui.add_space(14.0);
            ui.columns(2, |c| {
                card(p).show(&mut c[0], |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(RichText::new("Largest files").font(semibold(16.0)).color(p.text));
                    ui.add_space(6.0);
                    for (size, path) in s.big.iter().take(12) {
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                        if resp.hovered() || self.selected.as_ref() == Some(path) {
                            ui.painter().rect_filled(rect, 6.0, p.hover);
                        }
                        text(ui, pos2(rect.left() + 8.0, rect.center().y), Align2::LEFT_CENTER, icon::FILE, FontId::proportional(16.0), p.weak);
                        let clip = Rect::from_x_y_ranges(rect.left() + 34.0..=rect.right() - 90.0, rect.y_range());
                        let pt = ui.painter().with_clip_rect(clip);
                        pt.text(pos2(rect.left() + 34.0, rect.top() + 13.0), Align2::LEFT_CENTER, display_name(path), FontId::proportional(13.5), p.text);
                        let parent = path.parent().map(|x| x.display().to_string()).unwrap_or_default();
                        pt.text(pos2(rect.left() + 34.0, rect.top() + 29.0), Align2::LEFT_CENTER, parent, FontId::proportional(11.5), p.weak);
                        text(ui, pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, &human(*size), FontId::proportional(13.5), p.text);
                        if resp.double_clicked() {
                            acts.push(Act::Show(path.clone(), false));
                        } else if resp.clicked() {
                            acts.push(Act::Select(path.clone()));
                        }
                        resp.context_menu(|ui| self.item_menu(ui, path, false, acts));
                    }
                    if s.big.is_empty() {
                        ui.label(RichText::new("No files over 64 MB").color(p.weak));
                    }
                });
                card(p).show(&mut c[1], |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(RichText::new("File types").font(semibold(16.0)).color(p.text));
                    ui.add_space(6.0);
                    let top = s.exts.first().map_or(1, |e| e.1.max(1));
                    for (i, (ext, bytes, n)) in s.exts.iter().take(12).enumerate() {
                        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
                        let label = if ext.is_empty() { "No extension".to_string() } else { format!(".{ext}") };
                        let y = rect.top() + 13.0;
                        text(ui, pos2(rect.left() + 4.0, y), Align2::LEFT_CENTER, &label, FontId::proportional(13.5), p.text);
                        text(ui, pos2(rect.right() - 4.0, y), Align2::RIGHT_CENTER, &format!("{}  ·  {} files", human(*bytes), num(*n)), FontId::proportional(12.5), p.weak);
                        let b = Rect::from_min_size(pos2(rect.left() + 4.0, rect.top() + 26.0), vec2(rect.width() - 8.0, 5.0));
                        bar(ui, b, *bytes as f32 / top as f32, TILE_COLORS[i % TILE_COLORS.len()], p.track);
                    }
                });
            });
        });
    }

    fn treemap(&self, ui: &mut Ui, p: &Pal, acts: &mut Vec<Act>) {
        let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), 380.0), Sense::hover());
        let kids: Vec<&Row> = self.rows.iter().filter(|r| r.depth == 0.0 && r.size > 0).take(400).collect();
        if kids.is_empty() {
            text(ui, area.center(), Align2::CENTER_CENTER, "Nothing here", FontId::proportional(14.0), p.weak);
            return;
        }
        let rects = squarify(&kids.iter().map(|r| r.size as f64).collect::<Vec<_>>(), area);
        for (i, (r, rect)) in kids.iter().zip(rects).enumerate() {
            if !rect.is_positive() {
                continue;
            }
            let tile = rect.shrink(1.5);
            let resp = ui.interact(tile, ui.id().with(("tile", i)), Sense::click());
            let base = if r.dir { TILE_COLORS[i % TILE_COLORS.len()] } else { Color32::from_rgb(120, 125, 135) };
            let fill = if resp.hovered() { base.lerp_to_gamma(Color32::WHITE, 0.2) } else { base };
            ui.painter().rect_filled(tile, 4.0, fill);
            if tile.width() > 70.0 && tile.height() > 36.0 {
                let pt = ui.painter().with_clip_rect(tile.shrink(6.0));
                pt.text(tile.left_top() + vec2(8.0, 8.0), Align2::LEFT_TOP, &r.name, semibold(13.0), Color32::WHITE);
                pt.text(tile.left_top() + vec2(8.0, 26.0), Align2::LEFT_TOP, human(r.size), FontId::proportional(12.0), Color32::from_white_alpha(210));
            }
            let share = r.size as f32 / r.parent.max(1) as f32 * 100.0;
            let resp = resp.on_hover_text(format!("{}\n{} · {share:.1}%", r.name, human(r.size))).on_hover_cursor(CursorIcon::PointingHand);
            if resp.clicked() {
                acts.push(if r.dir { Act::Focus(r.path.clone()) } else { Act::Select(r.path.clone()) });
            }
            resp.context_menu(|ui| self.item_menu(ui, &r.path, r.dir, acts));
        }
    }

    fn cloud_view(&self, ui: &mut Ui, p: &Pal, acts: &mut Vec<Act>) {
        let s = self.scan.as_ref().unwrap();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.heading("Cloud backup");
            ui.label(RichText::new("Anything inside a cloud sync folder is backed up. Everything else exists only on this PC.").color(p.weak));
            ui.add_space(14.0);
            if s.sync.is_empty() {
                card(p).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(RichText::new(format!("{}  No cloud sync folders found", icon::WARN)).font(semibold(15.0)).color(p.warn));
                    ui.label(RichText::new("Sign in to OneDrive, Google Drive, Dropbox or iCloud to back up your files.").color(p.weak));
                });
                ui.add_space(14.0);
            }
            let home = s.known.iter().find(|k| k.0 == "Home").map(|k| k.1.clone()).unwrap_or_default();
            let size_of = |path: &Path| s.find(path).map_or((0, 0), |d| (d.size, d.local));
            let backed: u64 = s.sync.iter().map(|r| size_of(&r.path).0).sum();
            let online: u64 = s.sync.iter().map(|r| size_of(&r.path)).map(|(a, b)| a.saturating_sub(b)).sum();
            let home_synced: u64 = s.sync.iter().filter(|r| r.path.starts_with(&home)).map(|r| size_of(&r.path).0).sum();
            let unprotected = size_of(&home).0.saturating_sub(home_synced);
            ui.columns(3, |c| {
                for (ui, (icon, color, value, label)) in c.iter_mut().zip([
                    (icon::CHECK, p.ok, backed, "Backed up to the cloud"),
                    (icon::CLOUD, p.accent, online, "Online-only, not using space here"),
                    (icon::WARN, p.warn, unprotected, "Home folder, not backed up"),
                ]) {
                    card(p).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(RichText::new(icon).size(20.0).color(color));
                        ui.label(RichText::new(human(value)).font(semibold(24.0)).color(p.text));
                        ui.add(egui::Label::new(RichText::new(label).size(12.5).color(p.weak)).truncate());
                    });
                }
            });
            if !s.sync.is_empty() {
                ui.add_space(14.0);
                card(p).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(RichText::new("Sync folders").font(semibold(16.0)).color(p.text));
                    ui.add_space(6.0);
                    for r in &s.sync {
                        let (size, local) = size_of(&r.path);
                        let sub = format!("{} on this PC · {} online-only", human(local), human(size.saturating_sub(local)));
                        self.folder_row(ui, p, icon::CLOUD, p.accent, &r.provider, &r.path, size, (&sub, p.weak), acts);
                    }
                });
            }
            ui.add_space(14.0);
            card(p).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new("Your folders").font(semibold(16.0)).color(p.text));
                ui.label(RichText::new("Known folders and everything in your home folder, biggest first.").size(12.0).color(p.weak));
                ui.add_space(6.0);
                let mut seen = HashSet::new();
                let mut items: Vec<(String, PathBuf, &str)> = vec![];
                for (label, path) in s.known.iter().filter(|k| k.0 != "Home") {
                    if seen.insert(path.to_string_lossy().to_lowercase()) {
                        items.push((label.to_string(), path.clone(), known_icon(label)));
                    }
                }
                if let Some(h) = s.find(&home) {
                    for d in &h.dirs {
                        let path = home.join(&d.name);
                        if seen.insert(path.to_string_lossy().to_lowercase()) {
                            items.push((d.name.clone(), path, icon::FOLDER));
                        }
                    }
                }
                items.sort_by_key(|i| std::cmp::Reverse(size_of(&i.1).0));
                for (name, path, ic) in &items {
                    let status = match s.synced(path) {
                        Some(r) => (format!("Backed up · {}", r.provider), p.ok),
                        None => ("Not backed up".to_string(), p.warn),
                    };
                    self.folder_row(ui, p, ic, FOLDER_GREY, name, path, size_of(path).0, (&status.0, status.1), acts);
                }
            });
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn folder_row(&self, ui: &mut Ui, p: &Pal, ic: &str, tint: Color32, name: &str, path: &Path, size: u64, status: (&str, Color32), acts: &mut Vec<Act>) {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{name}, {}, {}", human(size), status.0)));
        if resp.hovered() {
            ui.painter().rect_filled(rect, 6.0, p.hover);
        }
        let (l, r) = (rect.left() + 10.0, rect.right() - 10.0);
        text(ui, pos2(l, rect.center().y), Align2::LEFT_CENTER, ic, FontId::proportional(18.0), tint);
        let clip = ui.painter().with_clip_rect(Rect::from_x_y_ranges(l + 34.0..=r - 330.0, rect.y_range()));
        clip.text(pos2(l + 34.0, rect.top() + 15.0), Align2::LEFT_CENTER, name, FontId::proportional(14.0), p.text);
        clip.text(pos2(l + 34.0, rect.top() + 32.0), Align2::LEFT_CENTER, path.display().to_string(), FontId::proportional(11.5), p.weak);
        text(ui, pos2(r - 230.0, rect.center().y), Align2::RIGHT_CENTER, &human(size), FontId::proportional(14.0), p.text);
        pill(ui, pos2(r - 210.0, rect.center().y), status.0, status.1);
        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            acts.push(Act::Focus(path.into()));
            acts.push(Act::View(View::Explorer));
        }
    }

    fn details(&self, ui: &mut Ui, p: &Pal, acts: &mut Vec<Act>) {
        let Some(s) = &self.scan else { return };
        let target = self.selected.clone().unwrap_or_else(|| self.focus.clone());
        let dir = s.find(&target);
        let pc = target.as_os_str().is_empty();
        let meta = if dir.is_none() { std::fs::metadata(&target).ok() } else { None };
        let (size, local, kind_icon, tint) = match (dir, &meta) {
            (Some(d), _) if pc => (d.size, d.local, icon::PC, p.accent),
            (Some(d), _) if s.drive(&target).is_some() => (d.size, d.local, icon::DRIVE, p.accent),
            (Some(d), _) => (d.size, d.local, icon::FOLDER, FOLDER_GREY),
            (None, Some(m)) => (m.len(), if scan::cloud_only(m) { 0 } else { m.len() }, icon::FILE, p.weak),
            _ => {
                ui.label(RichText::new("Select a file or folder to see details.").color(p.weak));
                return;
            }
        };
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.add_space(8.0);
            ui.label(RichText::new(kind_icon).size(44.0).color(tint));
            ui.add_space(4.0);
            let drive = s.drive(&target);
            let name = match drive {
                _ if pc => "This PC".into(),
                Some(d) => drive_name(d),
                None => display_name(&target),
            };
            ui.add(egui::Label::new(RichText::new(name).font(semibold(20.0)).color(p.text)).wrap());
            if !pc {
                ui.add(egui::Label::new(RichText::new(target.display().to_string()).size(12.0).color(p.weak)).wrap());
                ui.add_space(6.0);
                match s.synced(&target) {
                    Some(r) => badge(ui, &format!("{}  Backed up · {}", icon::CHECK, r.provider), p.ok),
                    None => badge(ui, &format!("{}  Not backed up", icon::WARN), p.warn),
                }
            }
            ui.add_space(12.0);
            card(p).inner_margin(12.0).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                let prop = |ui: &mut Ui, k: &str, v: String| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(k).color(p.weak));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(RichText::new(v).color(p.text)));
                    });
                };
                if let Some(d) = drive {
                    prop(ui, "Type", format!("{} drive", d.kind.name()));
                    prop(ui, "Capacity", human(d.total));
                    prop(ui, "Free", human(d.free));
                }
                prop(ui, "Size", human(size));
                prop(ui, "On this PC", human(local));
                if size > local {
                    prop(ui, "Online-only", human(size - local));
                }
                if let Some(d) = dir {
                    prop(ui, "Files", num(d.files));
                    prop(ui, "Folders", num(d.dirs.len() as u64));
                }
                if let Some(parent) = target.parent().and_then(|x| s.find(x)).filter(|_| !pc) {
                    prop(ui, "Share of parent", format!("{:.1}%", size as f64 / parent.size.max(1) as f64 * 100.0));
                }
                if let Some(m) = meta.as_ref().and_then(|m| m.modified().ok()) {
                    prop(ui, "Modified", ago(m));
                }
            });
            if let Some(d) = dir.filter(|d| !d.dirs.is_empty()) {
                ui.add_space(12.0);
                ui.label(RichText::new("Largest folders inside").font(semibold(14.0)).color(p.text));
                ui.add_space(4.0);
                for c in d.dirs.iter().take(6) {
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, 6.0, p.hover);
                    }
                    let clip = ui.painter().with_clip_rect(Rect::from_x_y_ranges(rect.left()..=rect.right() - 80.0, rect.y_range()));
                    clip.text(pos2(rect.left() + 6.0, rect.top() + 12.0), Align2::LEFT_CENTER, match s.drive(Path::new(&c.name)) { Some(dr) => format!("{}  {}", icon::DRIVE, drive_name(dr)), None => format!("{}  {}", icon::FOLDER, c.name) }, FontId::proportional(13.0), p.text);
                    text(ui, pos2(rect.right() - 6.0, rect.top() + 12.0), Align2::RIGHT_CENTER, &human(c.size), FontId::proportional(12.5), p.weak);
                    let b = Rect::from_min_size(pos2(rect.left() + 6.0, rect.top() + 26.0), vec2(rect.width() - 12.0, 4.0));
                    bar(ui, b, c.size as f32 / d.size.max(1) as f32, p.accent, p.track);
                    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        acts.push(Act::Focus(target.join(&c.name)));
                    }
                }
            }
            if !pc {
                ui.add_space(12.0);
                let w = vec2(ui.available_width(), 32.0);
                if dir.is_some() && ui.add(egui::Button::new(format!("{}   Open", icon::FOLDER_OPEN)).min_size(w)).clicked() {
                    acts.push(Act::Focus(target.clone()));
                }
                if ui.add(egui::Button::new(format!("{}   Show in Explorer", icon::OPEN)).min_size(w)).clicked() {
                    acts.push(Act::Show(target.clone(), dir.is_some()));
                }
                if ui.add(egui::Button::new(format!("{}   Copy path", icon::COPY)).min_size(w)).clicked() {
                    acts.push(Act::Copy(target.display().to_string()));
                }
            }
        });
    }

    fn loading(&self, ui: &mut Ui, p: &Pal) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() / 2.0 - 60.0);
            ui.add(egui::Spinner::new().size(36.0).color(p.accent));
            ui.add_space(12.0);
            ui.label(RichText::new("Scanning your drives…").font(semibold(18.0)).color(p.text));
            ui.label(RichText::new(format!("{} files so far", num(self.count.load(Ordering::Relaxed)))).color(p.weak));
        });
    }
}

fn ago(t: std::time::SystemTime) -> String {
    let s = t.elapsed().map_or(0, |d| d.as_secs());
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86400 => format!("{} h ago", s / 3600),
        86400..2_592_000 => format!("{} days ago", s / 86400),
        2_592_000..31_536_000 => format!("{} months ago", s / 2_592_000),
        _ => format!("{} years ago", s / 31_536_000),
    }
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut Ui, _: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        if let Some(Ok(s)) = self.rx.as_ref().map(|rx| rx.try_recv()) {
            self.rx = None;
            if s.find(&self.focus).is_none() {
                self.focus = PathBuf::new(); // focused folder vanished after rescan
            }
            self.scan = Some(s);
            self.files.clear(); // expanded folders stay expanded
            self.dirty = true;
        }
        if self.dirty {
            self.dirty = false;
            self.rows.clear();
            if let Some(s) = &self.scan {
                if let Some(d) = s.find(&self.focus) {
                    walk(d, &self.focus, 0.0, &self.open, &mut self.files, s, &mut self.rows);
                }
            }
        }
        if self.rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }

        let p = pal(ctx.theme() == egui::Theme::Dark, self.accent);
        let mut acts = vec![];
        let side = |m: i8| egui::Frame::new().fill(p.nav).inner_margin(Margin::same(m));
        egui::Panel::left("nav").exact_size(252.0).resizable(false).show_separator_line(false).frame(side(12)).show(root, |ui| self.nav(ui, &p, &mut acts));
        egui::Panel::right("details").exact_size(300.0).resizable(false).show_separator_line(false).frame(side(16)).show(root, |ui| self.details(ui, &p, &mut acts));
        egui::CentralPanel::default().frame(egui::Frame::new().fill(p.content)).show(root, |ui| {
            egui::Frame::new().inner_margin(16.0).show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                match (&self.scan, self.view) {
                    (None, _) => self.loading(ui, &p),
                    (_, View::Explorer) => self.explorer_view(ui, &p, &mut acts),
                    (_, View::Analysis) => self.analysis_view(ui, &p, &mut acts),
                    (_, View::Cloud) => self.cloud_view(ui, &p, &mut acts),
                }
            });
        });

        for a in acts {
            match a {
                Act::Focus(path) => {
                    if self.scan.as_ref().is_some_and(|s| s.find(&path).is_some()) {
                        self.focus = path;
                        self.selected = None;
                        self.dirty = true;
                        if self.view == View::Cloud {
                            self.view = View::Explorer;
                        }
                    }
                }
                Act::Toggle(path) => {
                    if !self.open.remove(&path) {
                        self.open.insert(path);
                    }
                    self.dirty = true;
                }
                Act::Select(path) => self.selected = Some(path),
                Act::View(v) => self.view = v,
                Act::Refresh if self.rx.is_none() => self.refresh(),
                Act::Refresh => {}
                Act::Show(path, dir) => show_in_explorer(&path, dir),
                Act::Copy(s) => ctx.copy_text(s),
            }
        }
    }
}

#[test]
fn squarify_fills_rect() {
    let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
    let rects = squarify(&[50.0, 25.0, 12.0, 8.0, 5.0], area);
    let sum: f32 = rects.iter().map(|r| r.area()).sum();
    assert!((sum - area.area()).abs() < 1.0, "{sum}");
    assert!(rects.iter().all(|r| area.expand(0.01).contains_rect(*r)));
    assert!((rects[0].area() / area.area() - 0.5).abs() < 0.01);
    assert_eq!(num(1234567), "1,234,567");
    assert_eq!(human(1536), "1.5 KB");
}
