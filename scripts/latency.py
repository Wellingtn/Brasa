#!/usr/bin/env python3
"""Latência de teclado de terminais, medida em uma tela virtual (Xephyr), igual para todos.

Para cada terminal: abre uma tela virtual nova, abre o terminal nela, manda uma tecla (XTest) e mede quanto
tempo passa até aparecerem pixels diferentes na tela (lendo a tela virtual em laço). Os pixels que mudam
sozinhos com o terminal parado (cursor piscando, relógio) são descartados antes.

O que a medida inclui: o terminal receber a tecla, o shell devolver o eco, o terminal desenhar e a tela
virtual mostrar. O que ela NÃO inclui: a placa de vídeo e o monitor reais (a tela virtual não tem GPU, o
desenho dos programas que usam OpenGL ou Vulkan é feito por software). Os números servem para comparar um
terminal com outro nas mesmas condições, não como latência de verdade no monitor. A resolução é o tempo de
uma leitura da tela (mostrada como "leitura").

Uso:
    scripts/latency.py [--trials 40] [--startup 8] nome=comando [nome=comando ...]
Exemplo:
    scripts/latency.py xterm="xterm -e bash --norc" brasa="./target/release/brasa"
Só age no display :77 (a tela virtual que ele mesmo abre) e só encerra os processos que abriu.
"""
import argparse, ctypes, ctypes.util, json, os, shlex, signal, statistics, subprocess, sys, time

import numpy as np

DISPLAY = b":77"
W, H = 1280, 800
x11 = ctypes.CDLL(ctypes.util.find_library("X11"))
xtst = ctypes.CDLL(ctypes.util.find_library("Xtst"))
x11.XOpenDisplay.restype = ctypes.c_void_p
x11.XDefaultRootWindow.restype = ctypes.c_ulong
x11.XStringToKeysym.restype = ctypes.c_ulong
x11.XKeysymToKeycode.restype = ctypes.c_ubyte
x11.XGetImage.restype = ctypes.c_void_p


class XImage(ctypes.Structure):
    _fields_ = [("width", ctypes.c_int), ("height", ctypes.c_int), ("xoffset", ctypes.c_int), ("format", ctypes.c_int),
                ("data", ctypes.c_void_p), ("byte_order", ctypes.c_int), ("bitmap_unit", ctypes.c_int),
                ("bitmap_bit_order", ctypes.c_int), ("bitmap_pad", ctypes.c_int), ("depth", ctypes.c_int),
                ("bytes_per_line", ctypes.c_int), ("bits_per_pixel", ctypes.c_int)]


class Screen:
    def __init__(self):
        self.d = ctypes.c_void_p(x11.XOpenDisplay(DISPLAY))
        assert self.d, "sem display :77"
        self.root = x11.XDefaultRootWindow(self.d)

    def grab(self):
        ptr = x11.XGetImage(self.d, ctypes.c_ulong(self.root), 0, 0, W, H, ctypes.c_ulong(0xFFFFFFFF), 2)
        img = ctypes.cast(ptr, ctypes.POINTER(XImage)).contents
        n = img.bytes_per_line * img.height
        arr = np.frombuffer((ctypes.c_ubyte * n).from_address(img.data), dtype=np.uint8).copy()
        x11.XDestroyImage(ctypes.c_void_p(ptr))
        return arr

    def key(self, name):
        kc = x11.XKeysymToKeycode(self.d, x11.XStringToKeysym(name.encode()))
        xtst.XTestFakeKeyEvent(self.d, kc, 1, 0)
        xtst.XTestFakeKeyEvent(self.d, kc, 0, 0)
        x11.XFlush(self.d)

    def click(self, x, y):
        xtst.XTestFakeMotionEvent(self.d, 0, x, y, 0)
        xtst.XTestFakeButtonEvent(self.d, 1, 1, 0)
        xtst.XTestFakeButtonEvent(self.d, 1, 0, 0)
        x11.XFlush(self.d)


def measure(cmd, trials, startup, env_extra):
    xephyr = subprocess.Popen(["Xephyr", DISPLAY.decode(), "-screen", f"{W}x{H}", "-ac", "-noreset", "-no-host-grab"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    term = None
    try:
        time.sleep(2)
        env = dict(os.environ, DISPLAY=DISPLAY.decode(), **env_extra)
        env.pop("WAYLAND_DISPLAY", None)
        term = subprocess.Popen(shlex.split(cmd), env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        time.sleep(startup)
        s = Screen()
        s.click(100, 100)  # sem gerenciador de janelas o foco segue o mouse: fica sobre a janela (canto superior esquerdo)
        time.sleep(1.0)
        # pixels que mudam sozinhos com o terminal parado
        base = s.grab()
        mask = np.zeros(base.shape, dtype=bool)
        t = time.perf_counter()
        frames = 0
        while time.perf_counter() - t < 2.0:
            mask |= s.grab() != base
            frames += 1
        grab_ms = (time.perf_counter() - t) / frames * 1000
        letters = "abcdefghijklmnopqrstuvwxyz"
        out = []
        for i in range(trials + 5):
            time.sleep(0.35)
            ref = s.grab()
            s.key(letters[i % 26])
            t0 = time.perf_counter()
            lat = None
            while time.perf_counter() - t0 < 0.6:
                cur = s.grab()
                if np.count_nonzero((cur != ref) & ~mask) > 12:
                    lat = (time.perf_counter() - t0) * 1000
                    break
            if i >= 5 and lat is not None:  # as 5 primeiras são aquecimento
                out.append(lat)
        return out, grab_ms
    finally:
        if term is not None:
            try:
                os.killpg(term.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            term.wait()
        xephyr.terminate()
        xephyr.wait()


def summarize(xs):
    xs = sorted(xs)
    if not xs:
        return None
    return {"n": len(xs), "mediana": round(statistics.median(xs), 1), "media": round(statistics.mean(xs), 1),
            "p95": round(xs[min(len(xs) - 1, int(len(xs) * 0.95))], 1), "min": round(xs[0], 1), "max": round(xs[-1], 1)}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--trials", type=int, default=40)
    ap.add_argument("--startup", type=float, default=8)
    ap.add_argument("--env", action="append", default=[], help="CHAVE=VALOR para todos os terminais")
    ap.add_argument("terminais", nargs="+", help='nome="comando"')
    a = ap.parse_args()
    extra = dict(e.split("=", 1) for e in a.env)
    results = {}
    for spec in a.terminais:
        name, cmd = spec.split("=", 1)
        print(f"medindo {name} ...", file=sys.stderr)
        xs, grab = measure(cmd, a.trials, a.startup, extra)
        results[name] = {"leitura_ms": round(grab, 1), **(summarize(xs) or {"n": 0})}
        print(json.dumps({name: results[name]}, ensure_ascii=False))
    print("\n| Terminal | Mediana (ms) | Média (ms) | p95 (ms) | Mín–máx (ms) | Teclas | Leitura da tela (ms) |\n|---|---|---|---|---|---|---|")
    for n, r in results.items():
        if r["n"]:
            print(f"| {n} | {r['mediana']} | {r['media']} | {r['p95']} | {r['min']}–{r['max']} | {r['n']} | {r['leitura_ms']} |")
        else:
            print(f"| {n} | sem medidas | | | | 0 | {r['leitura_ms']} |")


if __name__ == "__main__":
    main()
