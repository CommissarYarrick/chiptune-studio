@echo off
setlocal
if not defined CHIPTUNE_BIN set "CHIPTUNE_BIN=%~dp0target\release\chip.exe"
if not exist "%CHIPTUNE_BIN%" (
    echo Native Chiptune CLI not found: "%CHIPTUNE_BIN%" 1>&2
    exit /b 2
)
"%CHIPTUNE_BIN%" %*
exit /b %errorlevel%