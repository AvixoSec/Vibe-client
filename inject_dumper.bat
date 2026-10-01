@echo off
setlocal EnableDelayedExpansion
title VIBE CLIENT - CLASS DUMPER INJECTOR

:: Check for administrative rights and elevate if needed
net session >nul 2>&1
if %errorlevel% neq 0 (
    echo [!] Requesting administrative privileges...
    powershell -NoProfile -ExecutionPolicy Bypass -Command "Start-Process cmd -ArgumentList '/c \"\"%~f0\"\"' -Verb RunAs"
    exit /b
)

cd /d "%~dp0"
echo ============================================================
echo   VIBE CLIENT - RUNTIME CLASS DUMPER v6 (DIRECT JNI DUMP)
echo   Target: RustMe Minecraft 1.12.2 / OpenJDK 21 / LWJGL 3.3.3
echo ============================================================
echo.

"%~dp0target\release\vibe_injector.exe" rustme.exe "%~dp0tools\vibe-dumper\target\release\vibe_dumper_v11.dll"

echo.
echo ============================================================
echo Dump logs and classes are saved in: d:\project\rustme\dump\
echo ============================================================
pause
