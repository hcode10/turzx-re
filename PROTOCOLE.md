# Protocole série Turzx → écran 3,5" (USB35INCHIPSV2)

Reconstruit par analyse statique de `UsbMonitor.exe` (TURZX 2.2.1, assembly `usb_hid`,
.NET Framework 4.8, obfusqué Dotfuscator : noms renommés, chaînes chiffrées, flux de contrôle aplati).
Outils : REA (`inspect-managed-*`) + ILSpy 9.1 + déchiffreur de chaînes maison (`h.b`).
Rien n'a été exécuté ni envoyé à l'écran.

## Matériel / détection

- Port COM virtuel CH34x-like, ici **COM5**, `USB\VID_1A86&PID_5722\USB35INCHIPSV2`.
- Turzx fait `select * from Win32_SerialPort` et garde le port dont le `PNPDeviceID`
  contient `USB35INCH` (ou `USB24INCH` pour le modèle 2,4").
- Si le `PNPDeviceID` **ne contient pas `V2`**, Turzx insère `Thread.Sleep(10)` avant et après
  l'envoi des pixels d'une commande 197. Ton écran est V2 → pas de délai.

## Liaison

| Paramètre | Valeur |
|---|---|
| Débit | 115200 bauds |
| Format | 8N1, pas de parité |
| DTR / RTS | activés |
| ReadTimeout | 1000 ms |
| Sens | **PC → écran uniquement** (Turzx n'appelle jamais `Read`) |

Toutes les écritures passent par un `lock` global puis `SerialPort.Write(buf, 0, len)`.

## Trame de commande (6 octets)

Quatre coordonnées de 10 bits (`x`, `y`, `ex`, `ey`, avec `ex/ey` = coin bas-droit **inclus**)
puis l'octet de commande :

```
b0 = x >> 2
b1 = ((x & 3)  << 6) | (y  >> 4)
b2 = ((y & 15) << 4) | (ex >> 6)
b3 = ((ex & 63)<< 2) | (ey >> 8)
b4 = ey & 0xFF
b5 = cmd
```

Pour les commandes « longues », le PC fournit un tampon plus grand : les 6 premiers octets sont
écrasés par l'en-tête ci-dessus et la charge utile commence à l'**index 6**.

## Commandes

Légende : ✅ usage vu dans Turzx avec contexte clair · ❓ rôle déduit / incertain.

| Cmd | Hex | Rôle | Paramètres | |
|---|---|---|---|---|
| 101 | 0x65 | Reset | — | ✅ |
| 102 | 0x66 | Clear (valeur par défaut de la méthode d'effacement) | — | ❓ |
| 103 | 0x67 | Écran noir (« to black ») | — | ✅ |
| 108 | 0x6C | Écran OFF | — | ✅ |
| 109 | 0x6D | Écran ON | — | ✅ |
| 110 | 0x6E | Luminosité | `x` = 0…255, **0 = max, 255 = éteint** | ✅ |
| 121 | 0x79 | Orientation | tampon 16 o : `[6]=100+o`, `[7..8]=largeur` BE, `[9..10]=hauteur` BE | ✅ |
| 122 | 0x7A | Miroir | tampon 16 o : `[6]=0/1` | ✅ |
| 130 | 0x82 | Buzzer ON (bouton « BEEP ») ; aussi appelé avec `x`=valeur capteur (alarme ?) | — / `x` | ✅/❓ |
| 131 | 0x83 | Buzzer OFF | — | ✅ |
| 161–169, 180 | 0xA1… | Thèmes **intégrés au firmware** : le PC n'envoie que des chiffres | voir plus bas | ❓ |
| 194 | 0xC2 | Bitmap en une seule écriture (en-tête dans les 6 premiers octets du tampon pixels) | zone | ✅ |
| 197 | 0xC5 | **Afficher bitmap** : en-tête, puis écriture séparée des pixels | zone | ✅ |
| 198 | 0xC6 | Variante de 197 sans délai inter-écriture | zone | ✅ |
| 201 | 0xC9 | Veille (bouton « 休眠 ») | — | ✅ |
| 212 | 0xD4 | ? (appelé après un test de connexion) | — | ❓ |
| 255 | 0xFF | Test de connexion (« send test connect ») | — | ✅ |

Orientation `o` : `0` portrait, `1` portrait inversé, `2` paysage, `3` paysage inversé
(Turzx : `o = (paysage ? 2 : 0) + (invertLcd ? 1 : 0)`). Dimensions envoyées = celles de
l'orientation courante (320×480 ou 480×320).

### Zone bitmap

`x, y` = coin haut-gauche, `ex = x + w - 1`, `ey = y + h - 1`.
Turzx l'utilise pour le plein écran (`0,0,320,480`) **et** pour des mises à jour partielles
(chaque widget envoie seulement son rectangle `posX, posY, length, height`).

### Format des pixels

`Bitmap.LockBits(..., PixelFormat.Format16bppRgb565)` puis copie brute du buffer :
**RGB565 little-endian**, ligne par ligne, de haut en bas, sans padding
(`w*2` est multiple de 4 pour les largeurs utilisées).
Plein écran = 320 × 480 × 2 = **307 200 octets**.

```
v = ((R << 8) & 0xF800) | ((G << 3) & 0x07E0) | (B >> 3)
octets = v & 0xFF, v >> 8
```

### Thèmes firmware (161–169, 180) — partiel

Tampon de 20+ octets, payload = chiffres décimaux un par octet (pas d'ASCII). Exemples vus :

- **164** : `[6..9]` = heure HH MM (dizaines/unités), `[10..11]` = temp. CPU, `[12..13]` = temp. GPU.
- **166 / 169** : mêmes données (ordre différent pour 169) + en-tête `x=charge CPU, y=temp GPU, ex=charge CPU`.
- **161** : `x,y = 10,10`, `ex,ey` = dizaines/unités de la temp. CPU.
- **180** : `[9..20]` = charge CPU, temp. GPU (avec décimale), charge GPU, etc., 3 chiffres chacun.

Renvoyées toutes les ~1000 ms. Probablement destinées à d'autres variantes d'écran ; non
nécessaires pour piloter l'écran en mode bitmap.

## Séquences observées

- **Allumage** : `110(x=0)` → `109`.
- **Extinction** : `110(x=255)` → `121(orientation portrait)` → `108` → `103`.
- **Connexion** : ouverture du port → attente 100 ms → `255`.
- **Rafraîchissement** : `197` plein écran au démarrage, puis `197` par rectangle de widget.

## Limites

- Le code est aplati par Dotfuscator : l'ordre exact de certaines branches (ex. dimensions
  dans 121, octets de 166/169) est lu sur du code décompilé brouillé.
- Les sémantiques marquées ❓ ne sont pas confirmées sur le matériel.
- Recoupe le protocole « révision A » documenté par le projet open-source
  `turing-smart-screen-python` (101/102/103/108/109/110/121/197, luminosité inversée, RGB565 LE).
