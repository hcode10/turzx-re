//! Tableau de bord : rendu logiciel RGB888 puis envoi différentiel en RGB565.

use std::thread::sleep;
use std::time::Duration;

use fontdue::{Font, FontSettings};

use crate::adl::{self, Adl};
use crate::{open, rgb565, sys, Display, Orientation};

pub struct Canvas {
    w: usize,
    h: usize,
    px: Vec<[u8; 3]>,
}

impl Canvas {
    pub fn new(w: usize, h: usize, bg: [u8; 3]) -> Self {
        Canvas { w, h, px: vec![bg; w * h] }
    }

    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: [u8; 3]) {
        let (x0, y0) = (x.max(0) as usize, y.max(0) as usize);
        let x1 = ((x + w).max(0) as usize).min(self.w);
        let y1 = ((y + h).max(0) as usize).min(self.h);
        for yy in y0..y1 {
            self.px[yy * self.w + x0..yy * self.w + x1.max(x0)].fill(c);
        }
    }

    fn blend(&mut self, x: i32, y: i32, c: [u8; 3], a: u8) {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h || a == 0 {
            return;
        }
        let p = &mut self.px[y as usize * self.w + x as usize];
        for i in 0..3 {
            p[i] = ((c[i] as u32 * a as u32 + p[i] as u32 * (255 - a as u32)) / 255) as u8;
        }
    }

    pub fn to565(&self) -> Vec<u16> {
        self.px.iter().map(|p| rgb565(p[0], p[1], p[2])).collect()
    }
}

fn text_width(font: &Font, size: f32, s: &str) -> i32 {
    s.chars().map(|c| font.metrics(c, size).advance_width).sum::<f32>().round() as i32
}

/// Dessine `s` avec la ligne de base en `y`. `align` : 0 gauche, 1 centre, 2 droite.
fn text(cv: &mut Canvas, font: &Font, size: f32, x: i32, y: i32, s: &str, c: [u8; 3], align: u8) {
    let mut pen = match align {
        1 => x - text_width(font, size, s) / 2,
        2 => x - text_width(font, size, s),
        _ => x,
    } as f32;
    for ch in s.chars() {
        let (m, bmp) = font.rasterize(ch, size);
        let gx = pen.round() as i32 + m.xmin;
        let gy = y - m.height as i32 - m.ymin;
        for row in 0..m.height {
            for col in 0..m.width {
                cv.blend(gx + col as i32, gy + row as i32, c, bmp[row * m.width + col]);
            }
        }
        pen += m.advance_width;
    }
}

fn load_font(names: &[&str]) -> std::io::Result<Font> {
    for n in names {
        if let Ok(bytes) = std::fs::read(format!(r"C:\Windows\Fonts\{n}")) {
            if let Ok(f) = Font::from_bytes(bytes, FontSettings::default()) {
                return Ok(f);
            }
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("police introuvable : {names:?}")))
}

const BG: [u8; 3] = [12, 14, 20];
const TRACK: [u8; 3] = [36, 40, 50];
const PANEL: [u8; 3] = [18, 21, 30];
const FG: [u8; 3] = [235, 238, 245];
const DIM: [u8; 3] = [130, 138, 155];
const ACCENT: [u8; 3] = [80, 160, 255];
const UP: [u8; 3] = [245, 170, 50];

fn level_color(pct: f32) -> [u8; 3] {
    if pct < 60.0 {
        [70, 200, 120]
    } else if pct < 85.0 {
        [245, 170, 50]
    } else {
        [240, 70, 70]
    }
}

fn bar(cv: &mut Canvas, x: i32, y: i32, w: i32, h: i32, pct: f32) {
    cv.rect(x, y, w, h, TRACK);
    let fill = (w as f32 * pct.clamp(0.0, 100.0) / 100.0).round() as i32;
    cv.rect(x, y, fill, h, level_color(pct));
}

fn rate(bps: f64) -> String {
    if bps >= 1024.0 * 1024.0 {
        format!("{:.1} Mo/s", bps / 1024.0 / 1024.0)
    } else {
        format!("{:.0} Ko/s", bps / 1024.0)
    }
}

const JOURS: [&str; 7] = ["dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi"];
const MOIS: [&str; 12] = [
    "janvier", "février", "mars", "avril", "mai", "juin",
    "juillet", "août", "septembre", "octobre", "novembre", "décembre",
];


