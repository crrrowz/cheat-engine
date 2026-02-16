@echo off
cd /d "%~dp0"
echo Loading Driver MyRoot_756E...
kdu.exe -map MyRoot_756E.sys
timeout /t 2
start "" arcdebugger.exe
