@echo off
rem Builds grids.exe (release) and copies it next to this script.
rem Needs the Rust toolchain (rust-toolchain.toml pins the compiler) and the
rem Windows SDK: GPUI compiles its HLSL shaders with the SDK's fxc.exe.
setlocal
cd /d "%~dp0"

where cargo >nul 2>nul
if errorlevel 1 (
  echo build: cargo not found. Install Rust with rustup ^(https://rustup.rs^), then
  echo        open a new console. Rustup puts cargo in %%USERPROFILE%%\.cargo\bin.
  exit /b 1
)

rem GPUI's build script only knows the SDK version it was written against, so
rem point it at the newest installed one, as CI does. Left alone if the caller
rem already set it, or if no SDK is found (GPUI then says so itself).
if not defined GPUI_FXC_PATH (
  for /f "usebackq delims=" %%f in (`powershell -NoProfile -Command "$root = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'; $f = Get-ChildItem $root -Filter fxc.exe -Recurse -ErrorAction SilentlyContinue | Where-Object { $_.Directory.Name -eq 'x64' } | Sort-Object { [version]$_.Directory.Parent.Name } -Descending | Select-Object -First 1; if ($f) { $f.FullName }"`) do set "GPUI_FXC_PATH=%%f"
)

cargo build --release --locked
if errorlevel 1 exit /b 1

copy /y "target\release\grids.exe" "grids.exe" >nul
if errorlevel 1 (
  echo build: could not write grids.exe - close the running Grids and try again.
  exit /b 1
)

for %%a in ("grids.exe") do echo build: grids.exe %%~za bytes
