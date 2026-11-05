#!/usr/bin/env bash
# Verificação visual isolada: roda o app dentro de um Xephyr (tela virtual) e salva uma captura.
# Não toca na sua área de trabalho nem em outras janelas. Precisa de Xephyr e ImageMagick (import).
#   scripts/visual-check.sh [saida.png]
# Variáveis: BRASA_STARTUP (comandos, um por linha) e BRASA_OPEN_NOTE (nota aberta no início).
# Para interagir: scripts/xdo.py click X Y | type TEXTO | key NOME (só age no display :77).
set -euo pipefail
OUT=${1:-/tmp/brasa-visual.png}; D=:77
command -v Xephyr >/dev/null && command -v import >/dev/null || { echo "precisa de Xephyr e ImageMagick (import)"; exit 1; }
# fecha o app antes da tela virtual; ao contrário, o Xlib do app avisa "XIO: fatal IO error"
cleanup() {
  set +e
  [ -n "${APP_PID:-}" ] && kill "$APP_PID" 2>/dev/null && wait "$APP_PID" 2>/dev/null
  [ -n "${XEPHYR_PID:-}" ] && kill "$XEPHYR_PID" 2>/dev/null && wait "$XEPHYR_PID" 2>/dev/null
  return 0
}
trap cleanup EXIT
cd "$(dirname "$0")/.."
cargo build -p brasa -q 2>&1 | grep -v "proc-macro-error2\|future-incompat" || true
Xephyr "$D" -screen 1500x940 -ac -noreset -no-host-grab >/dev/null 2>&1 & XEPHYR_PID=$!
sleep 2
# padrão fora das aspas duplas: `$'...'` dentro de "${...}" quebra o parser em alguns modos do bash
DEFAULT_STARTUP=$'echo ola mundo\nls /etc | head -4\nfalse'
STARTUP=${BRASA_STARTUP:-$DEFAULT_STARTUP}
# a tela virtual não tem GPU: desenha por software (lavapipe), senão o app tenta a placa e fica preto
DISPLAY=$D env -u WAYLAND_DISPLAY VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json \
  BRASA_STARTUP="$STARTUP" \
  BRASA_OPEN_NOTE="${BRASA_OPEN_NOTE:-bem-vindo.md}" \
  ${BRASA_OPEN_FILE:+BRASA_OPEN_FILE="$BRASA_OPEN_FILE"} \
  ./target/debug/brasa >/dev/null 2>&1 & APP_PID=$!
sleep 10
python3 scripts/xdo.py nudge
sleep 2
DISPLAY=$D import -window root "$OUT"
echo "captura salva em $OUT"
