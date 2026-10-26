#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="${ROOT_DIR}/dist"
BUILD_DIR="${ROOT_DIR}/build/arch"
# Versione unica di verità: workspace.package.version in Cargo.toml
VERSION="$(awk '/^\[workspace\.package\]/{f=1;next} /^\[/{f=0} f && /^version[[:space:]]*=/{gsub(/["[:space:]]/,"",$3); print $3; exit}' "${ROOT_DIR}/Cargo.toml")"
if [[ -z "${VERSION}" ]]; then
    echo "ERRORE: impossibile determinare la versione da ${ROOT_DIR}/Cargo.toml"
    exit 1
fi

echo "=== Compilazione Pacchetto Arch Linux UniNote v${VERSION} ==="
mkdir -p "${DIST_DIR}" "${BUILD_DIR}"
rm -rf "${BUILD_DIR:?}"/*

# 1. Aggiorna .SRCINFO per entrambi i pacchetti AUR
echo "-> Rigenerazione metadati .SRCINFO..."
(cd "${ROOT_DIR}/packaging/aur/uninote" && makepkg --printsrcinfo > .SRCINFO)
(cd "${ROOT_DIR}/packaging/aur/uninote-git" && makepkg --printsrcinfo > .SRCINFO)
cp "${ROOT_DIR}/packaging/aur/uninote/PKGBUILD" "${ROOT_DIR}/packaging/aur/PKGBUILD"
cp "${ROOT_DIR}/packaging/aur/uninote/.SRCINFO" "${ROOT_DIR}/packaging/aur/.SRCINFO"
cp "${ROOT_DIR}/packaging/aur/uninote-git/PKGBUILD" "${ROOT_DIR}/packaging/aur/PKGBUILD.git"

# 2. Prepara la directory di build
cp "${ROOT_DIR}/packaging/aur/uninote/PKGBUILD" "${BUILD_DIR}/PKGBUILD"

# 3. Esegui makepkg (scarica il tarball della release ufficiale v${VERSION} e verifica sha256)
#    --nocheck: la suite completa è già eseguita dal quality gate (RULES §4) prima del tag;
#    il check() della ricetta resta disponibile per chi compila da AUR.
echo "-> Avvio makepkg..."
(
    cd "${BUILD_DIR}"
    makepkg -fc --nodeps --nocheck --noconfirm
)

# 4. Sposta il pacchetto in dist/
PKG_FILE=$(find "${BUILD_DIR}" -name "uninote-${VERSION}-*-x86_64.pkg.tar.zst" -print -quit)
if [[ -f "${PKG_FILE}" ]]; then
    cp -v "${PKG_FILE}" "${DIST_DIR}/"
    echo "=== Pacchetto Arch Linux generato con successo: ${DIST_DIR}/$(basename "${PKG_FILE}") ==="
else
    echo "ERRORE: Pacchetto .pkg.tar.zst non trovato!"
    exit 1
fi

# 5. Crea i tarball AUR pronti per l'upload
tar -czf "${DIST_DIR}/uninote-aur.tar.gz" -C "${ROOT_DIR}/packaging/aur/uninote" PKGBUILD .SRCINFO
tar -czf "${DIST_DIR}/uninote-git-aur.tar.gz" -C "${ROOT_DIR}/packaging/aur/uninote-git" PKGBUILD .SRCINFO
echo "=== Archivi AUR generati con successo: ==="
echo "  - ${DIST_DIR}/uninote-aur.tar.gz"
echo "  - ${DIST_DIR}/uninote-git-aur.tar.gz"
