; Per-user installer for TilingWindowsManager (Inno Setup 6).
; Built by tools/package.ps1, which stages the files and passes
;   /DAppVersion=<version> /DStageDir=<staged files> /DOutDir=<output folder>
; No administrator rights: installs to %LOCALAPPDATA%\Programs and writes
; only to HKEY_CURRENT_USER.

#ifndef AppVersion
  #error AppVersion must be defined (/DAppVersion=x.y.z)
#endif
#ifndef StageDir
  #error StageDir must be defined
#endif
#ifndef OutDir
  #define OutDir "..\dist"
#endif

[Setup]
AppId={{6D1B3F0E-2C4A-4E7B-9C1D-7A5E8B2F4C61}
AppName=TilingWindowsManager
AppVersion={#AppVersion}
AppVerName=TilingWindowsManager {#AppVersion}
AppPublisher=VVVULTURE
AppPublisherURL=https://github.com/VVVULTURE/TilingWindowsManager
AppSupportURL=https://github.com/VVVULTURE/TilingWindowsManager/issues
DefaultDirName={localappdata}\Programs\TilingWindowsManager
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.22000
OutputDir={#OutDir}
OutputBaseFilename=TilingWindowsManager-{#AppVersion}-setup
SetupIconFile=..\assets\twm.ico
UninstallDisplayIcon={app}\twm.exe
UninstallDisplayName=TilingWindowsManager
LicenseFile={#StageDir}\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; The manager is asked to quit (restoring windows) in PrepareToInstall;
; Restart Manager closes anything still holding the files.
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "autostart"; Description: "Start TilingWindowsManager when I sign in"; Flags: unchecked

[Files]
Source: "{#StageDir}\twm.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\twmctl.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\twm-settings.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\THIRD_PARTY_LICENSES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\docs\*"; DestDir: "{app}\docs"; Flags: ignoreversion recursesubdirs
Source: "{#StageDir}\examples\*"; DestDir: "{app}\examples"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{autoprograms}\TilingWindowsManager"; Filename: "{app}\twm.exe"; Comment: "Start the tiling window manager"
Name: "{autoprograms}\TilingWindowsManager Settings"; Filename: "{app}\twm-settings.exe"; Comment: "Settings and key bindings"

[Registry]
; Same value the tray menu's "Start with Windows" toggles.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "TilingWindowsManager"; ValueData: """{app}\twm.exe"""; Tasks: autostart

[Run]
Filename: "{app}\twm.exe"; Description: "Start TilingWindowsManager now"; Flags: nowait postinstall skipifsilent unchecked; Check: not WasRunning
Filename: "{app}\twm.exe"; Flags: nowait; Check: WasRunning

[UninstallRun]
; Quit first so every hidden window is shown and windows go back to place.
Filename: "{app}\twmctl.exe"; Parameters: "quit"; Flags: runhidden waituntilterminated; RunOnceId: "QuitManager"

[Code]
var
  ManagerWasRunning: Boolean;

function WasRunning: Boolean;
begin
  Result := ManagerWasRunning;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
  Ctl: String;
begin
  Result := '';
  ManagerWasRunning := False;
  Ctl := ExpandConstant('{app}\twmctl.exe');
  if FileExists(Ctl) then
  begin
    // Upgrade: if the manager is running, ask it to quit (it shows hidden
    // windows and restores placements) and start it again afterwards.
    if Exec(Ctl, 'ping', '', SW_HIDE, ewWaitUntilTerminated, Code) and (Code = 0) then
    begin
      ManagerWasRunning := True;
      Exec(Ctl, 'quit', '', SW_HIDE, ewWaitUntilTerminated, Code);
      Sleep(1500);
    end;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Data: String;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    // Remove the startup entry whether it came from setup or from the tray menu.
    RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'TilingWindowsManager');
    Data := ExpandConstant('{userappdata}\TilingWindowsManager');
    if DirExists(Data) and (SuppressibleMsgBox('Also delete your TilingWindowsManager configuration?' + #13#10 + Data + #13#10#13#10 +
        'Choose No to keep it for a later reinstall.', mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES) then
    begin
      DelTree(Data, True, True, True);
      DelTree(ExpandConstant('{localappdata}\TilingWindowsManager'), True, True, True);
    end;
  end;
end;
