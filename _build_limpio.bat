@echo off
REM ===============================================================
REM Archivo:     _build_limpio.bat
REM Proyecto:    Clasificador de repositorios
REM Autor:       Jorge Mera
REM Fecha:       2026-05-04
REM Version:     1.0.0
REM Descripcion: Build LIMPIO desde cero. Usar cuando el binario
REM              quedo desactualizado pese a que cargo build dice
REM              "Finished" (fingerprint inconsistente).
REM
REM              Pasos:
REM                1. Cierra clasificador.exe si esta abierto
REM                   (sino el copy /Y falla con sharing violation)
REM                2. cargo clean (borra target\ ~2-3 GB)
REM                3. cargo build --release (compila desde cero)
REM                4. Copia el .exe a la raiz
REM                5. Regenera buscador.html via CLI
REM
REM              Uso normal: _build_definitivo.bat
REM              Uso de emergencia (este): _build_limpio.bat
REM ===============================================================

setlocal enabledelayedexpansion

set "RS=%~dp0"
if "%RS:~-1%"=="\" set "RS=%RS:~0,-1%"
for %%I in ("%RS%\..") do set "ROOT=%%~fI"

echo ============================================================
echo   BUILD LIMPIO ^(desde cero^)
echo ============================================================
echo   RS    = %RS%
echo   ROOT  = %ROOT%
echo.

REM === 1. Cerrar clasificador.exe si esta abierto ===========================
REM    Windows bloquea sobreescribir un .exe que tiene un proceso encima.
REM    taskkill /F mata sin preguntar; /IM filtra por nombre del .exe.
echo [1/5] Cerrando clasificador.exe si esta abierto...
tasklist /FI "IMAGENAME eq clasificador.exe" 2>nul | find /I "clasificador.exe" >nul
if %errorlevel%==0 (
  taskkill /F /IM clasificador.exe >nul 2>&1
  echo       Proceso terminado.
) else (
  echo       No estaba corriendo.
)

REM === 2. cargo clean ========================================================
echo.
echo [2/5] cargo clean ^(borrando target\ ~2-3 GB^)...
cd /d "%RS%" || goto :err
cargo clean
if errorlevel 1 goto :err

REM === 3. cargo build --release ==============================================
echo.
echo [3/5] cargo build --release ^(va a tardar 1-3 minutos^)...
cargo build --release
if errorlevel 1 goto :err

REM Verificar que el binario realmente se genero.
if not exist "%RS%\target\release\clasificador.exe" (
  echo ERROR: target\release\clasificador.exe NO existe despues del build.
  goto :err
)

REM === 4. Copiar a raiz ======================================================
echo.
echo [4/5] Copiando clasificador.exe a la carpeta padre...
copy /Y "%RS%\target\release\clasificador.exe" "%ROOT%\clasificador.exe" || goto :err

REM === 5. Regenerar buscador.html ============================================
echo.
echo [5/5] Regenerando buscador.html y data\repos_index.json...
"%RS%\target\release\clasificador-cli.exe" --root "%ROOT%" --no-network
if errorlevel 1 goto :err

echo.
echo ============================================================
echo   RESULTADO FINAL
echo ============================================================
echo Binario en raiz:
dir "%ROOT%\clasificador.exe" | findstr /I "clasificador.exe"
echo.
echo data\:
dir /B "%ROOT%\data\repo_ids.json" "%ROOT%\data\repos_index.json"
echo.
echo ============================================================
echo   BUILD LIMPIO OK
echo ============================================================
echo   Ahora abri clasificador.exe y pulsa "Refrescar GitHub IDs".
echo   Verifica que data\repo_ids.json quede de ~80-100 KB.
echo ============================================================
pause
exit /b 0

:err
echo.
echo ============================================================
echo   ERROR -- revisa la salida anterior
echo ============================================================
pause
exit /b 1
