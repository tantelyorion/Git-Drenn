#!/bin/sh
# GIT DRENN (créateur : Mr Tantely Orion) - compile et installe sous macOS, Linux, FreeBSD, OpenBSD, NetBSD.
# Usage :  sh scripts/install.sh            (installe dans ~/.local/bin)
#          PREFIX=/usr/local sh scripts/install.sh   (peut demander les droits administrateur)
set -eu
cd "$(dirname "$0")/.."
command -v cargo >/dev/null 2>&1 || { echo "[ERREUR] Rust (cargo) est requis : https://rustup.rs (ou le paquet 'rust' de votre système : pkg install rust / pkg_add rust / pkgin install rust)"; exit 1; }
command -v git   >/dev/null 2>&1 || echo "[ATTENTION] Git est introuvable : installez-le avant d'utiliser drenn."
command -v curl  >/dev/null 2>&1 || echo "[ATTENTION] curl est introuvable : GitHub, le mode cloud et 'drenn model install' en ont besoin."
cargo build --release
PREFIX="${PREFIX:-$HOME/.local}"
mkdir -p "$PREFIX/bin"
cp target/release/drenn "$PREFIX/bin/drenn"
chmod 755 "$PREFIX/bin/drenn"
echo "[OK] Installé : $PREFIX/bin/drenn"
case ":$PATH:" in *":$PREFIX/bin:"*) ;; *) echo "[ASTUCE] Ajoutez $PREFIX/bin à votre PATH (ex. : echo 'export PATH=\"$PREFIX/bin:\$PATH\"' >> ~/.profile)";; esac
echo "Ensuite : drenn doctor   puis   drenn model engine   et   drenn model install"
