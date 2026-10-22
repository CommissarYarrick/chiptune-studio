@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0start_studio.ps1" -OpenBrowser %*
