//! Variante sans console de `turzx monitor`, pour le démarrage automatique.
//! Arguments optionnels, dans n'importe quel ordre :
//!   portrait | portrait-inv | paysage | paysage-inv   et   classique | gamer
#![windows_subsystem = "windows"]

use turzx::{dash::Theme, Orientation};

fn main() {
    let mut o = Orientation::Portrait;
    let mut theme = Theme::Classic;
    for a in std::env::args().skip(1) {
        if let Some(v) = Orientation::from_name(&a) {
            o = v;
        } else if let Some(v) = Theme::from_name(&a) {
            theme = v;
        }
    }
    let _ = turzx::dash::monitor(None, 1000, o, theme);
}
