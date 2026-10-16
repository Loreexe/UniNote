#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="${ROOT_DIR}/dist"
BUILD_DIR="${ROOT_DIR}/build/arch"
VERSION="0.3.0"

echo "=== Compilazione Pacchetto Arch Linux UniNote v${VERSION} ==="
mkdir -p "${DIST_DIR}" "${BUILD_DIR}"
rm -rf "${BUILD_DIR:?}"/*

# 1. Crea il tarball dei sorgenti pulito dal workspace locale
echo "-> Preparazione tarball dei sorgenti..."
TARBALL="${BUILD_DIR}/uninote-${VERSION}.tar.gz"
tar --exclude-vcs \
    --exclude="./target" \
    --exclude="./build" \
    --exclude="./dist" \
    --exclude="./.uninote-wiki" \
    --exclude="./.git" \
    --transform="s,^\.,uninote-${VERSION}," \
    -czf "${TARBALL}" -C "${ROOT_DIR}" .

# 2. Copia PKGBUILD nella directory di build
cp "${ROOT_DIR}/packaging/aur/PKGBUILD" "${BUILD_DIR}/PKGBUILD"

# 3. Calcola lo SHA-256 reale del tarball e aggiorna il PKGBUILD locale
echo "-> Calcolo checksum SHA-256..."
SHA256=$(sha256sum "${TARBALL}" | awk '{print $1}')
sed -i "s/sha256sums=('SKIP')/sha256sums=('${SHA256}')/" "${BUILD_DIR}/PKGBUILD"

# 4. Esegui makepkg
echo "-> Avvio makepkg..."
(
    cd "${BUILD_DIR}"
    makepkg -fc --noconfirm
)

# 5. Sposta il pacchetto in dist/
PKG_FILE=$(find "${BUILD_DIR}" -name "uninote-${VERSION}-*-x86_64.pkg.tar.zst" -print -quit)
if [[ -f "${PKG_FILE}" ]]; then
    cp -v "${PKG_FILE}" "${DIST_DIR}/"
    echo "=== Pacchetto Arch Linux generato con successo: ${DIST_DIR}/$(basename "${PKG_FILE}") ==="
else
    echo "ERRORE: Pacchetto .pkg.tar.zst non trovato!"
    exit 1
fi

# 6. Crea tarball AUR pronto per upload
tar -czf "${DIST_DIR}/uninote-aur.tar.gz" -C "${ROOT_DIR}/packaging/aur" PKGBUILD .SRCINFO
echo "=== Archivio AUR generato con successo: ${DIST_DIR}/uninote-aur.tar.gz ==="
