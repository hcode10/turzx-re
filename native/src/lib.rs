//! Pilote de l'écran Turzx 3,5" (USB35INCH), protocole « révision A ».
//! Voir ../PROTOCOLE.md pour la spécification reconstruite.

use std::io::{self, Write};
use std::thread::sleep;
use std::time::Duration;

pub mod adl;
pub mod dash;
pub mod sys;

pub mod cmd {
    pub const RESET: u8 = 101;
    pub const CLEAR: u8 = 102;
    pub const TO_BLACK: u8 = 103;
    pub const SCREEN_OFF: u8 = 108;
    pub const SCREEN_ON: u8 = 109;
    pub const SET_BRIGHTNESS: u8 = 110;
    pub const SET_ORIENTATION: u8 = 121;
    pub const SET_MIRROR: u8 = 122;
    pub const BEEP_ON: u8 = 130;
    pub const BEEP_OFF: u8 = 131;
    pub const DISPLAY_BITMAP: u8 = 197;
    pub const SLEEP: u8 = 201;
    pub const PING: u8 = 255;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Orientation {
    Portrait = 0,
    ReversePortrait = 1,
    Landscape = 2,
    ReverseLandscape = 3,
}

impl Orientation {
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "portrait" => Some(Self::Portrait),
            "portrait-inv" => Some(Self::ReversePortrait),
            "paysage" => Some(Self::Landscape),
            "paysage-inv" => Some(Self::ReverseLandscape),
            _ => None,
        }
    }

    pub fn dims(self) -> (u16, u16) {
        match self {
            Self::Portrait | Self::ReversePortrait => (320, 480),
            _ => (480, 320),
        }
    }
}

/// En-tête 6 octets : quatre coordonnées 10 bits puis la commande.
pub fn header(cmd: u8, x: u16, y: u16, ex: u16, ey: u16) -> [u8; 6] {
    [
        (x >> 2) as u8,
        (((x & 3) << 6) + (y >> 4)) as u8,
        (((y & 0xF) << 4) + (ex >> 6)) as u8,
        (((ex & 0x3F) << 2) + (ey >> 8)) as u8,
        (ey & 0xFF) as u8,
        cmd,
    ]
}

#[inline]
pub fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16) << 8 & 0xF800) | ((g as u16) << 3 & 0x07E0) | (b as u16 >> 3)
}

/// Cherche l'écran par numéro de série USB (comme Turzx), puis par VID/PID.
/// Renvoie (port, est_v2).
pub fn find_port() -> Option<(String, bool)> {
    let ports = serialport::available_ports().ok()?;
    let mut fallback = None;
    for p in ports {
        if let serialport::SerialPortType::UsbPort(info) = &p.port_type {
            let sn = info.serial_number.clone().unwrap_or_default().to_uppercase();
            if sn.contains("USB35INCH") {
                return Some((p.port_name, sn.contains("V2")));
            }
            if info.vid == 0x1A86 && info.pid == 0x5722 {
                fallback = Some((p.port_name.clone(), false));
            }
        }
    }
    fallback
}

/// Ouvre `port`, ou l'écran détecté automatiquement.
pub fn open(port: Option<&str>) -> io::Result<Screen> {
    let (name, v2) = match port {
        Some(p) => (p.to_string(), true),
        None => find_port().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "écran USB35INCH introuvable (Turzx est-il fermé ?)")
        })?,
    };
    Screen::open(&name, v2).map_err(|e| io::Error::new(e.kind(), format!("{name} : {e}")))
}

pub struct Screen {
    port: Box<dyn serialport::SerialPort>,
    v2: bool,
    pub width: u16,
    pub height: u16,
}

impl Screen {
    pub fn open(name: &str, v2: bool) -> io::Result<Self> {
        let mut port = serialport::new(name, 115_200)
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .flow_control(serialport::FlowControl::None)
            // Sous Windows ce délai s'applique aussi à chaque écriture : on écrit par blocs.
            .timeout(Duration::from_secs(5))
            .open()?;
        port.write_data_terminal_ready(true)?;
        port.write_request_to_send(true)?;
        sleep(Duration::from_millis(100));
        let mut s = Screen { port, v2, width: 320, height: 480 };
        s.send(cmd::PING, 0, 0, 0, 0)?;
        Ok(s)
    }

    pub fn send(&mut self, cmd: u8, x: u16, y: u16, ex: u16, ey: u16) -> io::Result<()> {
        self.port.write_all(&header(cmd, x, y, ex, ey))
    }

