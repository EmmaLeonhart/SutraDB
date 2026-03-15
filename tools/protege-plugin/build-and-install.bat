@echo off
REM Build the SutraDB Protege plugin and install it.
REM Usage: build-and-install.bat [PROTEGE_DIR]
REM Default PROTEGE_DIR: C:\Users\Immanuelle\Desktop\Protege-5.6.7

setlocal

set PROTEGE_DIR=%~1
if "%PROTEGE_DIR%"=="" set PROTEGE_DIR=C:\Users\Immanuelle\Desktop\Protege-5.6.7

echo Building SutraDB Protege plugin...
cd /d "%~dp0"
call mvn package -q
if errorlevel 1 (
    echo Build failed!
    exit /b 1
)

echo Installing to %PROTEGE_DIR%\plugins\ ...
copy /y target\sutradb-protege-plugin-0.1.0.jar "%PROTEGE_DIR%\plugins\"
if errorlevel 1 (
    echo Install failed! Is Protege directory correct?
    exit /b 1
)

echo.
echo Done! Restart Protege, then go to Window ^> Tabs ^> SutraDB
echo.
