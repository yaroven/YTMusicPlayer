; Inno Setup script for the Windows installer (built by the release workflow:
;   iscc /DAppVersion=0.3.0 /DBinDir=<dir with ytm.exe> packaging\windows\ytm-player.iss
; ). Per-user install, no admin rights: %LOCALAPPDATA%\Programs\ytm-player
; (same place install.ps1 uses). Adds a Start menu entry that opens the
; window, puts ytm on the user's PATH and registers an uninstaller in
; "Apps & features", which can also delete the library and sign-in.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef BinDir
  #define BinDir "..\..\target\release"
#endif

[Setup]
AppId={{6F1D2B8E-3C1A-4E4B-9A57-7D2E5C0B9F31}
AppName=ytm-player
AppVersion={#AppVersion}
AppVerName=ytm-player {#AppVersion}
AppPublisher=ytm-player
AppPublisherURL=https://github.com/yaroven/YTMusicPlayer
DefaultDirName={localappdata}\Programs\ytm-player
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputBaseFilename=ytm-player-{#AppVersion}-setup-x64
SetupIconFile=..\..\assets\ytm-player.ico
UninstallDisplayIcon={app}\ytm-player.ico
UninstallDisplayName=ytm-player
ChangesEnvironment=yes
CloseApplications=force
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern

[Tasks]
Name: desktopicon; Description: "Create a &desktop shortcut"; Flags: unchecked
Name: addtopath; Description: "Add &ytm to PATH (use it from a terminal)"

[Files]
Source: "{#BinDir}\ytm.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\assets\ytm-player.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion isreadme

[Icons]
Name: "{userprograms}\ytm-player"; Filename: "{app}\ytm.exe"; Parameters: "gui"; IconFilename: "{app}\ytm-player.ico"; Comment: "Your YouTube Music library"
Name: "{userdesktop}\ytm-player"; Filename: "{app}\ytm.exe"; Parameters: "gui"; IconFilename: "{app}\ytm-player.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\ytm.exe"; Parameters: "gui"; Description: "Open ytm-player"; Flags: nowait postinstall skipifsilent

[Code]
const
  EnvKey = 'Environment';

function PathHasDir(Dir: string): Boolean;
var
  Path: string;
begin
  if not RegQueryStringValue(HKCU, EnvKey, 'Path', Path) then
    Path := '';
  Result := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Path) + ';') > 0;
end;

procedure AddToPath(Dir: string);
var
  Path: string;
begin
  if PathHasDir(Dir) then
    exit;
  if not RegQueryStringValue(HKCU, EnvKey, 'Path', Path) then
    Path := '';
  if (Path <> '') and (Path[Length(Path)] <> ';') then
    Path := Path + ';';
  RegWriteExpandStringValue(HKCU, EnvKey, 'Path', Path + Dir);
end;

procedure RemoveFromPath(Dir: string);
var
  Path: string;
  P: Integer;
begin
  if not RegQueryStringValue(HKCU, EnvKey, 'Path', Path) then
    exit;
  Path := ';' + Path + ';';
  P := Pos(';' + Uppercase(Dir) + ';', Uppercase(Path));
  if P = 0 then
    exit;
  Delete(Path, P, Length(Dir) + 1);
  Path := Copy(Path, 2, Length(Path) - 2);
  RegWriteExpandStringValue(HKCU, EnvKey, 'Path', Path);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('addtopath') then
    AddToPath(ExpandConstant('{app}'));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Code: Integer;
begin
  if CurUninstallStep = usUninstall then
  begin
    // The player may still be running.
    Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM ytm.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
    // Silent uninstalls keep user data.
    if (not UninstallSilent) and (MsgBox('Also delete your library, settings, logs and Google sign-in?' + #13#10 +
        'Choose No to keep them for a later reinstall.', mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES) then
    begin
      Exec(ExpandConstant('{app}\ytm.exe'), 'logout', '', SW_HIDE, ewWaitUntilTerminated, Code);
      DelTree(ExpandConstant('{userappdata}\ytm-player'), True, True, True);
      DelTree(ExpandConstant('{localappdata}\ytm-player'), True, True, True);
    end;
    RemoveFromPath(ExpandConstant('{app}'));
  end;
end;
