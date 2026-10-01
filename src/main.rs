#![windows_subsystem = "windows"]
//! wams - who ate my storage. Folder/file sizes for every drive.

mod scan;
mod ui;

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\wams";

/// Per-user install: copy to %LOCALAPPDATA%\Programs\wams, run at login, list in Apps & features.
/// Returns true when this process should exit.
fn install() -> bool {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--portable") {
        return false;
    }
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let dir = PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join(r"Programs\wams");
    let target = dir.join("wams.exe");
    let t = target.display().to_string();
    if args.iter().any(|a| a == "--uninstall") {
        let _ = hkcu.open_subkey_with_flags(RUN_KEY, winreg::enums::KEY_SET_VALUE).and_then(|k| k.delete_value("wams"));
        let _ = hkcu.delete_subkey_all(UNINSTALL_KEY);
        let _ = Command::new("cmd")
            .raw_arg(format!("/c timeout /t 2 >nul & rmdir /s /q \"{}\"", dir.display()))
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
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
    if let Ok((k, _)) = hkcu.create_subkey(RUN_KEY) {
        let _ = k.set_value("wams", &quoted);
    }
    if let Ok((k, _)) = hkcu.create_subkey(UNINSTALL_KEY) {
        let _ = k.set_value("DisplayName", &"WAMS - Who Ate My Storage");
        let _ = k.set_value("DisplayIcon", &t);
        let _ = k.set_value("Publisher", &"codejunction");
        let _ = k.set_value("DisplayVersion", &env!("CARGO_PKG_VERSION"));
        let _ = k.set_value("UninstallString", &format!("{quoted} --uninstall"));
        let _ = k.set_value("NoModify", &1u32);
    }
    Command::new(&target).spawn().is_ok()
}

fn main() -> eframe::Result {
    if install() {
        return Ok(());
    }
    eframe::run_native(
        "WAMS - Who Ate My Storage",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size([1280.0, 800.0])
                .with_min_inner_size([900.0, 560.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(ui::App::new(&cc.egui_ctx)))),
    )
}
