@echo off
echo Starting SutraDB on port 3030...
echo Press Ctrl+C to stop.
"%~dp0target\release\sutra.exe" serve --port 3030
pause
