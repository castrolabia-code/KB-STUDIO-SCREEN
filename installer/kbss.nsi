; KB STUDIO SCREEN — instalador para Windows 10/11 x64 (NSIS)
; by Castro Labs
;
; Compilar (Linux o Windows con NSIS 3):  makensis installer/kbss.nsi
; Requiere en descargas/: kbss-engine.exe y kb-probe-win.exe

Unicode true
!include "MUI2.nsh"
!include "x64.nsh"

!define APP "KB Studio Screen"
!define COMPANY "Castro Labs"
!define VERSION "0.8.0"
!define REGKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\KBStudioScreen"
!define PORT "47800"

Name "${APP}"
OutFile "..\descargas\KBStudioScreen-Setup.exe"
InstallDir "$PROGRAMFILES64\${COMPANY}\${APP}"
; Administrador SÓLO para instalar (Archivos de programa y regla del firewall).
; El programa en sí se usa sin administrador.
RequestExecutionLevel admin
SetCompressor /SOLID lzma
BrandingText "${APP} ${VERSION} — by ${COMPANY}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=1034 "ProductName" "${APP}"
VIAddVersionKey /LANG=1034 "CompanyName" "${COMPANY}"
VIAddVersionKey /LANG=1034 "FileDescription" "Instalador de ${APP}"
VIAddVersionKey /LANG=1034 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=1034 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=1034 "LegalCopyright" "(c) 2026 ${COMPANY}"

!define MUI_ICON "kbss.ico"
!define MUI_UNICON "kbss.ico"
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TITLE "Bienvenido a ${APP}"
!define MUI_WELCOMEPAGE_TEXT "Este asistente instala ${APP} ${VERSION}.$\r$\n$\r$\nConvierte hasta tres teléfonos o tablets Android en pantallas táctiles de tu PC, por tu red local. Sin Internet, sin cuentas y sin audio.$\r$\n$\r$\nSe necesita permiso de administrador sólo para instalar y para permitir el programa en el Firewall de Windows (sólo en redes privadas)."
!define MUI_FINISHPAGE_TITLE "Instalación terminada"
!define MUI_FINISHPAGE_TEXT "Abrí ${APP} desde el menú Inicio (sin ejecutarlo como administrador).$\r$\n$\r$\nEn tu teléfono o tablet instalá KBStudioScreen.apk y seguí el Manual de uso."
!define MUI_FINISHPAGE_SHOWREADME "$INSTDIR\Manual de uso.txt"
!define MUI_FINISHPAGE_SHOWREADME_TEXT "Ver el manual de uso"
!define MUI_FINISHPAGE_SHOWREADME_NOTCHECKED

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "LICENCIAS.txt"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Spanish"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "${APP} requiere Windows 10 u 11 de 64 bits."
    Abort
  ${EndIf}
  SetRegView 64
FunctionEnd

