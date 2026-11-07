@echo off
setlocal
cd /d "%~dp0"

echo [1/2] Setting up MSVC x64 environment...
if exist "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" (
    call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
) else if exist "C:\Program Files (x86)\Microsoft Visual Studio\2019\Community\VC\Auxiliary\Build\vcvars64.bat" (
    call "C:\Program Files (x86)\Microsoft Visual Studio\2019\Community\VC\Auxiliary\Build\vcvars64.bat"
)

echo [2/2] Compiling install_helper.c...
cl /utf-8 /O2 /MT /GS- /Fe:install_helper.exe install_helper.c shell32.lib ole32.lib advapi32.lib /link /SUBSYSTEM:WINDOWS

if exist install_helper.obj del install_helper.obj
echo Compilation finished: %~dp0install_helper.exe
endlocal
