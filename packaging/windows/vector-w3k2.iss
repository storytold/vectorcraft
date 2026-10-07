; Vector W3K2 installer (Inno Setup 6). Build with packaging/windows/build-installer.ps1, or:
;
;   ISCC.exe /DVersion=0.3.1 /DBinDir=<folder with vectorcraft.exe and vectorcraft-cli.exe> ^
;            /DRoot=<repository root> /DOutDir=<output folder> packaging\windows\vector-w3k2.iss
;
; Installs for the current user (no admin rights needed) or, when chosen in the first dialog, for
; all users into Program Files. Adds a Start Menu shortcut, an optional desktop shortcut and an
; optional association for .vectorcraft documents.

#ifndef Version
  #define Version "0.0.0"
#endif
#ifndef BinDir
  #define BinDir "..\..\target\release"
#endif
#ifndef Root
  #define Root "..\.."
#endif
#ifndef OutDir
  #define OutDir "..\..\dist\release"
#endif

[Setup]
AppId={{90977317-23F5-46DF-B677-D92CF3D07456}
AppName=Vector W3K2
AppVersion={#Version}
AppVerName=Vector W3K2 {#Version}
AppPublisher=Print That 204
AppPublisherURL=https://printthat.ca
AppSupportURL=https://printthat.ca
DefaultDirName={autopf}\Vector W3K2
DefaultGroupName=Vector W3K2
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutDir}
OutputBaseFilename=vector-w3k2-{#Version}-windows-x64-setup
SetupIconFile={#Root}\assets\app-icon\vectorcraft.ico
UninstallDisplayIcon={app}\Vector W3K2.exe
UninstallDisplayName=Vector W3K2
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes
LicenseFile={#Root}\LICENSE-MIT

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "assoc"; Description: "Open .vectorcraft documents with Vector W3K2"; GroupDescription: "File types:"

[Files]
Source: "{#BinDir}\vectorcraft.exe"; DestDir: "{app}"; DestName: "Vector W3K2.exe"; Flags: ignoreversion
Source: "{#BinDir}\vectorcraft-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\NOTICE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Vector W3K2"; Filename: "{app}\Vector W3K2.exe"
Name: "{autodesktop}\Vector W3K2"; Filename: "{app}\Vector W3K2.exe"; Tasks: desktopicon

[Registry]
Root: HKA; Subkey: "Software\Classes\.vectorcraft"; ValueType: string; ValueName: ""; ValueData: "VectorW3K2.Document"; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\VectorW3K2.Document"; ValueType: string; ValueName: ""; ValueData: "Vector W3K2 Document"; Flags: uninsdeletekey; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\VectorW3K2.Document\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\Vector W3K2.exe,0"; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\VectorW3K2.Document\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\Vector W3K2.exe"" ""%1"""; Tasks: assoc
Root: HKA; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\Vector W3K2.exe"; ValueType: string; ValueName: ""; ValueData: "{app}\Vector W3K2.exe"; Flags: uninsdeletekey

[Run]
Filename: "{app}\Vector W3K2.exe"; Description: "{cm:LaunchProgram,Vector W3K2}"; Flags: nowait postinstall skipifsilent