Section "${APP} (requerido)" SecMain
  SectionIn RO
  ; Si el motor está abierto (actualización), se cierra para reemplazarlo.
  nsExec::Exec 'taskkill /IM kbss-engine.exe /F'
  Sleep 500

  SetOutPath "$INSTDIR"
  File "..\descargas\kbss-engine.exe"
  ; Cargador de WebView2 (Microsoft, redistribuible) para la ventana propia del programa.
  File "..\descargas\WebView2Loader.dll"
  File "..\descargas\kb-probe-win.exe"
  File "kbss.ico"
  File "LICENCIAS.txt"
  File "/oname=Manual de uso.txt" "..\docs\MANUAL.md"
  WriteUninstaller "$INSTDIR\Desinstalar.exe"

  ; Accesos directos para todos los usuarios.
  SetShellVarContext all
  CreateDirectory "$SMPROGRAMS\${APP}"
  CreateShortcut "$SMPROGRAMS\${APP}\${APP}.lnk" "$INSTDIR\kbss-engine.exe" "" "$INSTDIR\kbss.ico" 0
  CreateShortcut "$SMPROGRAMS\${APP}\Manual de uso.lnk" "$INSTDIR\Manual de uso.txt"
  CreateShortcut "$SMPROGRAMS\${APP}\Diagnóstico del equipo.lnk" "$INSTDIR\kb-probe-win.exe" "" "$INSTDIR\kbss.ico" 0
  CreateShortcut "$SMPROGRAMS\${APP}\Desinstalar.lnk" "$INSTDIR\Desinstalar.exe"

  ; Firewall: sólo este programa, sólo UDP ${PORT}, sólo redes privadas/dominio.
  ; Nunca se desactiva el firewall ni se abre en redes públicas.
  nsExec::Exec 'netsh advfirewall firewall delete rule name="${APP}"'
  nsExec::ExecToLog 'netsh advfirewall firewall add rule name="${APP}" dir=in action=allow program="$INSTDIR\kbss-engine.exe" protocol=UDP localport=${PORT} profile=private,domain enable=yes'

  ; Entrada en "Aplicaciones instaladas".
  WriteRegStr HKLM "${REGKEY}" "DisplayName" "${APP}"
  WriteRegStr HKLM "${REGKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${REGKEY}" "Publisher" "${COMPANY}"
  WriteRegStr HKLM "${REGKEY}" "DisplayIcon" "$INSTDIR\kbss.ico"
  WriteRegStr HKLM "${REGKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${REGKEY}" "UninstallString" '"$INSTDIR\Desinstalar.exe"'
  WriteRegDWORD HKLM "${REGKEY}" "NoModify" 1
  WriteRegDWORD HKLM "${REGKEY}" "NoRepair" 1
  WriteRegDWORD HKLM "${REGKEY}" "EstimatedSize" 6000
SectionEnd

Section /o "Acceso directo en el escritorio" SecDesktop
  SetShellVarContext all
  CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\kbss-engine.exe" "" "$INSTDIR\kbss.ico" 0
SectionEnd

Section /o "Iniciar con Windows (sin abrir el navegador)" SecAutostart
  ; HKCU: arranca con la sesión del usuario, SIN privilegios de administrador.
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "KBStudioScreen" '"$INSTDIR\kbss-engine.exe" --sin-navegador'
SectionEnd

LangString DESC_MAIN ${LANG_SPANISH} "El motor (kbss-engine), el diagnóstico, el manual y la regla del firewall para redes privadas."
LangString DESC_DESK ${LANG_SPANISH} "Crea un ícono en el escritorio."
LangString DESC_AUTO ${LANG_SPANISH} "Abre el motor al iniciar sesión. El panel se abre desde el menú Inicio cuando lo necesites."
!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} $(DESC_MAIN)
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} $(DESC_DESK)
  !insertmacro MUI_DESCRIPTION_TEXT ${SecAutostart} $(DESC_AUTO)
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Function un.onInit
  SetRegView 64
FunctionEnd

Section "Uninstall"
  nsExec::Exec 'taskkill /IM kbss-engine.exe /F'
  Sleep 500
  nsExec::Exec 'netsh advfirewall firewall delete rule name="${APP}"'
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "KBStudioScreen"

  SetShellVarContext all
  Delete "$DESKTOP\${APP}.lnk"
  RMDir /r "$SMPROGRAMS\${APP}"

  Delete "$INSTDIR\kbss-engine.exe"
  Delete "$INSTDIR\WebView2Loader.dll"
  Delete "$INSTDIR\kb-probe-win.exe"
  Delete "$INSTDIR\kbss.ico"
  Delete "$INSTDIR\LICENCIAS.txt"
  Delete "$INSTDIR\Manual de uso.txt"
  Delete "$INSTDIR\Desinstalar.exe"
  RMDir "$INSTDIR"
  RMDir "$PROGRAMFILES64\${COMPANY}"
  DeleteRegKey HKLM "${REGKEY}"

  ; Datos del usuario (identidad, dispositivos emparejados, perfiles, fondo).
  MessageBox MB_YESNO|MB_ICONQUESTION "¿Borrar también los dispositivos emparejados, los perfiles y las preferencias?$\r$\n$\r$\nSi elegís No, se conservan para una próxima instalación." /SD IDNO IDNO keep
    SetShellVarContext current
    RMDir /r "$LOCALAPPDATA\CastroLabs\KBStudioScreen"
    RMDir "$LOCALAPPDATA\CastroLabs"
  keep:
SectionEnd
