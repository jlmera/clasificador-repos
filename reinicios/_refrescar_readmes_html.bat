@echo off
REM ===============================================================
REM Archivo:     _refrescar_readmes_html.bat
REM Proyecto:    Clasificador de repositorios (definitivo)
REM Autor:       Jorge Mera
REM Fecha:       2026-04-29
REM Version:     2.0.0
REM Descripcion: Borra TODOS los HTML cacheados de README en cada
REM              repo:
REM                - _README.html       (idioma original)
REM                - _README_es.html    (traduccion al espanol)
REM              CONSERVA los _README_es.md (cache de traduccion).
REM
REM              La proxima vez que pulses cualquier accion que
REM              dispare reindex (Aplicar / Solo reindexar /
REM              Refrescar GitHub IDs / Traducir READMEs), los
REM              HTML se regeneran desde los .md cacheados con
REM              el parser actual. Cero costo en tokens API.
REM
REM v2.0: ahora borra ambos _README.html y _README_es.html.
REM       Antes solo el .es.html. Necesario cuando el parser
REM       de markdown_to_html cambia (ej. fix de bloques HTML
REM       con div align="center" envolviendo markdown).
REM ===============================================================

setlocal enabledelayedexpansion

for %%I in ("%~dp0..\..") do set "ROOT=%%~fI"

set /a borrados_orig=0
set /a borrados_es=0
set /a md_conservados=0

echo ============================================================
echo   Borrando HTML cacheados (_README.html + _README_es.html)
echo   Conservando los _README_es.md (traducciones).
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
    set /a cat_orig=0
    set /a cat_es=0
    set /a cat_md=0

    REM 1) HTML del README en idioma original
    for /F "usebackq delims=" %%F in (`dir /S /B "%%C\_README.html" 2^>nul`) do (
      if exist "%%F" (
        del /F /Q "%%F" 2>nul
        if not exist "%%F" (
          set /a borrados_orig+=1
          set /a cat_orig+=1
        )
      )
    )
    REM 2) HTML del README traducido al espanol
    for /F "usebackq delims=" %%F in (`dir /S /B "%%C\_README_es.html" 2^>nul`) do (
      if exist "%%F" (
        del /F /Q "%%F" 2>nul
        if not exist "%%F" (
          set /a borrados_es+=1
          set /a cat_es+=1
        )
      )
    )
    REM 3) Contar .md ES preservados (verificacion)
    for /F "usebackq delims=" %%M in (`dir /S /B "%%C\_README_es.md" 2^>nul`) do (
      if exist "%%M" (
        set /a md_conservados+=1
        set /a cat_md+=1
      )
    )

    echo      _README.html      eliminados : !cat_orig!
    echo      _README_es.html   eliminados : !cat_es!
    echo      _README_es.md     conservados: !cat_md!
  )
)

popd

echo.
echo ============================================================
echo   TOTAL
echo ============================================================
echo   _README.html      eliminados :  !borrados_orig!
echo   _README_es.html   eliminados :  !borrados_es!
echo   _README_es.md     conservados :  !md_conservados!
echo.
echo Proximo paso:
echo   1. Abre clasificador.exe
echo   2. Cualquier accion que dispare reindex regenera los HTML:
echo      - Aplicar (mover)
echo      - Solo reindexar
echo      - Refrescar GitHub IDs
echo      - Traducir READMEs
echo   3. Pulsa "Abrir buscador" y verifica los READMEs.
echo      Los _README.html ahora deben renderizar correctamente
echo      el markdown dentro de bloques ^<div^>.
echo.
pause
exit /b 0

