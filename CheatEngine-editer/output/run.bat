@echo off
cd /d "%~dp0"
echo Loading Driver MyRoot_74BF...
kdu.exe -map MyRoot_74BF.sys
timeout /t 2
start "" arcdebugger.exe
