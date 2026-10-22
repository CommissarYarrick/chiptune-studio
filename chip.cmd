@echo off
set "PYTHON=%~dp0.venv\Scripts\python.exe"
if not exist "%PYTHON%" (
    echo Chiptune Python environment not found. Create it with: python -m venv .venv 1>&2
    exit /b 2
)
"%PYTHON%" "%~dp0chip.py" %*