const M: i32 = 16;
/// Points des courbes (une mesure par seconde).
const HIST: usize = 72;
/// Points effacés devant le curseur de balayage.
const SWEEP_GAP: usize = 2;
/// Variation minimale (en points de %) avant de mettre à jour une jauge.
const DEADBAND: f32 = 2.0;

/// Historique circulaire façon oscilloscope : la nouvelle valeur remplace la plus ancienne
/// sous le curseur. Rien ne défile, donc seules quelques colonnes changent à chaque seconde.
#[derive(Clone)]
struct Sweep {
    vals: Vec<f32>,
    filled: Vec<bool>,
    pos: usize,
}

impl Sweep {
    fn new() -> Self {
        Sweep { vals: vec![0.0; HIST], filled: vec![false; HIST], pos: 0 }
    }

    fn push(&mut self, v: f32) {
        self.vals[self.pos] = v;
        self.filled[self.pos] = true;
        self.pos = (self.pos + 1) % HIST;
    }

    fn max(&self) -> f32 {
        self.vals.iter().zip(&self.filled).filter(|p| *p.1).map(|p| *p.0).fold(0.0, f32::max)
    }

    fn visible(&self, i: usize) -> bool {
        self.filled[i] && (i + HIST - self.pos) % HIST >= SWEEP_GAP
    }

    fn newest(&self) -> usize {
        (self.pos + HIST - 1) % HIST
    }
}

/// Échelle réseau par paliers (×4) : elle ne bouge que quand un pic entre ou sort de l'historique.
fn net_scale(rx: &Sweep, tx: &Sweep) -> f32 {
    let m = rx.max().max(tx.max());
    let mut s = 128.0 * 1024.0;
    while s < m {
        s *= 4.0;
    }
    s
}

/// Ne suit `raw` que s'il s'écarte d'au moins `DEADBAND` de la valeur affichée.
fn settle(shown: &mut f32, raw: f32) -> f32 {
    if (raw - *shown).abs() >= DEADBAND || raw <= 0.5 || raw >= 99.5 {
        *shown = raw;
    }
    *shown
}

/// Segment anti-aliasé d'épaisseur `th`.
fn seg(cv: &mut Canvas, x0: f32, y0: f32, x1: f32, y1: f32, th: f32, c: [u8; 3]) {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let len2 = (dx * dx + dy * dy).max(1e-6);
    let r = th / 2.0 + 1.0;
    for py in (y0.min(y1) - r).floor() as i32..=(y0.max(y1) + r).ceil() as i32 {
        for px in (x0.min(x1) - r).floor() as i32..=(x0.max(x1) + r).ceil() as i32 {
            let (qx, qy) = (px as f32 + 0.5 - x0, py as f32 + 0.5 - y0);
            let t = ((qx * dx + qy * dy) / len2).clamp(0.0, 1.0);
            let (ex, ey) = (qx - t * dx, qy - t * dy);
            let cov = (th / 2.0 - (ex * ex + ey * ey).sqrt() + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                cv.blend(px, py, c, (cov * 255.0) as u8);
            }
        }
    }
}

/// Courbe balayée dans [x, x+w) × [y, y+h) : remplissage dégradé sous la ligne (si `fill` > 0)
/// et point lumineux sur la dernière mesure.
fn curve(cv: &mut Canvas, x: i32, w: i32, y: i32, h: i32, s: &Sweep, scale: f32, c: [u8; 3], fill: u8) {
    let step = (w - 1) as f32 / (HIST - 1) as f32;
    let pt = |i: usize| {
        let v = (s.vals[i] / scale).clamp(0.0, 1.0);
        (x as f32 + i as f32 * step + 0.5, y as f32 + 1.0 + (h - 3) as f32 * (1.0 - v))
    };
    for i in 0..HIST - 1 {
        if !(s.visible(i) && s.visible(i + 1)) {
            continue;
        }
        let ((x0, y0), (x1, y1)) = (pt(i), pt(i + 1));
        if fill > 0 {
            for px in x0.round() as i32..x1.round() as i32 {
                let ly = y0 + (y1 - y0) * (px as f32 + 0.5 - x0) / (x1 - x0);
                for py in ly.round() as i32..y + h {
                    let a = fill as f32 * (1.0 - 0.8 * (py - y) as f32 / h as f32);
                    cv.blend(px, py, c, a as u8);
                }
            }
        }
        seg(cv, x0, y0, x1, y1, 2.0, c);
    }
    let n = s.newest();
    if s.filled[n] {
        let (cx, cy) = pt(n);
        for py in (cy - 6.0) as i32..=(cy + 6.0) as i32 {
            for px in (cx - 6.0) as i32..=(cx + 6.0) as i32 {
                let d = ((px as f32 + 0.5 - cx).powi(2) + (py as f32 + 0.5 - cy).powi(2)).sqrt();
                let core = (3.0 - d + 0.5).clamp(0.0, 1.0);
                let glow = (1.0 - d / 6.0).clamp(0.0, 1.0) * 0.5;
                cv.blend(px, py, c, (core.max(glow) * 255.0) as u8);
            }
        }
    }
}

