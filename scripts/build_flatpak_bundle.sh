#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="${ROOT_DIR}/dist"
MANIFEST="${ROOT_DIR}/packaging/flatpak/org.uninote.UniNote.yaml"
APP_ID="org.uninote.UniNote"
BUILD_DIR="${ROOT_DIR}/build/flatpak-build"
REPO_DIR="${ROOT_DIR}/build/flatpak-repo"
BUNDLE_FILE="${DIST_DIR}/${APP_ID}.flatpak"

echo "=== Compilazione Bundle Flatpak UniNote (${APP_ID}) ==="
mkdir -p "${DIST_DIR}" "${BUILD_DIR}" "${REPO_DIR}"

# Verifica presenza flatpak-builder
if ! command -v flatpak-builder >/dev/null 2>&1; then
    echo "ERRORE: flatpak-builder non trovato. Installalo con: sudo pacman -S flatpak-builder"
    exit 1
fi

# Verifica o installa runtime/sdk GNOME se mancante
if ! flatpak info org.gnome.Sdk//49 >/dev/null 2>&1 && ! flatpak info --user org.gnome.Sdk//49 >/dev/null 2>&1; then
    echo "-> Rilevato SDK GNOME 49 mancante. Tentativo di installazione da Flathub..."
    flatpak install --user -y flathub org.gnome.Platform//49 org.gnome.Sdk//49 || {
        echo "AVVISO: Impossibile scaricare SDK GNOME 49. Se hai una versione diversa, aggiorna il manifest."
    }
fi

echo "-> Compilazione Flatpak tramite flatpak-builder..."
flatpak-builder --force-clean --user --disable-rofiles-fuse \
    --repo="${REPO_DIR}" \
    "${BUILD_DIR}" \
    "${MANIFEST}"

echo "-> Generazione bundle standalone .flatpak..."
flatpak build-bundle "${REPO_DIR}" "${BUNDLE_FILE}" "${APP_ID}"

echo "=== Bundle Flatpak generato con successo! ==="
echo "File: ${BUNDLE_FILE}"
echo "Puoi testarlo o installarlo con: flatpak install --user ${BUNDLE_FILE}"
