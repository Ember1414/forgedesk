@echo off
REM ============================================================
REM ForgeDesk - Install / repair the Rust stable toolchain
REM
REM ASCII ONLY. cmd.exe parses .cmd files with the OEM codepage
REM (GBK on zh-CN Windows); UTF-8 CJK comments get mangled into
REM garbage tokens and executed as commands.
REM
REM Why a separate script from windows-toolchain.cmd:
REM   The plain `rustup-init -y` route can end up with a partially
REM   installed toolchain ("missing manifest in toolchain") when the
REM   download from static.rust-lang.org is interrupted. This script
REM   removes the broken toolchain and reinstalls it explicitly.
REM
REM Usage:
REM   scripts\setup\rust-toolchain.cmd
REM ============================================================

setlocal

REM Clear NODE_OPTIONS: the injected safe-delete shim breaks file
REM removal inside cargo/rustup operations.
set NODE_OPTIONS=

REM Ensure cargo/rustup are reachable in this session.
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
set "RUSTUP=%USERPROFILE%\.cargo\bin\rustup.exe"

REM ---- Mirror (mainland China) -------------------------------
REM Comment out the next two lines if you are outside mainland China
REM or already have fast access to static.rust-lang.org.
REM Alternatives: https://mirrors.tuna.tsinghua.edu.cn/rustup
REM               https://mirrors.ustc.edu.cn/rust-static
set "RUSTUP_DIST_SERVER=https://rsproxy.cn"
set "RUSTUP_UPDATE_ROOT=https://rsproxy.cn/rustup"
REM ------------------------------------------------------------

echo ============================================================
echo [1/3] Removing broken toolchain (safe if none exists)
echo ============================================================
"%RUSTUP%" toolchain uninstall stable
echo [1/3] exit code = %ERRORLEVEL%
echo.

echo ============================================================
echo [2/3] Installing stable toolchain (rustc, cargo, rustfmt, clippy)
echo       Mirrored via: %RUSTUP_DIST_SERVER%
echo ============================================================
"%RUSTUP%" toolchain install stable --profile default --no-self-update
echo [2/3] exit code = %ERRORLEVEL%
echo.

echo ============================================================
echo [3/3] Setting stable as default and verifying
echo ============================================================
"%RUSTUP%" default stable
echo [3/3] exit code = %ERRORLEVEL%
echo.
rustc --version
cargo --version
cargo fmt --version
cargo clippy --version
echo.
echo ============================================================
echo Toolchain setup finished. Expected versions:
echo   rustc / cargo / rustfmt / clippy all reporting a stable version
echo ============================================================

endlocal
