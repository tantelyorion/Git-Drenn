<#
  GIT DRENN (créateur : Mr Tantely Orion) - installation guidée du moteur IA local (llama.cpp + modèle GGUF).
  Rien n'est téléchargé sans votre accord. Le script propose des modèles Qwen recommandés ; vous pouvez
  toujours fournir votre propre lien. Script non testé sur une vraie machine Windows.
  Usage :  powershell -ExecutionPolicy Bypass -File setup-model.ps1
#>
$ErrorActionPreference = 'Stop'
$base = Join-Path $env:USERPROFILE '.drenn\models'
New-Item -ItemType Directory -Force $base | Out-Null
$drenn = Join-Path $env:LOCALAPPDATA 'Programs\GitDrenn\drenn.exe'
if (-not (Test-Path $drenn)) { $drenn = (Get-Command drenn -ErrorAction SilentlyContinue).Source }
if (-not $drenn) { Write-Host '[ERREUR] drenn.exe introuvable : lancez d''abord install.ps1.' -ForegroundColor Red; exit 1 }

function Telecharger($url, $fichier) {
    if ($url -notmatch '^https://') { throw 'Seules les adresses https:// sont acceptées.' }
    $taille = $null
    try { $taille = [int64](Invoke-WebRequest -Uri $url -Method Head -UseBasicParsing).Headers['Content-Length'] } catch { }
    if ($taille) {
        $libre = (Get-PSDrive -Name ($base.Substring(0, 1))).Free
        Write-Host ("Taille : {0:N0} Mo - espace libre : {1:N0} Mo" -f ($taille / 1MB), ($libre / 1MB))
        if ($taille * 1.5 -gt $libre) { throw 'Espace disque insuffisant.' }
    } else { Write-Host 'Taille inconnue.' }
    $rep = Read-Host 'Télécharger maintenant ? (o/N)'
    if ($rep -notmatch '^(o|oui|y|yes)$') { throw 'Téléchargement annulé.' }
    Invoke-WebRequest -Uri $url -OutFile $fichier -UseBasicParsing
    Write-Host ("SHA-256 : " + (Get-FileHash $fichier -Algorithm SHA256).Hash + "  (comparez-le à celui publié par la source)")
}

# --- 1) llama.cpp : on cherche automatiquement la dernière version Windows CPU sur GitHub ---
Write-Host "`n1) Moteur llama.cpp" -ForegroundColor Cyan
$u1 = $null
try {
    $rel = Invoke-RestMethod -Uri 'https://api.github.com/repos/ggml-org/llama.cpp/releases/latest' -UseBasicParsing
    $asset = $rel.assets | Where-Object { $_.name -match 'bin-win-cpu-x64\.zip$' } | Select-Object -First 1
    if (-not $asset) { $asset = $rel.assets | Where-Object { $_.name -match 'bin-win-avx2-x64\.zip$' } | Select-Object -First 1 }
    if ($asset) { $u1 = $asset.browser_download_url; Write-Host ("Version trouvée : " + $rel.tag_name + " (" + $asset.name + ")") }
} catch { Write-Host 'Recherche automatique impossible (réseau ?).' -ForegroundColor Yellow }
if (-not $u1) { $u1 = Read-Host 'Lien .zip Windows CPU x64 de llama.cpp (vide = llama-cli.exe déjà installé)' }
if ($u1) {
    $zip = Join-Path $base 'llama.zip'
    Telecharger $u1 $zip
    $dir = Join-Path $base 'llama.cpp'
    Expand-Archive $zip -DestinationPath $dir -Force
    Remove-Item $zip
    $cli = Get-ChildItem $dir -Recurse -Filter 'llama-cli.exe' | Select-Object -First 1
    if (-not $cli) { throw 'llama-cli.exe introuvable dans l''archive.' }
    $cliPath = $cli.FullName
} else {
    $cliPath = Read-Host 'Chemin complet de llama-cli.exe'
}

# --- 2) Modèle : choix guidé selon la RAM ---
Write-Host "`n2) Modèle IA (format GGUF quantifié Q4_K_M)" -ForegroundColor Cyan
$ramGo = [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB, 1)
Write-Host "RAM détectée : $ramGo Go"
$hf = 'https://huggingface.co/Qwen'
$choix = @(
    @{ n = 'Qwen2.5 0.5B Instruct  (~0,4 Go) - très léger, moins fiable';           u = "$hf/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf" },
    @{ n = 'Qwen2.5 1.5B Instruct  (~1,1 Go) - RECOMMANDÉ pour 4 Go de RAM';         u = "$hf/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf" },
    @{ n = 'Qwen2.5 3B Instruct    (~2,0 Go) - plus précis, pour 8 Go de RAM et plus'; u = "$hf/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf" }
)
for ($i = 0; $i -lt $choix.Count; $i++) { Write-Host ("  {0}. {1}" -f ($i + 1), $choix[$i].n) }
Write-Host '  4. Autre lien .gguf (ou fichier déjà téléchargé)'
$c = Read-Host 'Votre choix (1-4)'
if ($c -match '^[1-3]$') {
    $u2 = $choix[[int]$c - 1].u
    $modele = Join-Path $base ([IO.Path]::GetFileName(([Uri]$u2).AbsolutePath))
    Telecharger $u2 $modele
} else {
    $u2 = Read-Host 'Lien .gguf (vide = indiquer un fichier déjà téléchargé)'
    if ($u2) {
        $nom = [IO.Path]::GetFileName(([Uri]$u2).AbsolutePath)
        if ($nom -notmatch '\.gguf$') { throw 'Le lien doit pointer vers un fichier .gguf.' }
        $modele = Join-Path $base $nom
        Telecharger $u2 $modele
    } else {
        $modele = Read-Host 'Chemin complet du fichier .gguf'
    }
}

& $drenn config set llama_bin $cliPath
& $drenn config set model_path $modele
& $drenn doctor
Write-Host "`nEnsuite : drenn bench   (mesure de justesse et de temps ; relevez la RAM dans le Gestionnaire des tâches)" -ForegroundColor Green
