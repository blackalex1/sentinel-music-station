@echo off
if "%1"=="--version" (
    echo 0.29.2
    exit /b 0
)
if "%1"=="--modversion" (
    echo 1.2.9
    exit /b 0
)
if "%1"=="--atleast-pkgconfig-version" (
    exit /b 0
)
if "%1"=="--atleast-version" (
    exit /b 0
)
set "LIBDIR=%~dp0..\libs\armv7"
set "LIBDIR=%LIBDIR:\=/%"
echo -L%LIBDIR% -lasound
exit /b 0
