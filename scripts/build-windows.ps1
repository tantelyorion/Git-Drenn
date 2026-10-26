<#
  GIT DRENN - compilation sous Windows 10/11.
  Usage :  powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1 [-SansIcone] [-SansTests]
  (Script non testé sur une vraie machine Windows : voir docs\GUIDE_COMPILATION.md, section Dépannage.)
#>
param([switch]$SansIcone, [switch]$SansTests)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

function Etape($t) { Write-Host "`n==> $t" -ForegroundColor Cyan }
function Echec($t) { Write-Host "[ERREUR] $t" -ForegroundColor Red; exit 1 }
function Exiger($cmd, $aide) {
    if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) { Echec "'$cmd' est introuvable. $aide" }
}

Etape 'Vérification des prérequis'
Exiger 'git' 'Installez Git : winget install Git.Git (puis rouvrez PowerShell).'
Exiger 'cargo' 'Installez Rust : winget install Rustlang.Rustup (puis rouvrez PowerShell).'
Exiger 'curl.exe' 'curl est fourni avec Windows 10 (version 1803 ou plus) et Windows 11.'
git --version
rustc --version
cargo --version

$cible = (rustc -vV | Select-String 'host:').ToString()
if ($cible -match 'msvc') {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    $outils = $null
    if (Test-Path $vswhere) {
        $outils = & $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    }
    if (-not $outils) {
        Echec ("Les outils de compilation C++ (Visual Studio Build Tools) sont absents. Installez-les :`n" +
               'winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"')
    }
}

if (-not $SansTests) {
    Etape 'Tests (cargo test)'
    cargo test --release
    if ($LASTEXITCODE -ne 0) { Echec 'Des tests ont échoué. Relancez avec -SansTests pour compiler quand même, et signalez le problème.' }
}

Etape 'Compilation (cargo build --release)'
$ok = $false
if (-not $SansIcone) {
    cargo build --release --features icon
    if ($LASTEXITCODE -eq 0) { $ok = $true } else {
        Write-Host '[!!] Compilation avec icône impossible (Windows SDK / rc.exe absent ?) : nouvelle tentative sans icône.' -ForegroundColor Yellow
    }
}
if (-not $ok) {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { Echec 'La compilation a échoué : voir docs\GUIDE_COMPILATION.md, section Dépannage.' }
}

Etape 'Préparation du dossier dist'
New-Item -ItemType Directory -Force dist | Out-Null
Copy-Item target\release\drenn.exe dist\ -Force
Copy-Item README.md dist\ -Force
Copy-Item docs\GUIDE_COMPILATION.md dist\ -Force
Copy-Item scripts\install.ps1, scripts\setup-model.ps1 dist\ -Force
Copy-Item assets\mascot-128.png dist\ -Force
$exe = Get-Item dist\drenn.exe
Write-Host ("drenn.exe : {0:N0} Ko" -f ($exe.Length / 1KB))
Write-Host ("SHA-256   : " + (Get-FileHash $exe.FullName -Algorithm SHA256).Hash)
Compress-Archive -Path dist\* -DestinationPath git-drenn-windows.zip -Force

Write-Host "`n[OK] Terminé. Étape suivante : powershell -ExecutionPolicy Bypass -File dist\install.ps1" -ForegroundColor Green
