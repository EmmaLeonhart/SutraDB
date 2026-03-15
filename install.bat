@echo off
REM Install SutraDB CLI to user's local bin directory.
REM Requires Rust toolchain (cargo) to be installed.

setlocal

echo Building SutraDB (release)...
cargo build --release -p sutra-cli
if errorlevel 1 (
    echo Build failed!
    exit /b 1
)

REM Create install directory if needed
set INSTALL_DIR=%USERPROFILE%\.sutra\bin
if not exist "%INSTALL_DIR%" mkdir "%INSTALL_DIR%"

echo Installing to %INSTALL_DIR%\sutra.exe ...
copy /y target\release\sutra.exe "%INSTALL_DIR%\"
if errorlevel 1 (
    echo Install failed!
    exit /b 1
)

echo.
echo Done! Add %INSTALL_DIR% to your PATH if not already there:
echo   set PATH=%INSTALL_DIR%;%%PATH%%
echo.
echo Usage:
echo   sutra serve                    Start the HTTP server
echo   sutra query "SELECT ..."       Run a SPARQL query
echo   sutra import data.nt           Import N-Triples file
echo   sutra export -o dump.nt        Export all triples
echo   sutra info                     Show database statistics
echo.