#[derive(Default, Clone)]
struct Gpu {
    load: f32,
    temp: Option<i32>,
    hotspot: Option<i32>,
    power: Option<i32>,
}

fn read_gpu(adl: &Adl) -> Option<Gpu> {
    let s = adl.sensors()?;
    let get = |i: usize| s.get(i).filter(|v| v.0).map(|v| v.1);
    Some(Gpu {
        load: get(adl::PMLOG_INFO_ACTIVITY_GFX).unwrap_or(0) as f32,
        temp: get(adl::PMLOG_TEMPERATURE_EDGE),
        hotspot: get(adl::PMLOG_TEMPERATURE_HOTSPOT),
        power: get(adl::PMLOG_ASIC_POWER),
    })
}

struct Fonts {
    regular: Font,
    bold: Font,
    tech: Font,
}

impl Fonts {
    fn load() -> std::io::Result<Self> {
        Ok(Fonts {
            regular: load_font(&["segoeui.ttf", "arial.ttf"])?,
            bold: load_font(&["segoeuib.ttf", "arialbd.ttf"])?,
            tech: load_font(&["bahnschrift.ttf", "segoeuib.ttf", "arialbd.ttf"])?,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Theme {
    Classic,
    Gamer,
}

impl Theme {
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "classique" | "classic" => Some(Self::Classic),
            "gamer" => Some(Self::Gamer),
            _ => None,
        }
    }
}

/// Une mesure complète, rendue ensuite selon l'orientation.
struct Snapshot {
    now: sys::Time,
    cpu: f32,
    cpu_hist: Sweep,
    gpu: Option<Gpu>,
    mem: sys::Memory,
    disk: Option<(u64, u64)>,
    rx: f64,
    tx: f64,
    rx_hist: Sweep,
    tx_hist: Sweep,
}

fn gib(b: u64) -> f64 {
    b as f64 / (1u64 << 30) as f64
}

/// Ligne « LIBELLÉ  détail ……… xx % » + barre, sur la colonne [x, x+w).
fn row(cv: &mut Canvas, f: &Fonts, x: i32, w: i32, y: i32, label: &str, detail: &str, pct: f32) {
    text(cv, &f.bold, 21.0, x, y, label, FG, 0);
    let lw = text_width(&f.bold, 21.0, label);
    text(cv, &f.regular, 17.0, x + lw + 10, y, detail, DIM, 0);
    text(cv, &f.bold, 21.0, x + w, y, &format!("{pct:.0} %"), level_color(pct), 2);
    bar(cv, x, y + 8, w, 8, pct);
}

fn clock(s: &Snapshot) -> String {
    format!("{:02}:{:02}", s.now.hour, s.now.min)
}

fn date(s: &Snapshot) -> String {
    format!("{} {} {}", JOURS[s.now.weekday as usize % 7], s.now.day, MOIS[(s.now.month as usize + 11) % 12])
}

fn gpu_detail(g: &Gpu, compact: bool) -> String {
    let mut d = Vec::new();
    match (g.temp, g.hotspot) {
        (Some(t), Some(h)) if compact => d.push(format!("{t} / {h} °C")),
        (t, h) => {
            if let Some(t) = t {
                d.push(format!("{t} °C"));
            }
            if let Some(h) = h {
                d.push(format!("pt chaud {h} °C"));
            }
        }
    }
    if let Some(p) = g.power {
        d.push(format!("{p} W"));
    }
    d.join(" · ")
}

fn gpu_row(cv: &mut Canvas, f: &Fonts, x: i32, w: i32, y: i32, s: &Snapshot, compact: bool) {
    match &s.gpu {
        Some(g) => row(cv, f, x, w, y, "GPU", &gpu_detail(g, compact), g.load),
        None => text(cv, &f.regular, 17.0, x, y, "GPU : capteurs AMD indisponibles", DIM, 0),
    }
}

fn ram_row(cv: &mut Canvas, f: &Fonts, x: i32, w: i32, y: i32, s: &Snapshot) {
    let used = s.mem.total - s.mem.avail;
    row(cv, f, x, w, y, "RAM", &format!("{:.1} / {:.1} Go", gib(used), gib(s.mem.total)), s.mem.load as f32);
}

fn disk_row(cv: &mut Canvas, f: &Fonts, x: i32, w: i32, y: i32, s: &Snapshot) {
    if let Some((free, total)) = s.disk {
        let pct = 100.0 * (total - free) as f32 / total as f32;
        row(cv, f, x, w, y, "C:", &format!("{:.0} Go libres", gib(free)), pct);
    }
}

/// Bloc réseau : titre en `y`, graphe de hauteur `gh`, débits en dessous.
fn net_block(cv: &mut Canvas, f: &Fonts, x: i32, w: i32, y: i32, gh: i32, s: &Snapshot) {
    text(cv, &f.bold, 21.0, x, y, "Réseau", FG, 0);
    let gy = y + 10;
    cv.rect(x, gy, w, gh, PANEL);
    let scale = net_scale(&s.rx_hist, &s.tx_hist);
    curve(cv, x, w, gy, gh, &s.rx_hist, scale, ACCENT, 60);
    curve(cv, x, w, gy, gh, &s.tx_hist, scale, UP, 0);
    text(cv, &f.regular, 13.0, x + w, y, &format!("échelle {}", rate(scale as f64)), DIM, 2);
    text(cv, &f.bold, 19.0, x, gy + gh + 24, &format!("↓ {}", rate(s.rx)), ACCENT, 0);
    text(cv, &f.bold, 19.0, x + w, gy + gh + 24, &format!("↑ {}", rate(s.tx)), UP, 2);
}

fn footer(cv: &mut Canvas, f: &Fonts, cx: i32, y: i32) {
    text(cv, &f.regular, 13.0, cx, y, "turzx natif", [70, 76, 90], 1);
}

/// 320×480 : une colonne.
fn render_portrait(cv: &mut Canvas, f: &Fonts, s: &Snapshot) {
    let (w, inner) = (320, 320 - 2 * M);
    text(cv, &f.bold, 54.0, w / 2, 66, &clock(s), FG, 1);
    text(cv, &f.regular, 20.0, w / 2, 96, &date(s), DIM, 1);
    cv.rect(M, 110, inner, 1, TRACK);

    row(cv, f, M, inner, 138, "CPU", "", s.cpu);
    cv.rect(M, 160, inner, 44, PANEL);
    curve(cv, M, inner, 160, 44, &s.cpu_hist, 100.0, ACCENT, 60);

    gpu_row(cv, f, M, inner, 232, s, false);
    ram_row(cv, f, M, inner, 276, s);
    disk_row(cv, f, M, inner, 320, s);
    net_block(cv, f, M, inner, 364, 48, s);
    footer(cv, f, w / 2, 472);
}

/// 480×320 : heure + CPU + GPU à gauche, RAM + disque + réseau à droite.
fn render_landscape(cv: &mut Canvas, f: &Fonts, s: &Snapshot) {
    const GAP: i32 = 16;
    let col_w = (480 - 2 * M - GAP) / 2;
    let (lx, rx) = (M, M + col_w + GAP);

    text(cv, &f.bold, 46.0, lx + col_w / 2, 54, &clock(s), FG, 1);
    text(cv, &f.regular, 18.0, lx + col_w / 2, 80, &date(s), DIM, 1);

    row(cv, f, lx, col_w, 120, "CPU", "", s.cpu);
    cv.rect(lx, 142, col_w, 60, PANEL);
    curve(cv, lx, col_w, 142, 60, &s.cpu_hist, 100.0, ACCENT, 60);
    gpu_row(cv, f, lx, col_w, 234, s, true);

    cv.rect(rx - GAP / 2, M, 1, 252, TRACK);

    ram_row(cv, f, rx, col_w, 38, s);
    disk_row(cv, f, rx, col_w, 82, s);
    net_block(cv, f, rx, col_w, 126, 100, s);

    footer(cv, f, 240, 306);
}


// ---------------------------------------------------------------- thème gamer

const G_BG: [u8; 3] = [5, 6, 12];
const G_TRACK: [u8; 3] = [22, 26, 42];
const CYAN: [u8; 3] = [0, 229, 255];
const MAGENTA: [u8; 3] = [255, 0, 170];
const G_DIM: [u8; 3] = [90, 110, 140];
const WHITE: [u8; 3] = [240, 248, 255];

fn lerp(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    [0, 1, 2].map(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8)
}

impl Canvas {
    fn blend_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: [u8; 3], a: u8) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.blend(xx, yy, c, a);
            }
        }
    }

    /// Assombrit une ligne sur deux (effet écran cathodique).
    fn scanlines(&mut self) {
        for y in (1..self.h).step_by(2) {
            for p in &mut self.px[y * self.w..(y + 1) * self.w] {
                *p = p.map(|v| (v as u16 * 7 / 8) as u8);
            }
        }
    }
}

