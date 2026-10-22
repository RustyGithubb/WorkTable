; Installeur Windows de WorkTable, en français et en anglais (Inno Setup 6 — https://jrsoftware.org/isinfo.php).
; Construit par installer\build.ps1, qui compile l'app puis appelle ISCC avec la version de Cargo.toml.
;
; - Installation par utilisateur par défaut (pas besoin d'être administrateur) dans
;   %LOCALAPPDATA%\Programs\WorkTable ; « pour tous les utilisateurs » reste possible (Program Files).
; - Les données de l'utilisateur ne sont JAMAIS dans le dossier du programme : elles vont dans
;   %APPDATA%\WorkTable (réglages, notes, identifiants) et %LOCALAPPDATA%\WorkTable\cache.
;   Elles survivent aux mises à jour ; à la désinstallation, l'utilisateur choisit de les garder.

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#define AppName "WorkTable"
#define AppExe "WorkTable.exe"

[Setup]
AppId={{6B1E0C52-7F7A-4C8B-9D1E-2A9F3C4B5D60}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Dorian Noirel
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=Output
OutputBaseFilename=WorkTable-{#AppVersion}-setup
SetupIconFile=..\assets\logo.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
CloseApplications=yes
RestartApplications=no
; Langue de l'installeur : celle de Windows (français ou anglais), sans question.
ShowLanguageDialog=auto

[Languages]
Name: "french"; MessagesFile: "compiler:Languages\French.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
french.DesktopIcon=Créer un raccourci sur le Bureau
english.DesktopIcon=Create a desktop shortcut
french.Shortcuts=Raccourcis :
english.Shortcuts=Shortcuts:
french.StartupIcon=Lancer WorkTable à l'ouverture de session
english.StartupIcon=Start WorkTable when I sign in
french.Startup=Démarrage :
english.Startup=Startup:
french.DeleteData=Supprimer aussi vos données WorkTable (réglages, notes, identifiants Google) ?
english.DeleteData=Also delete your WorkTable data (settings, notes, Google credentials)?

[Tasks]
Name: "desktopicon"; Description: "{cm:DesktopIcon}"; GroupDescription: "{cm:Shortcuts}"
Name: "startup"; Description: "{cm:StartupIcon}"; GroupDescription: "{cm:Startup}"; Flags: unchecked

[Files]
Source: "..\target\dist\worktable.exe"; DestDir: "{app}"; DestName: "{#AppExe}"; Flags: ignoreversion
; Identifiants OAuth Google (client « Application de bureau ») à livrer avec l'app, si présents :
; placez-les dans installer\google.json avant de construire (jamais versionné, voir .gitignore).
Source: "google.json"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon
Name: "{userstartup}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: startup

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Code]
// Désinstallation : proposer de supprimer aussi les données de l'utilisateur (non par défaut).
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    if MsgBox(CustomMessage('DeleteData') + #13#10 +
              ExpandConstant('{userappdata}\WorkTable'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
    begin
      DelTree(ExpandConstant('{userappdata}\WorkTable'), True, True, True);
      DelTree(ExpandConstant('{localappdata}\WorkTable'), True, True, True);
    end;
  end;
end;
