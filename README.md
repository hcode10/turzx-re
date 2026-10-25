# turzx-re

Remplaçant natif (Rust) du logiciel TURZX / UsbMonitor pour l'écran USB 3,5"
Turing Smart Screen (révision A, `USB\VID_1A86&PID_5722`, `USB35INCHIPSV2`).

- `PROTOCOLE.md` — protocole série reconstruit (en-tête 6 octets, commandes, RGB565).
- `turzx_client.py` — client Python de référence (pyserial + Pillow).
- `native/` — client natif Windows :
  - `turzx.exe` — CLI (`on`, `off`, `brightness`, `orientation`, `image`, `clear`, `beep`,
    `monitor`, `preview`, `bench`, `gpu`, `stats`).
  - `turzxw.exe` — tableau de bord sans console pour le démarrage automatique
    (`turzxw.exe gamer paysage`).

Tableau de bord : heure, CPU, RAM, disque, GPU AMD (ADL / PMLog), débit réseau ;
thèmes `classique` et `gamer`, portrait ou paysage. Envoi différentiel
(rectangles modifiés seulement) et courbes en mode oscilloscope pour limiter le
rafraîchissement visible (~4 Ko/s en moyenne).

## Compilation

```
cd native
cargo build --release
```
