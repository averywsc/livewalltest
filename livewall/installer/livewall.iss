; Requires Inno Setup 6.3 or newer. Built by tools\build.ps1.
#define AppName "Live Wallpaper"
#define AppVersion "0.1.0"

[Setup]
AppId={{B3A5E5B0-6C0F-4C3E-9B53-0C1D2E9A7F11}
AppName={#AppName}
AppVersion={#AppVersion}
DefaultDirName={autopf}\LiveWallpaper
DefaultGroupName={#AppName}
OutputDir=..\dist
OutputBaseFilename=LiveWallpaper-Setup
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
UninstallDisplayIcon={app}\livewall.exe
AppMutex=LiveWallDaemon
LicenseFile=..\LICENSE
WizardStyle=modern
DisableProgramGroupPage=yes

[Files]
Source: "..\target\release\livewall.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\livewall-settings.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\vendor\libmpv-2.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\THIRD_PARTY.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\livewall.exe"

[Registry]
; Removes the start-with-Windows entry when the app is uninstalled.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "LiveWallpaper"; Flags: uninsdeletevalue dontcreatekey

[Run]
Filename: "{app}\livewall.exe"; Description: "Start Live Wallpaper"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/f /im livewall.exe"; Flags: runhidden skipifdoesntexist; RunOnceId: "StopRenderer"
Filename: "{sys}\taskkill.exe"; Parameters: "/f /im livewall-settings.exe"; Flags: runhidden skipifdoesntexist; RunOnceId: "StopSettings"
