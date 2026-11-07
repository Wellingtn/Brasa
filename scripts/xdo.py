"""Mini xdotool via XTest (ctypes). Só fala com o display passado (Xephyr), nunca com o desktop real."""
import ctypes, ctypes.util, sys, time
x11 = ctypes.CDLL("libX11.so.6"); xtst = ctypes.CDLL("libXtst.so.6")
x11.XOpenDisplay.restype = ctypes.c_void_p
x11.XKeysymToKeycode.restype = ctypes.c_ubyte
x11.XStringToKeysym.restype = ctypes.c_ulong
class Xdo:
    def __init__(self, display=b":77"):
        assert display == b":77", "recuso agir fora do Xephyr"
        self.d = x11.XOpenDisplay(display); assert self.d, "sem display"
    def flush(self): x11.XFlush(ctypes.c_void_p(self.d)); time.sleep(0.12)
    def move(self, x, y): xtst.XTestFakeMotionEvent(ctypes.c_void_p(self.d), 0, int(x), int(y), 0); self.flush()
    def click(self, x, y, button=1, count=1):
        self.move(x, y)
        for _ in range(count):
            xtst.XTestFakeButtonEvent(ctypes.c_void_p(self.d), button, 1, 0); xtst.XTestFakeButtonEvent(ctypes.c_void_p(self.d), button, 0, 0); self.flush()
    def drag(self, x1, y1, x2, y2, steps=12):
        self.move(x1, y1)
        xtst.XTestFakeButtonEvent(ctypes.c_void_p(self.d), 1, 1, 0); self.flush()
        for k in range(1, steps + 1):
            self.move(x1 + (x2 - x1) * k / steps, y1 + (y2 - y1) * k / steps)
        xtst.XTestFakeButtonEvent(ctypes.c_void_p(self.d), 1, 0, 0); self.flush()
    def _kc(self, name):
        ks = x11.XStringToKeysym(name.encode()); return x11.XKeysymToKeycode(ctypes.c_void_p(self.d), ks)
    def reset_mods(self):
        """Solta Alt, Ctrl, Shift e Super: o Xephyr repassa o estado do teclado real, que pode estar com uma tecla presa."""
        for m in ("Alt_L", "Alt_R", "Meta_L", "Meta_R", "Control_L", "Control_R", "Shift_L", "Shift_R", "Super_L", "Super_R"):
            kc = self._kc(m)
            if kc:  # nem todo teclado tem todas
                xtst.XTestFakeKeyEvent(ctypes.c_void_p(self.d), kc, 0, 0)
        self.flush()
    def key(self, name, mods=()):
        for m in mods: xtst.XTestFakeKeyEvent(ctypes.c_void_p(self.d), self._kc(m), 1, 0)
        xtst.XTestFakeKeyEvent(ctypes.c_void_p(self.d), self._kc(name), 1, 0); xtst.XTestFakeKeyEvent(ctypes.c_void_p(self.d), self._kc(name), 0, 0)
        for m in reversed(mods): xtst.XTestFakeKeyEvent(ctypes.c_void_p(self.d), self._kc(m), 0, 0)
        self.flush()
    def type(self, text):
        names = {" ": "space", "\n": "Return", "/": "slash", "-": "minus", ".": "period", "'": "apostrophe", "$": "dollar", "|": "bar", "~": "asciitilde", "#": "numbersign", ",": "comma", ">": "greater", "<": "less", "(": "parenleft", ")": ")" and "parenright", "*": "asterisk", "=": "equal", "_": "underscore", ";": "semicolon", "'": "apostrophe"}
        for ch in text:
            n = names.get(ch, ch)
            if ch.isupper() or ch in "$|~#><()*_": self.key(n, ("Shift_L",))
            else: self.key(n)
if __name__ == "__main__":
    # uso: xdo.py nudge | click X Y | type TEXTO | key NOME | wheel X Y up|down N | drag X1 Y1 X2 Y2
    x = Xdo(); a = sys.argv[1:] or ["nudge"]
    if a[0] == "nudge": x.move(700, 400); x.move(710, 410); x.click(900, 500)   # força o primeiro redesenho
    elif a[0] == "click": x.click(int(a[1]), int(a[2]))
    elif a[0] == "type": x.type(a[1])
    elif a[0] == "key":
        parts = a[1].split("+"); mods = {"ctrl": "Control_L", "alt": "Alt_L", "shift": "Shift_L"}
        x.key(parts[-1], tuple(mods[m] for m in parts[:-1]))
    elif a[0] == "drag": x.drag(*map(int, a[1:5]))
    elif a[0] == "wheel": x.click(int(a[1]), int(a[2]), button=4 if a[3] == "up" else 5, count=int(a[4]))
