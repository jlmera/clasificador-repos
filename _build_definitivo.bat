@echo off
REM ===============================================================
REM Archivo:     _build_definitivo.bat
REM Proyecto:    Clasificador de repositorios (definitivo)
REM Autor:       Jorge Mera
REM Fecha:       2026-04-28
REM Version:     1.1.0
REM Descripcion: Compila clasificador.exe (Rust), promueve los
REM              artefactos a la carpeta padre y regenera
REM              buscador.html + data\repos_index.json.
REM
REM              Este .bat vive dentro de fuente/. Detecta sus
REM              propios paths a partir de %~dp0, asi que se
REM              puede mover el arbol completo a otro disco
REM              sin tocar el script.
REM ===============================================================

setlocal enabledelayedexpansion

REM %~dp0 = directorio del .bat con trailing backslash.
REM Le quitamos el backslash final para que no se duplique al concatenar.
set "RS=%~dp0"
if "%RS:~-1%"=="\" set "RS=%RS:~0,-1%"

REM ROOT = padre de fuente\, canonicalizado a path absoluto sin "..".
for %%I in ("%RS%\..") do set "ROOT=%%~fI"

echo ============================================================
echo   Paths detectados
echo ============================================================
echo   RS    = %RS%
echo   ROOT  = %ROOT%
echo.

echo ============================================================
echo   1. cargo build --release
echo ============================================================
cd /d "%RS%" || goto :err
cargo build --release
if errorlevel 1 goto :err

REM === Verificacion de fingerprint inconsistente (bug de cargo) ====
REM A veces cargo dice "Finished" pero NO regenera el binario aunque
REM los .rs son mas nuevos (target/ quedo en estado inconsistente, p.ej.
REM por una compilacion previa interrumpida). Si no detectamos esto, el
REM usuario corre un .exe obsoleto sin saberlo y los bugs ya arreglados
REM "siguen ahi". Comparamos LastWriteTime con PowerShell (independiente
REM del locale, a diferencia de xcopy/forfiles).
if not exist "%RS%\target\release\clasificador.exe" (
  echo ERROR: target\release\clasificador.exe no existe despues del build.
  goto :err
)
powershell -NoProfile -NonInteractive -Command ^
  "$exe = (Get-Item '%RS%\target\release\clasificador.exe').LastWriteTime;" ^
  "$src = (Get-ChildItem '%RS%\src' -Recurse -Filter *.rs | Sort-Object LastWriteTime -Descending | Select-Object -First 1).LastWriteTime;" ^
  "if ($exe -lt $src) { exit 7 } else { exit 0 }"
if errorlevel 7 (
  echo.
  echo ============================================================
  echo   ATENCION: cargo dice 'Finished' pero el binario quedo
  echo   mas viejo que algun .rs en src\. Esto significa que el
  echo   fingerprint de cargo esta inconsistente y NO recompilo.
  echo.
  echo   Solucion:
  echo     cd /d "%RS%"
  echo     cargo clean
  echo     _build_definitivo.bat
  echo.
  echo   Cargo clean borra target\ ^(~2-3 GB^) y la siguiente
  echo   compilacion va a tardar 1-3 minutos para reconstruir todo.
  echo ============================================================
  goto :err
)

echo.
echo ============================================================
echo   2. Copiar clasificador.exe a la carpeta padre
echo ============================================================
copy /Y "%RS%\target\release\clasificador.exe" "%ROOT%\clasificador.exe" || goto :err

echo.
echo ============================================================
echo   3. Eliminar binarios obsoletos en la raiz (idempotente)
echo ============================================================
if exist "%ROOT%\clasificadors.exe" (
  del /F /Q "%ROOT%\clasificadors.exe"
  echo   - clasificadors.exe   ELIMINADO
)
if exist "%ROOT%\clasificadopy.exe" (
  del /F /Q "%ROOT%\clasificadopy.exe"
  echo   - clasificadopy.exe   ELIMINADO
)

echo.
echo ============================================================
echo   4. Eliminar carpeta del proyecto Python (idempotente)
echo ============================================================
if exist "%ROOT%\clasificador\" (
  rmdir /S /Q "%ROOT%\clasificador"
  echo   - clasificador\       ELIMINADA
)

echo.
echo ============================================================
echo   5. Eliminar artefactos *_rs.* obsoletos (idempotente)
echo ============================================================
if exist "%ROOT%\buscador_rs.html" (
  del /F /Q "%ROOT%\buscador_rs.html"
  echo   - buscador_rs.html               ELIMINADO
)
if exist "%ROOT%\data\repos_index_rs.json" (
  del /F /Q "%ROOT%\data\repos_index_rs.json"
  echo   - data\repos_index_rs.json       ELIMINADO
)
if exist "%ROOT%\data\repo_ids_rs.json" (
  del /F /Q "%ROOT%\data\repo_ids_rs.json"
  echo   - data\repo_ids_rs.json          ELIMINADO
)

echo.
echo ============================================================
echo   6. Regenerar buscador.html y data\repos_index.json
echo ============================================================
"%RS%\target\release\clasificador-cli.exe" --root "%ROOT%" --no-network
if errorlevel 1 goto :err

echo.
echo ============================================================
echo   RESULTADO FINAL
echo ============================================================
echo Binarios en la raiz:
dir /B "%ROOT%\clasificador*.exe" 2>nul
echo.
echo HTMLs en la raiz:
dir /B "%ROOT%\buscador*.html" 2>nul
echo.
echo JSONs en data\:
dir /B "%ROOT%\data\repos_index*.json" 2>nul
dir /B "%ROOT%\data\repo_ids*.json"   2>nul
echo.
echo ============================================================
echo   BUILD OK
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