    /// Commande longue : tampon de 16 octets, charge utile à partir de l'index 6.
    pub fn send_payload(&mut self, cmd: u8, payload: &[u8]) -> io::Result<()> {
        let mut buf = [0u8; 16];
        buf[..6].copy_from_slice(&header(cmd, 0, 0, 0, 0));
        buf[6..6 + payload.len()].copy_from_slice(payload);
        self.port.write_all(&buf)
    }

    /// 0..=100 %. Le matériel est inversé : 0 = max, 255 = éteint.
    pub fn brightness(&mut self, percent: u8) -> io::Result<()> {
        let level = 255 - (percent.min(100) as u32 * 255 / 100) as u16;
        self.send(cmd::SET_BRIGHTNESS, level, 0, 0, 0)
    }

    pub fn on(&mut self) -> io::Result<()> {
        self.brightness(100)?;
        self.send(cmd::SCREEN_ON, 0, 0, 0, 0)
    }

    pub fn off(&mut self) -> io::Result<()> {
        self.brightness(0)?;
        self.send(cmd::SCREEN_OFF, 0, 0, 0, 0)?;
        self.send(cmd::TO_BLACK, 0, 0, 0, 0)
    }

    pub fn set_orientation(&mut self, o: Orientation, mirror: bool) -> io::Result<()> {
        let (w, h) = o.dims();
        self.width = w;
        self.height = h;
        let p = [100 + o as u8, (w >> 8) as u8, w as u8, (h >> 8) as u8, h as u8];
        self.send_payload(cmd::SET_ORIENTATION, &p)?;
        sleep(Duration::from_millis(10));
        self.send_payload(cmd::SET_MIRROR, &[mirror as u8])?;
        sleep(Duration::from_millis(10));
        Ok(())
    }

    pub fn beep(&mut self, ms: u64) -> io::Result<()> {
        self.send(cmd::BEEP_ON, 0, 0, 0, 0)?;
        sleep(Duration::from_millis(ms));
        self.send(cmd::BEEP_OFF, 0, 0, 0, 0)
    }