/// Ligne horizontale néon avec halo.
fn neon_line(cv: &mut Canvas, x: i32, y: i32, w: i32, c: [u8; 3]) {
    cv.blend_rect(x, y - 2, w, 5, c, 30);
    cv.blend_rect(x, y - 1, w, 3, c, 70);
    cv.rect(x, y, w, 1, c);
}

/// Coins en équerre façon HUD autour de [x, x+w) × [y, y+h).
fn brackets(cv: &mut Canvas, x: i32, y: i32, w: i32, h: i32, c: [u8; 3]) {
    const L: i32 = 12;
    for (cx, cy, sx, sy) in [(x, y, 1, 1), (x + w - 1, y, -1, 1), (x, y + h - 1, 1, -1), (x + w - 1, y + h - 1, -1, -1)] {
        let hx = if sx > 0 { cx } else { cx - L + 1 };
        let vy = if sy > 0 { cy } else { cy - L + 1 };
        cv.rect(hx, cy - if sy > 0 { 0 } else { 1 }, L, 2, c);
        cv.rect(cx - if sx > 0 { 0 } else { 1 }, vy, 2, L, c);
    }
    cv.blend_rect(x + 2, y + 2, w - 4, h - 4, c, 10);
}

/// Jauge circulaire 270° (ouverte en bas), dégradé cyan → magenta, avec halo.
fn ring(cv: &mut Canvas, cx: i32, cy: i32, r: f32, thick: f32, pct: f32) {
    use std::f32::consts::{PI, TAU};
    let start = 0.75 * PI;
    let sweep = 1.5 * PI;
    let fill = pct.clamp(0.0, 100.0) / 100.0;
    let ext = (r + thick / 2.0 + 5.0).ceil() as i32;
    for y in cy - ext..=cy + ext {
        for x in cx - ext..=cx + ext {
            let dx = x as f32 + 0.5 - cx as f32;
            let dy = y as f32 + 0.5 - cy as f32;
            let a = (dy.atan2(dx) - start).rem_euclid(TAU);
            if a > sweep {
                continue;
            }
            let t = a / sweep;
            let edge = thick / 2.0 - ((dx * dx + dy * dy).sqrt() - r).abs();
            let lit = t <= fill;
            let c = if lit { lerp(CYAN, MAGENTA, t) } else { G_TRACK };
            let cov = (edge + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                cv.blend(x, y, c, (cov * 255.0) as u8);
            } else if lit {
                let g = (1.0 + edge / 5.0).clamp(0.0, 1.0);
                cv.blend(x, y, c, (g * g * 90.0) as u8);
            }
        }
    }
}

