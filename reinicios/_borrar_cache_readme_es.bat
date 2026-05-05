@echo off
REM ===============================================================
REM Archivo:     _borrar_cache_readme_es.bat
REM Proyecto:    Clasificador de repositorios
REM Autor:       Jorge Mera
REM Fecha:       2026-04-29
REM Version:     1.3.0
REM Descripcion: Borra TODAS las caches de traduccion al espanol
REM              (_README_es.md y _README_es.html) en cada repo.
REM              La proxima vez que pidas traduccion, la app
REM              llamara al LLM y regenerara el cache desde cero
REM              (consume API tokens).
REM
REM v1.3: vive en fuente\reinicios\ con paths relativos (%~dp0..\..)
REM       para detectar el ROOT del workspace dinamicamente.
REM ===============================================================

setlocal enabledelayedexpansion

REM Resolver ROOT a partir de la ubicacion del .bat:
REM   %~dp0          = ...\fuente\reinicios\
REM   %~dp0..\..     = ...\GitHub  (padre del padre)
for %%I in ("%~dp0..\..") do set "ROOT=%%~fI"

set /a md_count=0
set /a html_count=0

echo ============================================================
echo   Buscando caches en %ROOT%
echo ============================================================

pushd "%ROOT%" || (
  echo ERROR: no se pudo entrar a %ROOT%
  pause
  exit /b 1
)

for /D %%C in (0?-* _inbox) do (
  if exist "%%C\" (
    echo.
    echo   ^>^> %%C
    set /a cat_md=0
    set /a cat_html=0

    for /F "usebackq delims=" %%F in (`dir /S /B "%%C\_README_es.md" 2^>nul`) do (
      if exist "%%F" (
        del /F /Q "%%F" 2>nul
        if not exist "%%F" (
          set /a md_count+=1
          set /a cat_md+=1
        )
      )
    )
    for /F "usebackq delims=" %%F in (`dir /S /B "%%C\_README_es.html" 2^>nul`) do (
      if exist "%%F" (
        del /F /Q "%%F" 2>nul
        if not exist "%%F" (
          set /a html_count+=1
          set /a cat_html+=1
        )
      )
    )

    echo      _README_es.md  : !cat_md!
    echo      _README_es.html: !cat_html!
  )
)

popd

echo.
echo ============================================================
echo   TOTAL eliminados:
echo     _README_es.md   : !md_count!
echo     _README_es.html : !html_count!
echo ============================================================
echo.
echo La cache se regenerara la proxima vez que pidas traduccion
echo desde la GUI (consume API tokens).
echo.
pause
exit /b 0

