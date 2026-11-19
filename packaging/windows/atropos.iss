; Inno Setup script for the Windows installer.
; iscc /DAppVersion=1.2.3 /DSourceDir=<Release folder> /DOutputDir=<dist> /DIconFile=<app_icon.ico> atropos.iss
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{0D14D80C-7229-446E-A939-65E07300429E}
AppName=Atropos
AppVersion={#AppVersion}
AppPublisher=aimatochysia
AppPublisherURL=https://github.com/aimatochysia/atropos
AppSupportURL=https://github.com/aimatochysia/atropos/issues
AppUpdatesURL=https://github.com/aimatochysia/atropos/releases
DefaultDirName={autopf}\Atropos
DefaultGroupName=Atropos
DisableProgramGroupPage=yes
; Per-user install by default (no admin prompt); the person can choose
; all users instead.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir={#OutputDir}
OutputBaseFilename=atropos-{#AppVersion}-windows-x64-setup
SetupIconFile={#IconFile}
UninstallDisplayIcon={app}\atropos.exe
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[InstallDelete]
; Remove files from older versions that this one no longer ships.
Type: filesandordirs; Name: "{app}\data"

[Icons]
Name: "{autoprograms}\Atropos"; Filename: "{app}\atropos.exe"
Name: "{autodesktop}\Atropos"; Filename: "{app}\atropos.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\atropos.exe"; Description: "{cm:LaunchProgram,Atropos}"; Flags: nowait postinstall skipifsilent