/// Jauge + valeur au centre + libellé dans l'ouverture + détail en dessous.
fn gauge(cv: &mut Canvas, f: &Fonts, cx: i32, cy: i32, r: f32, pct: f32, label: &str, detail: &str) {
    ring(cv, cx, cy, r, 10.0, pct);
    let v = format!("{pct:.0}");
    let size = (r * 0.7).min(34.0);
    let vw = text_width(&f.tech, size, &v);
    let pw = text_width(&f.tech, size * 0.47, "%");
    let x0 = cx - (vw + pw + 2) / 2;
    let base = cy + (size * 0.3) as i32;
    text(cv, &f.tech, size, x0, base, &v, WHITE, 0);
    text(cv, &f.tech, size * 0.47, x0 + vw + 2, base, "%", G_DIM, 0);
    text(cv, &f.tech, 15.0, cx, cy + r as i32 + 4, label, CYAN, 1);
    if !detail.is_empty() {
        text(cv, &f.tech, 15.0, cx, cy + r as i32 + 26, detail, G_DIM, 1);
    }
}

/// Barre segmentée, segments allumés en dégradé.
fn seg_bar(cv: &mut Canvas, x: i32, y: i32, w: i32, h: i32, pct: f32) {
    const SEG: i32 = 6;
    const GAP: i32 = 2;
    let n = (w + GAP) / (SEG + GAP);
    let lit = (n as f32 * pct.clamp(0.0, 100.0) / 100.0).round() as i32;
    for i in 0..n {
        let sx = x + i * (SEG + GAP);
        if i < lit {
            let c = lerp(CYAN, MAGENTA, i as f32 / (n - 1).max(1) as f32);
            cv.blend_rect(sx - 1, y - 1, SEG + 2, h + 2, c, 50);
            cv.rect(sx, y, SEG, h, c);
        } else {
            cv.rect(sx, y, SEG, h, G_TRACK);
        }
    }
}

