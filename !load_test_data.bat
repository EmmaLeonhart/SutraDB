@echo off
echo Loading stress test data into SutraDB (port 3030)...
echo Make sure SutraDB is running first: !serve.bat
python "%~dp0stress_test.py"
pause
