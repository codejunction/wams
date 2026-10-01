# WAMS: Who Ate My Storage

A small native Windows app (Rust + egui, ~5 MB) that shows the size of every folder and file on all your drives, laid out as an expandable tree with breadcrumbs.

![screenshot](screenshot.png)

## Run on any Windows PC

Paste into PowerShell. No admin rights needed:

```powershell
irm https://github.com/codejunction/wams/releases/latest/download/wams.exe -OutFile $env:TEMP\wams.exe; & $env:TEMP\wams.exe
```

The first launch installs WAMS for the current user:

- copies itself to `%LOCALAPPDATA%\Programs\wams\wams.exe`
- registers itself to start at login (`HKCU\...\Run`), so a fresh scan is ready after every boot
- adds itself to **Settings > Apps** so you can uninstall it there

To try it without installing, run `wams.exe --portable`. To uninstall from the command line, run `wams.exe --uninstall`.

## Usage

- **Click** a folder to expand or collapse it.
- **Double-click** a folder to open it; the breadcrumb bar shows where you are. Click a breadcrumb to go back up.
- **Double-click** a file, or **right-click** any row, to show it in Windows Explorer.
- **Refresh** rescans all drives.

## Notes

- Sizes are logical file sizes. Symlinks and junctions are skipped, so nothing is counted twice.
- Without admin rights, some protected system folders can't be read, so their totals come out a little low.
- Every drive letter is scanned, including USB and network drives.

## Build

```sh
cargo build --release   # target/release/wams.exe
cargo test
cargo run -- --portable # run without installing
```