fn header(cv: &mut Canvas, f: &Fonts, w: i32, s: &Snapshot) {
    text(cv, &f.tech, 15.0, M, 22, "// SYSTÈME EN LIGNE", CYAN, 0);
    text(cv, &f.tech, 12.0, M, 38, &date(s).to_uppercase(), G_DIM, 0);
    text(cv, &f.tech, 32.0, w - M, 36, &clock(s), WHITE, 2);
    neon_line(cv, M, 48, w - 2 * M, CYAN);
}

fn gpu_gauge_detail(s: &Snapshot) -> String {
    match &s.gpu {
        Some(g) => match (g.temp, g.hotspot) {
            (Some(t), Some(h)) => format!("{t}° / {h}°C"),
            (Some(t), None) => format!("{t}°C"),
            _ => String::new(),
        },
        None => "N/D".into(),
    }
}

fn mem_detail(s: &Snapshot) -> String {
    format!("{:.1} / {:.0} GO", gib(s.mem.total - s.mem.avail), gib(s.mem.total))
}

fn seg_row(cv: &mut Canvas, f: &Fonts, x: i32, w: i32, y: i32, label: &str, detail: &str, pct: f32) {
    text(cv, &f.tech, 15.0, x, y, label, CYAN, 0);
    text(cv, &f.tech, 15.0, x + w, y, &format!("{detail}  {pct:.0} %"), WHITE, 2);
    seg_bar(cv, x, y + 8, w, 9, pct);
}

/// Panneau réseau encadré : graphe à gauche (ou au-dessus), débits ensuite.
fn net_panel(cv: &mut Canvas, f: &Fonts, x: i32, y: i32, w: i32, h: i32, side: i32, s: &Snapshot) {
    brackets(cv, x, y, w, h, CYAN);
    text(cv, &f.tech, 13.0, x + 10, y + 18, "RÉSEAU", CYAN, 0);
    let scale = net_scale(&s.rx_hist, &s.tx_hist);
    let (gx, gy, gw, gh) = if side > 0 {
        (x + 10, y + 26, w - side - 30, h - 34)
    } else {
        (x + 10, y + 26, w - 20, h - 64)
    };
    cv.rect(gx, gy, gw, gh, [10, 12, 22]);
    curve(cv, gx, gw, gy, gh, &s.rx_hist, scale, CYAN, 70);
    curve(cv, gx, gw, gy, gh, &s.tx_hist, scale, MAGENTA, 0);
    text(cv, &f.tech, 11.0, gx + gw, y + 18, &format!("MAX {}", rate(scale as f64)), G_DIM, 2);
    if side > 0 {
        let tx0 = x + w - side - 10;
        text(cv, &f.tech, 12.0, tx0, y + 34, "DOWN", G_DIM, 0);
        text(cv, &f.tech, 20.0, tx0, y + 56, &rate(s.rx), CYAN, 0);
        text(cv, &f.tech, 12.0, tx0, y + 74, "UP", G_DIM, 0);
        text(cv, &f.tech, 20.0, tx0, y + 96, &rate(s.tx), MAGENTA, 0);
    } else {
        let by = gy + gh + 26;
        text(cv, &f.tech, 20.0, x + 10, by, &format!("DOWN {}", rate(s.rx)), CYAN, 0);
        text(cv, &f.tech, 20.0, x + w - 10, by, &format!("UP {}", rate(s.tx)), MAGENTA, 2);
    }
}

