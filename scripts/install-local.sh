#!/usr/bin/env bash
# Instala o Brasa só para o seu usuário (sem sudo): entrada no menu de aplicativos e ícone, para o GNOME
# mostrar o logo da Brasa em vez do ícone padrão. Aponta para o executável compilado deste repositório.
#   scripts/install-local.sh            (depois de `cargo build -p brasa --release`)
# Para desfazer: scripts/install-local.sh --remover
set -euo pipefail
cd "$(dirname "$0")/.."
APPS=${XDG_DATA_HOME:-$HOME/.local/share}/applications
ICONS=${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor
if [ "${1:-}" = "--remover" ]; then
  rm -f "$APPS/brasa.desktop"
  for s in 16 32 48 64 128 256 512; do rm -f "$ICONS/${s}x${s}/apps/brasa.png"; done
  echo "removido"; exit 0
fi
BIN=$(pwd)/target/release/brasa
[ -x "$BIN" ] || { echo "compile antes: cargo build -p brasa --release"; exit 1; }
mkdir -p "$APPS"
sed "s|^Exec=.*|Exec=$BIN|" packaging/brasa.desktop > "$APPS/brasa.desktop"
for s in 16 32 48 64 128 256 512; do
  mkdir -p "$ICONS/${s}x${s}/apps"
  cp "branding/icons/brasa-$s.png" "$ICONS/${s}x${s}/apps/brasa.png"
done
command -v update-desktop-database >/dev/null && update-desktop-database -q "$APPS" || true
echo "instalado: $APPS/brasa.desktop (abre $BIN)"
