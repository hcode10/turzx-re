use std::process::exit;
use std::time::Instant;

use turzx::{adl::Adl, dash, open, rgb565, Display, Orientation};

const USAGE: &str = "turzx [--port COMx] <commande>

Commandes :
  on | off                    allume / éteint l'écran
  brightness <0-100>          luminosité en %
  orientation <portrait|portrait-inv|paysage|paysage-inv> [--mirror]
  image <fichier>             affiche une image (png/jpg/bmp), recadrée plein écran
  clear                       écran noir
  beep [ms]                   buzzer
  monitor [--interval ms] [--orientation portrait|paysage|…] [--theme classique|gamer]
                              tableau de bord (heure, CPU, GPU, RAM, disque, réseau)
  preview [-o fichier.png] [--orientation …] [--theme …]
                              rend le tableau de bord dans un PNG, sans écran
  gpu                         liste les capteurs AMD (diagnostic)";

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let port = take_opt(&mut args, "--port");
    if args.is_empty() {
        eprintln!("{USAGE}");
        exit(2);
    }
    if let Err(e) = run(port, args) {
        eprintln!("erreur : {e}");
        exit(1);
    }
}

fn take_opt(args: &mut Vec<String>, name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    args.remove(i);
    (i < args.len()).then(|| args.remove(i))
}

fn take_flag(args: &mut Vec<String>, name: &str) -> bool {
    match args.iter().position(|a| a == name) {
        Some(i) => {
            args.remove(i);
            true
        }
        None => false,
    }
}

type Res<T> = Result<T, Box<dyn std::error::Error>>;

/// `--orientation` et `--theme` du tableau de bord.
fn look(args: &mut Vec<String>) -> Res<(Orientation, dash::Theme)> {
    let o = match take_opt(args, "--orientation") {
        Some(n) => Orientation::from_name(&n).ok_or("orientation inconnue")?,
        None => Orientation::Portrait,
    };
    let theme = match take_opt(args, "--theme") {
        Some(n) => dash::Theme::from_name(&n).ok_or("thème inconnu (classique | gamer)")?,
        None => dash::Theme::Classic,
    };
    Ok((o, theme))
}

fn run(port: Option<String>, mut args: Vec<String>) -> Res<()> {
    let command = args.remove(0);
    match command.as_str() {
        "monitor" => {
            let interval = take_opt(&mut args, "--interval").map(|v| v.parse()).transpose()?.unwrap_or(1000u64);
            let (o, theme) = look(&mut args)?;
            return Ok(dash::monitor(port, interval, o, theme)?);
        }
        "bench" => {
            let (o, theme) = look(&mut args)?;
            let secs = args.first().map(|a| a.parse()).transpose()?.unwrap_or(8);
            dash::bench(o, theme, secs)?;
            return Ok(());
        }
        "preview" => {
            let path = take_opt(&mut args, "-o").unwrap_or_else(|| "preview.png".into());
            let (o, theme) = look(&mut args)?;
            dash::preview(&path, o, theme)?;
            println!("aperçu écrit dans {path}");
            return Ok(());
        }
        "gpu" => {
            let adl = Adl::open().ok_or("ADL indisponible (pilote AMD absent ?)")?;
            println!("adaptateur {} : {}", adl.adapter, adl.name);
            for (i, (ok, v)) in adl.sensors().ok_or("lecture PMLog échouée")?.iter().enumerate() {
                if *ok {
                    println!("  capteur {i:3} = {v}");
                }
            }
            return Ok(());
        }
        "stats" => {
            let mut cpu = turzx::sys::CpuSampler::new();
            let mut net = turzx::sys::NetSampler::new();
            std::thread::sleep(std::time::Duration::from_secs(1));
            let (rx, tx) = net.sample();
            let m = turzx::sys::memory();
            println!("cpu {:.1} % · ram {} % · réseau ↓ {:.0} o/s ↑ {:.0} o/s", cpu.sample(), m.load, rx, tx);
            if take_flag(&mut args, "-v") {
                for (alias, flags, status, rx) in turzx::sys::NetSampler::interfaces() {
                    println!("  {alias:40} flags={flags:08b} état={status} reçu={rx}");
                }
            }
            return Ok(());
        }
        _ => {}
    }

    let mut s = open(port.as_deref())?;
    match command.as_str() {
        "on" => s.on()?,
        "off" => s.off()?,
        "brightness" => s.brightness(args.first().ok_or("valeur manquante")?.parse()?)?,
        "orientation" => {
            let mirror = take_flag(&mut args, "--mirror");
            let o = args.first().and_then(|n| Orientation::from_name(n)).ok_or("orientation inconnue")?;
            s.set_orientation(o, mirror)?;
        }
        "image" => {
            let path = args.first().ok_or("fichier manquant")?;
            s.set_orientation(Orientation::Portrait, false)?;
            let img = image::open(path)?
                .resize_to_fill(s.width as u32, s.height as u32, image::imageops::FilterType::Triangle)
                .to_rgb8();
            let fb: Vec<u16> = img.pixels().map(|p| rgb565(p[0], p[1], p[2])).collect();
            let t = Instant::now();
            let n = Display::new(s).present(&fb)?;
            println!("{n} octets envoyés en {:.2?}", t.elapsed());
        }
        "clear" => s.send(turzx::cmd::TO_BLACK, 0, 0, 0, 0)?,
        "beep" => s.beep(args.first().map(|a| a.parse()).transpose()?.unwrap_or(150))?,
        _ => return Err(format!("commande inconnue : {command}\n\n{USAGE}").into()),
    }
    Ok(())
}