fn gamer_footer(cv: &mut Canvas, f: &Fonts, cx: i32, y: i32) {
    text(cv, &f.tech, 11.0, cx, y, "TURZX // NATIF", [50, 60, 85], 1);
}

fn disk_pct(s: &Snapshot) -> Option<(f32, u64)> {
    s.disk.map(|(free, total)| (100.0 * (total - free) as f32 / total as f32, free))
}

fn render_gamer_landscape(cv: &mut Canvas, f: &Fonts, s: &Snapshot) {
    header(cv, f, 480, s);
    let gpu = s.gpu.as_ref().map(|g| g.load).unwrap_or(0.0);
    let (cy, r) = (116, 38.0);
    gauge(cv, f, 66, cy, r, s.cpu, "CPU", "CHARGE");
    gauge(cv, f, 182, cy, r, gpu, "GPU", &gpu_gauge_detail(s));
    gauge(cv, f, 298, cy, r, s.mem.load as f32, "RAM", &mem_detail(s));
    match disk_pct(s) {
        Some((pct, free)) => gauge(cv, f, 414, cy, r, pct, "C:", &format!("{:.0} GO LIBRES", gib(free))),
        None => gauge(cv, f, 414, cy, r, 0.0, "C:", "N/D"),
    }

    net_panel(cv, f, M, 200, 480 - 2 * M, 104, 120, s);
    gamer_footer(cv, f, 240, 316);
    cv.scanlines();
}

fn render_gamer_portrait(cv: &mut Canvas, f: &Fonts, s: &Snapshot) {
    header(cv, f, 320, s);
    let gpu = s.gpu.as_ref().map(|g| g.load).unwrap_or(0.0);
    gauge(cv, f, 86, 120, 48.0, s.cpu, "CPU", "CHARGE");
    gauge(cv, f, 234, 120, 48.0, gpu, "GPU", &gpu_gauge_detail(s));

    let inner = 320 - 2 * M;
    seg_row(cv, f, M, inner, 228, "RAM", &mem_detail(s), s.mem.load as f32);
    if let Some((pct, free)) = disk_pct(s) {
        seg_row(cv, f, M, inner, 272, "C:", &format!("{:.0} GO LIBRES", gib(free)), pct);
    }
    net_panel(cv, f, M, 304, inner, 152, 0, s);
    gamer_footer(cv, f, 160, 474);
    cv.scanlines();
}

// ---------------------------------------------------------------- boucle

struct Sampler {
    adl: Option<Adl>,
    cpu: sys::CpuSampler,
    net: sys::NetSampler,
    cpu_hist: Sweep,
    rx_hist: Sweep,
    tx_hist: Sweep,
    /// Valeurs affichées (avec zone morte) : CPU, GPU, débits.
    shown: (f32, f32, f64, f64),
}

/// Zone morte relative pour les débits : ±15 %, ou retour à ~0.
fn settle_rate(shown: &mut f64, raw: f64) -> f64 {
    if (raw - *shown).abs() > 0.15 * shown.max(1024.0) {
        *shown = raw;
    }
    *shown
}

impl Sampler {
    fn new() -> Self {
        Sampler {
            adl: Adl::open(),
            cpu: sys::CpuSampler::new(),
            net: sys::NetSampler::new(),
            cpu_hist: Sweep::new(),
            rx_hist: Sweep::new(),
            tx_hist: Sweep::new(),
            shown: (0.0, 0.0, 0.0, 0.0),
        }
    }

    fn snap(&mut self) -> Snapshot {
        let cpu = self.cpu.sample();
        self.cpu_hist.push(cpu);
        let (rx, tx) = self.net.sample();
        self.rx_hist.push(rx as f32);
        self.tx_hist.push(tx as f32);
        let mut gpu = self.adl.as_ref().and_then(read_gpu);
        if let Some(g) = gpu.as_mut() {
            g.load = settle(&mut self.shown.1, g.load);
        }
        Snapshot {
            now: sys::local_time(),
            cpu: settle(&mut self.shown.0, cpu),
            cpu_hist: self.cpu_hist.clone(),
            gpu,
            mem: sys::memory(),
            disk: sys::disk("C:\\"),
            rx: settle_rate(&mut self.shown.2, rx),
            tx: settle_rate(&mut self.shown.3, tx),
            rx_hist: self.rx_hist.clone(),
            tx_hist: self.tx_hist.clone(),
        }
    }
}

