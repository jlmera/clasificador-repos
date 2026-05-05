@echo off
REM ===============================================================
REM Archivo:     _reset_descripciones_es.bat
REM Proyecto:    Clasificador de repositorios (definitivo)
REM Autor:       Jorge Mera
REM Fecha:       2026-04-29
REM Version:     1.1.0
REM Descripcion: Borra data\descripciones_es.json para que la
REM              proxima corrida de "Traducir READMEs" lo regenere
REM              completo desde GitHub API description_en y los
REM              _README_es.md cacheados, con el parser actual.
REM              NO consume tokens API si los .md ya estan.
REM
REM v1.1: vive en fuente\reinicios\ con paths relativos.
REM ===============================================================

setlocal

for %%I in ("%~dp0..\..") do set "ROOT=%%~fI"
set "FILE=%ROOT%\data\descripciones_es.json"

if not exist "%FILE%" (
  echo INFO: %FILE% no existe — ya estaba limpio.
  echo.
  goto :next
)

for %%I in ("%FILE%") do (
  echo Archivo a borrar:
  echo   ruta:    %FILE%
  echo   tamano:  %%~zI bytes
)
echo.
del /F /Q "%FILE%"
if exist "%FILE%" (
  echo ERROR: no se pudo borrar.
  pause
  exit /b 1
)
echo OK: descripciones_es.json eliminado.
echo.

:next
echo ============================================================
echo   PROXIMO PASO
echo ============================================================
echo  1. Abre clasificador.exe
echo  2. Pulsa "Traducir READMEs"
echo     - Pasada 1: traduce description_en de GitHub (1 linea cada)
echo     - Pasada 2: extrae fallback de los .md cacheados
echo     - Costo en tokens: bajo (descripciones cortas)
echo     - Al final el log mostrara "N desc ES sync"
echo  3. Pulsa "Abrir buscador"
echo     - Las descripciones aparecen limpias
echo.
pause
exit /b 0

