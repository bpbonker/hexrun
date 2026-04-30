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
%*