fn render(f: &Fonts, s: &Snapshot, o: Orientation, theme: Theme) -> Canvas {
    let (w, h) = o.dims();
    let landscape = w > h;
    let bg = if theme == Theme::Gamer { G_BG } else { BG };
    let mut cv = Canvas::new(w as usize, h as usize, bg);
    match (theme, landscape) {
        (Theme::Classic, false) => render_portrait(&mut cv, f, s),
        (Theme::Classic, true) => render_landscape(&mut cv, f, s),
        (Theme::Gamer, false) => render_gamer_portrait(&mut cv, f, s),
        (Theme::Gamer, true) => render_gamer_landscape(&mut cv, f, s),
    }
    cv
}

/// Rend une image du tableau de bord dans un PNG, sans écran (quantifiée RGB565 comme à l'affichage).
pub fn preview(path: &str, o: Orientation, theme: Theme) -> Result<(), Box<dyn std::error::Error>> {
    let f = Fonts::load()?;
    let mut sampler = Sampler::new();
    let mut s = sampler.snap();
    for _ in 0..8 {
        sleep(Duration::from_millis(150));
        s = sampler.snap();
    }
    let cv = render(&f, &s, o, theme);
    let raw: Vec<u8> = cv
        .to565()
        .iter()
        .flat_map(|&v| {
            let (r, g, b) = ((v >> 11) as u8, (v >> 5 & 0x3F) as u8, (v & 0x1F) as u8);
            [r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2]
        })
        .collect();
    image::RgbImage::from_raw(cv.w as u32, cv.h as u32, raw).ok_or("tampon invalide")?.save(path)?;
    Ok(())
}

/// Boucle du tableau de bord. Ne rend la main qu'en cas d'erreur fatale (police absente).
pub fn monitor(port: Option<String>, interval: u64, orientation: Orientation, theme: Theme) -> std::io::Result<()> {
    let f = Fonts::load()?;
    let mut sampler = Sampler::new();
    let mut display: Option<Display> = None;

    loop {
        if display.is_none() {
            match open(port.as_deref()).and_then(|mut s| {
                s.on()?;
                s.set_orientation(orientation, false)?;
                Ok(s)
            }) {
                Ok(s) => display = Some(Display::new(s)),
                Err(e) => {
                    eprintln!("écran indisponible ({e}), nouvel essai dans 3 s");
                    sleep(Duration::from_secs(3));
                    continue;
                }
            }
        }

        // Écran débranché : une écriture n'échoue pas toujours, mais garder le port ouvert
        // empêche Windows de redémarrer le périphérique au rebranchement (CM_PROB_FAILED_START).
        if port.is_none() && crate::find_port().is_none() {
            eprintln!("écran débranché, port libéré");
            display = None;
            sleep(Duration::from_secs(2));
            continue;
        }

        let snap = sampler.snap();
        let cv = render(&f, &snap, orientation, theme);

        let d = display.as_mut().unwrap();
        match d.present(&cv.to565()) {
            Ok(_) => {}
            Err(e) => {
                // Câble débranché / écran redémarré : on rouvrira au tour suivant.
                eprintln!("écriture échouée ({e}), reconnexion…");
                display = None;
                sleep(Duration::from_secs(2));
                continue;
            }
        }

        // Cale le rafraîchissement sur le début de la seconde suivante.
        let ms = if interval == 1000 { 1000 - sys::local_time().ms as u64 } else { interval };
        sleep(Duration::from_millis(ms.max(10)));
    }
}

/// Mesure le coût d'envoi réel du rendu, seconde par seconde, sans écran.
pub fn bench(o: Orientation, theme: Theme, frames: usize) -> Result<(), Box<dyn std::error::Error>> {
    let f = Fonts::load()?;
    let mut sampler = Sampler::new();
    let (w, h) = o.dims();
    let mut prev = render(&f, &sampler.snap(), o, theme).to565();
    let mut total = 0;
    for i in 0..frames {
        sleep(Duration::from_secs(1));
        let cur = render(&f, &sampler.snap(), o, theme).to565();
        let (n, bytes) = crate::diff_cost(&prev, &cur, w as usize, h as usize);
        println!("seconde {:2} : {n:2} rectangles, {:6.1} Ko (≈ {:.0} ms de transfert)", i + 1, bytes as f32 / 1024.0, bytes as f32 / 160.0);
        total += bytes;
        prev = cur;
    }
    println!("moyenne : {:.1} Ko/s", total as f32 / frames as f32 / 1024.0);
    Ok(())
}