    /// Envoie un rectangle de pixels RGB565 little-endian (`data.len() == w*h*2`).
    pub fn blit(&mut self, x: u16, y: u16, w: u16, h: u16, data: &[u8]) -> io::Result<()> {
        debug_assert_eq!(data.len(), w as usize * h as usize * 2);
        self.send(cmd::DISPLAY_BITMAP, x, y, x + w - 1, y + h - 1)?;
        if !self.v2 {
            sleep(Duration::from_millis(10));
        }
        for chunk in data.chunks(16 * 1024) {
            self.port.write_all(chunk)?;
        }
        if !self.v2 {
            sleep(Duration::from_millis(10));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

/// Écran + dernière image envoyée : n'émet que les zones modifiées.
pub struct Display {
    pub screen: Screen,
    prev: Option<Vec<u16>>,
}

const BAND: usize = 16;

impl Display {
    pub fn new(screen: Screen) -> Self {
        Display { screen, prev: None }
    }

    /// Oublie l'image précédente : le prochain `present` sera plein écran.
    pub fn invalidate(&mut self) {
        self.prev = None;
    }

    /// Affiche `fb` (RGB565, largeur × hauteur de l'écran). Renvoie les octets envoyés.
    pub fn present(&mut self, fb: &[u16]) -> io::Result<usize> {
        let (w, h) = (self.screen.width as usize, self.screen.height as usize);
        assert_eq!(fb.len(), w * h);

        let rects = plan(self.prev.as_deref(), fb, w, h);
        let mut sent = 0;
        let mut buf = Vec::new();
        for r in rects {
            buf.clear();
            buf.reserve(r.w * r.h * 2);
            for row in r.y..r.y + r.h {
                for &p in &fb[row * w + r.x..row * w + r.x + r.w] {
                    buf.extend_from_slice(&p.to_le_bytes());
                }
            }
            self.screen.blit(r.x as u16, r.y as u16, r.w as u16, r.h as u16, &buf)?;
            sent += buf.len() + 6;
        }
        self.prev = Some(fb.to_vec());
        Ok(sent)
    }
}

/// Rectangles à envoyer pour passer de `prev` à `cur` (plein écran si plus rentable).
fn plan(prev: Option<&[u16]>, cur: &[u16], w: usize, h: usize) -> Vec<Rect> {
    let rects = match prev {
        Some(prev) if prev.len() == cur.len() => dirty_rects(prev, cur, w, h),
        _ => vec![Rect { x: 0, y: 0, w, h }],
    };
    let area: usize = rects.iter().map(|r| r.w * r.h).sum();
    if area * 10 > w * h * 7 {
        vec![Rect { x: 0, y: 0, w, h }]
    } else {
        rects
    }
}

/// (nombre de rectangles, octets envoyés) pour passer de `prev` à `cur`, sans écran.
pub fn diff_cost(prev: &[u16], cur: &[u16], w: usize, h: usize) -> (usize, usize) {
    let rects = plan(Some(prev), cur, w, h);
    let bytes = rects.iter().map(|r| r.w * r.h * 2 + 6).sum();
    (rects.len(), bytes)
}

/// Écart horizontal (px) en dessous duquel deux zones modifiées d'une même bande sont réunies.
const SPLIT_GAP: usize = 24;

/// Par bande de 16 lignes : une boîte par groupe de colonnes modifiées (séparés de plus de
/// `SPLIT_GAP` px), puis fusion verticale avec la bande précédente quand c'est rentable.
fn dirty_rects(prev: &[u16], cur: &[u16], w: usize, h: usize) -> Vec<Rect> {
    let mut out: Vec<Rect> = Vec::new();
    let mut cols = vec![false; w];
    for band in (0..h).step_by(BAND) {
        let end = (band + BAND).min(h);
        cols.fill(false);
        let (mut y0, mut y1) = (usize::MAX, 0);
        for y in band..end {
            let (a, b) = (&prev[y * w..(y + 1) * w], &cur[y * w..(y + 1) * w]);
            let mut any = false;
            for (x, (p, q)) in a.iter().zip(b).enumerate() {
                if p != q {
                    cols[x] = true;
                    any = true;
                }
            }
            if any {
                y0 = y0.min(y);
                y1 = y;
            }
        }
        if y0 == usize::MAX {
            continue;
        }
        // Groupes de colonnes modifiées.
        let mut x = 0;
        let first_new = out.len();
        while x < w {
            if !cols[x] {
                x += 1;
                continue;
            }
            let start = x;
            let mut last = x;
            while x < w && x <= last + SPLIT_GAP {
                if cols[x] {
                    last = x;
                }
                x += 1;
            }
            out.push(Rect { x: start, y: y0, w: last - start + 1, h: y1 - y0 + 1 });
        }
        // Fusion verticale avec une boîte de la bande précédente qui la chevauche.
        let mut i = first_new;
        while i < out.len() {
            let r = out[i];
            let target = (0..first_new).rev().find(|&j| {
                let p = out[j];
                p.y + p.h + BAND >= r.y && p.x < r.x + r.w && r.x < p.x + p.w
            });
            if let Some(j) = target {
                let p = out[j];
                let ux0 = p.x.min(r.x);
                let ux1 = (p.x + p.w).max(r.x + r.w);
                let u = Rect { x: ux0, y: p.y, w: ux1 - ux0, h: r.y + r.h - p.y };
                if u.w * u.h * 4 <= (p.w * p.h + r.w * r.h) * 5 {
                    out[j] = u;
                    out.remove(i);
                    continue;
                }
            }
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_full_screen() {
        assert_eq!(header(197, 0, 0, 319, 479), [0x00, 0x00, 0x04, 0xfd, 0xdf, 0xc5]);
    }

    #[test]
    fn rgb565_primaries() {
        assert_eq!(rgb565(255, 0, 0), 0xF800);
        assert_eq!(rgb565(0, 255, 0), 0x07E0);
        assert_eq!(rgb565(0, 0, 255), 0x001F);
    }

    #[test]
    fn dirty_single_pixel() {
        let a = vec![0u16; 320 * 480];
        let mut b = a.clone();
        b[100 * 320 + 50] = 1;
        let r = dirty_rects(&a, &b, 320, 480);
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].x, r[0].y, r[0].w, r[0].h), (50, 100, 1, 1));
    }

    #[test]
    fn dirty_splits_distant_changes() {
        let a = vec![0u16; 480 * 320];
        let mut b = a.clone();
        b[100 * 480 + 10] = 1;
        b[100 * 480 + 400] = 1;
        let r = dirty_rects(&a, &b, 480, 320);
        assert_eq!(r.len(), 2);
        assert_eq!(r.iter().map(|r| r.w * r.h).sum::<usize>(), 2);
    }
}
