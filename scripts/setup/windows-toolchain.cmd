@echo off
REM ============================================================
REM ForgeDesk - Windows dev toolchain installer (all free, no account)
REM
REM ASCII ONLY. cmd.exe parses .cmd files with the OEM codepage
REM (GBK on zh-CN Windows); UTF-8 CJK comments get mangled into
REM garbage tokens and executed as commands.
REM
REM This script is an ORCHESTRATOR. It runs two focused scripts so
REM that each step can be re-run independently when it fails, and so
REM that the two rustup-related pitfalls are avoided:
REM   - never run two rustup install commands concurrently (deadlock)
REM   - always use a mirror for the Rust toolchain in mainland China
REM
REM   scripts/setup/rust-toolchain.cmd   Rust stable (+ rustfmt, clippy)
REM                                      no admin required, mirrored
REM   scripts/setup/msvc-buildtools.cmd  MSVC C++ Build Tools
REM                                      NEEDS ADMIN, approx 4-7 GB
REM
REM Usage (run from an ELEVATED Command Prompt so step 2 can proceed):
REM   scripts\setup\windows-toolchain.cmd
REM
REM Already-present components are skipped automatically by winget/rustup.
REM See docs/DEV-ENV.md for the full explanation and verification steps.
REM ============================================================

setlocal

set "SCRIPT_DIR=%~dp0"

echo ############################################################
echo # ForgeDesk Windows toolchain setup
echo ############################################################
echo.

echo ############################################################
echo # STEP 1 of 2: Rust toolchain
echo ############################################################
call "%SCRIPT_DIR%rust-toolchain.cmd"
echo STEP 1 exit code = %ERRORLEVEL%
echo.

echo ############################################################
echo # STEP 2 of 2: MSVC C++ Build Tools
echo ############################################################
call "%SCRIPT_DIR%msvc-buildtools.cmd"
echo STEP 2 exit code = %ERRORLEVEL%
echo.

echo ############################################################
echo # Verifying the toolchain
echo ############################################################
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
rustc --version
cargo --version
cargo fmt --version
cargo clippy --version
where link.exe
where cl.exe

echo.
echo If link.exe was not found, re-run this script from an elevated
echo Command Prompt. See docs/DEV-ENV.md section 6.

endlocal
