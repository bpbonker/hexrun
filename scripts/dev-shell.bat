@echo off
REM Initialize MSVC ARM64 environment and pass through to whatever follows on the command line.
REM Usage:  scripts\dev-shell.bat cargo check --workspace
setlocal
REM Prepend toolchain dirs. The VS Installer dir is needed because vcvarsall.bat
REM internally calls `vswhere.exe` without a fully-qualified path.
set "PATH=%USERPROFILE%\.cargo\bin;C:\Program Files\LLVM\bin;C:\Program Files (x86)\Microsoft Visual Studio\Installer;%PATH%"
set "LIBCLANG_PATH=C:\Program Files\LLVM\bin"
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvarsall.bat" arm64 >nul 2>&1
if errorlevel 1 (
    echo [dev-shell] vcvarsall.bat failed with errorlevel %errorlevel%
    exit /b %errorlevel%
)
REM If QAIRT is configured, prepend its bin/lib dirs so any exe launched here
REM (cargo-run examples, hexrun, integration tests) can locate Genie.dll,
REM QnnHtp.dll, and the Hexagon stub libraries at process startup.
if defined QNN_SDK_ROOT (
    set "PATH=%QNN_SDK_ROOT%\bin\aarch64-windows-msvc;%QNN_SDK_ROOT%\lib\aarch64-windows-msvc;%PATH%"
    if not defined ADSP_LIBRARY_PATH set "ADSP_LIBRARY_PATH=%QNN_SDK_ROOT%\lib\hexagon-v73\unsigned"
)
%*
