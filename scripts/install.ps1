<#
  GIT DRENN - installation pour l'utilisateur courant (sans droits administrateur).
  Usage :  powershell -ExecutionPolicy Bypass -File install.ps1
#>
$ErrorActionPreference = 'Stop'
$dest = Join-Path $env:LOCALAPPDATA 'Programs\GitDrenn'
$src = Join-Path $PSScriptRoot 'drenn.exe'
if (-not (Test-Path $src)) { $src = Join-Path $PSScriptRoot '..\dist\drenn.exe' }
if (-not (Test-Path $src)) { Write-Host '[ERREUR] drenn.exe introuvable : compilez d''abord (scripts\build-windows.ps1).' -ForegroundColor Red; exit 1 }

New-Item -ItemType Directory -Force $dest | Out-Null
Copy-Item $src (Join-Path $dest 'drenn.exe') -Force
Write-Host "[OK] drenn.exe copié dans $dest"

$rep = Read-Host "Ajouter $dest au PATH de votre compte (pour taper simplement 'drenn') ? (o/N)"
if ($rep -match '^(o|oui|y|yes)$') {
    $path = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($path -split ';') -notcontains $dest) {
        [Environment]::SetEnvironmentVariable('Path', ($path.TrimEnd(';') + ';' + $dest), 'User')
        Write-Host '[OK] PATH mis à jour : fermez puis rouvrez PowerShell.'
    } else { Write-Host '[..] Déjà dans le PATH.' }
}
& (Join-Path $dest 'drenn.exe') doctor
