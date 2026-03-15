@echo off
echo Opening SutraDB Browser...
echo Make sure SutraDB is running: sutra serve --port 3030
start "" "%~dp0tools\browse.html"
