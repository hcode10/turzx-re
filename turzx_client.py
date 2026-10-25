"""Client minimal pour l'écran 3,5" Turzx (USB35INCHIPSV2), d'après PROTOCOLE.md.

Dépendances : pip install pyserial pillow
Fermer Turzx (UsbMonitor.exe) avant : il garde le port COM ouvert.

    python turzx_client.py COM5 image.png
"""
import sys
import time

RESET, CLEAR, TO_BLACK = 101, 102, 103
SCREEN_OFF, SCREEN_ON, SET_BRIGHTNESS = 108, 109, 110
SET_ORIENTATION, SET_MIRROR = 121, 122
BEEP_ON, BEEP_OFF = 130, 131
DISPLAY_BITMAP = 197
SLEEP, PING = 201, 255

PORTRAIT, REVERSE_PORTRAIT, LANDSCAPE, REVERSE_LANDSCAPE = 0, 1, 2, 3


def header(cmd, x=0, y=0, ex=0, ey=0):
    return bytes((
        (x >> 2) & 0xFF,
        (((x & 3) << 6) + (y >> 4)) & 0xFF,
        (((y & 0xF) << 4) + (ex >> 6)) & 0xFF,
        (((ex & 0x3F) << 2) + (ey >> 8)) & 0xFF,
        ey & 0xFF,
        cmd & 0xFF,
    ))


def rgb565le(img):
    img = img.convert("RGB")
    out = bytearray()
    for r, g, b in img.getdata():
        v = ((r << 8) & 0xF800) | ((g << 3) & 0x07E0) | (b >> 3)
        out += bytes((v & 0xFF, v >> 8))
    return bytes(out)


class Screen:
    def __init__(self, port, v2=True):
        import serial
        self.v2 = v2
        self.width, self.height = 320, 480
        self.ser = serial.Serial(port, 115200, bytesize=8, parity="N", stopbits=1, timeout=1)
        self.ser.dtr = True
        self.ser.rts = True
        time.sleep(0.1)
        self.send(PING)

    def send(self, cmd, x=0, y=0, ex=0, ey=0, payload=None):
        buf = bytearray(header(cmd, x, y, ex, ey))
        if payload is not None:
            buf += payload
        self.ser.write(buf)

    def brightness(self, percent):
        # 0 = max, 255 = éteint
        self.send(SET_BRIGHTNESS, x=255 - round(percent * 255 / 100))

    def orientation(self, o, mirror=False):
        self.width, self.height = (480, 320) if o >= LANDSCAPE else (320, 480)
        p = bytearray(10)
        p[0] = 100 + o
        p[1:3] = self.width.to_bytes(2, "big")
        p[3:5] = self.height.to_bytes(2, "big")
        self.send(SET_ORIENTATION, payload=p)
        time.sleep(0.01)
        m = bytearray(10)
        m[0] = 1 if mirror else 0
        self.send(SET_MIRROR, payload=m)
        time.sleep(0.01)

    def bitmap(self, img, x=0, y=0):
        w, h = img.size
        self.send(DISPLAY_BITMAP, x, y, x + w - 1, y + h - 1)
        if not self.v2:
            time.sleep(0.01)
        self.ser.write(rgb565le(img))
        if not self.v2:
            time.sleep(0.01)

    def on(self):
        self.send(SET_BRIGHTNESS, x=0)
        self.send(SCREEN_ON)

    def off(self):
        self.send(SET_BRIGHTNESS, x=255)
        self.send(SCREEN_OFF)
        self.send(TO_BLACK)

    def close(self):
        self.ser.close()


if __name__ == "__main__":
    from PIL import Image
    port, path = sys.argv[1], sys.argv[2]
    s = Screen(port)
    s.on()
    s.orientation(PORTRAIT)
    s.bitmap(Image.open(path).resize((s.width, s.height)))
    s.close()
