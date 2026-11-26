; Inno Setup script of Bento Desk.
; Per-user installation: no administrator rights, files in
; %LOCALAPPDATA%\Programs\Bento Desk, start with Windows through the
; HKCU Run key (also managed by the app itself).
;
; Build: cargo build --release, then
;   ISCC.exe /DAppVersion=0.1.0 installer\bento-desk.iss

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#ifndef Arch
  #define Arch "x64"
#endif
#ifndef BinDir
  #define BinDir "..\target\release"
#endif

[Setup]
AppId={{6C3B2F7E-4B1D-4F0A-9E57-BE2D0E5A7D41}
AppName=Bento Desk
AppVersion={#AppVersion}
AppVerName=Bento Desk {#AppVersion}
AppPublisher=Bento Desk
AppPublisherURL=https://github.com/HugoErb/Bento-Desk
AppSupportURL=https://github.com/HugoErb/Bento-Desk/issues
DefaultDirName={autopf}\Bento Desk
DefaultGroupName=Bento Desk
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
LicenseFile=..\LICENSE
OutputDir=..\dist
OutputBaseFilename=BentoDesk-{#AppVersion}-{#Arch}-setup
SetupIconFile=..\assets\bento.ico
UninstallDisplayIcon={app}\bento-desk.exe
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
MinVersion=10.0
#if Arch == "arm64"
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
#else
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
#endif
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "fr"; MessagesFile: "compiler:Languages\French.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "autostart"; Description: "{cm:AutoStart}"; GroupDescription: "{cm:Options}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:Options}"; Flags: unchecked

[Files]
Source: "{#BinDir}\bento-desk.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\bento-settings.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion isreadme

[Icons]
Name: "{group}\Bento Desk"; Filename: "{app}\bento-desk.exe"
Name: "{group}\{cm:SettingsName}"; Filename: "{app}\bento-settings.exe"
Name: "{autodesktop}\Bento Desk"; Filename: "{app}\bento-settings.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "BentoDesk"; ValueData: """{app}\bento-desk.exe"""; Tasks: autostart; Flags: uninsdeletevalue

[Run]
Filename: "{app}\bento-desk.exe"; Description: "{cm:LaunchProgram,Bento Desk}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Puts Explorer options back, then stops the resident app.
Filename: "{app}\bento-desk.exe"; Parameters: "--command restore_explorer_options"; Flags: runhidden; RunOnceId: "RestoreExplorer"
Filename: "{app}\bento-desk.exe"; Parameters: "--command quit"; Flags: runhidden; RunOnceId: "QuitDaemon"

[UninstallDelete]
Type: dirifempty; Name: "{app}"

[CustomMessages]
fr.AutoStart=Démarrer Bento Desk avec Windows
en.AutoStart=Start Bento Desk with Windows
fr.Options=Options :
en.Options=Options:
fr.SettingsName=Paramètres de Bento Desk
en.SettingsName=Bento Desk settings
