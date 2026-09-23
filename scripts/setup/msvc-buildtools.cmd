@echo off
REM ============================================================
REM ForgeDesk - Install MSVC C++ Build Tools (required by Tauri on Windows)
REM
REM ASCII ONLY. See scripts/setup/rust-toolchain.cmd for why.
REM
REM REQUIRES ADMINISTRATOR PRIVILEGES.
REM If run without elevation, winget will fail with an elevation error
REM and you must re-run this script from an elevated Command Prompt
REM (right click -> Run as administrator).
REM
REM Size / duration: approx 4-7 GB, 10-30 minutes.
REM
REM Usage:
REM   scripts\setup\msvc-buildtools.cmd
REM ============================================================

setlocal

set NODE_OPTIONS=

echo ============================================================
echo Installing MSVC C++ Build Tools (Visual Studio 2022)
echo   Workload: Microsoft.VisualStudio.Workload.VCTools
echo   Includes: MSVC compiler, Windows SDK, CMake
echo ============================================================
echo.

winget install --id Microsoft.VisualStudio.2022.BuildTools -e ^
  --accept-package-agreements --accept-source-agreements --disable-interactivity ^
  --override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"

echo.
echo exit code = %ERRORLEVEL%
echo.
echo ============================================================
echo Done. CLOSE AND REOPEN your terminal, then verify:
echo   where link.exe
echo   where cl.exe
echo   rustc --version
echo.
echo If exit code is 0 but link.exe is still missing, the workload
echo may need a second pass - re-run this script.
echo ============================================================

endlocal
