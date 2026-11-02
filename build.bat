@echo off
REM =========================================================================
REM  Gerlink TelemetryAgent Build Script
REM  Compiles release binary and creates NSIS Windows installer
REM =========================================================================

echo [1/3] Building telemetry-agent (Rust Release)...
cargo build --release
if errorlevel 1 (
    echo [ERROR] Cargo build failed!
    exit /b 1
)

echo [2/3] Checking NSIS Compiler...
set MAKENSIS_CMD=""
where makensis >nul 2>&1
if not errorlevel 1 (
    set MAKENSIS_CMD=makensis
) else if exist "C:\Program Files (x86)\NSIS\makensis.exe" (
    set MAKENSIS_CMD="C:\Program Files (x86)\NSIS\makensis.exe"
) else if exist "C:\Program Files\NSIS\makensis.exe" (
    set MAKENSIS_CMD="C:\Program Files\NSIS\makensis.exe"
) else if exist "%LOCALAPPDATA%\Temp\nsis-3.13\nsis-3.13\makensis.exe" (
    set MAKENSIS_CMD="%LOCALAPPDATA%\Temp\nsis-3.13\nsis-3.13\makensis.exe"
) else (
    echo [ERROR] makensis.exe not found. Please install NSIS.
    exit /b 1
)

echo [3/3] Compiling NSIS Installer...
if not exist "target\installer" mkdir "target\installer"
%MAKENSIS_CMD% installer.nsi
if errorlevel 1 (
    echo [ERROR] NSIS compile failed!
    exit /b 1
)

echo.
echo =========================================================================
echo  BUILD SUCCESS!
echo  Installer created at:
echo  target\installer\Gerlink-TelemetryAgent-Setup-v1.2.0.exe
echo =========================================================================
pause
