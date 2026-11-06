//! Tema "brasa": fundos em carvão (preto quente com um fio de vermelho), cinzas de cinza e dois
//! acentos tirados da brasa: coral = algo rodando/ação de executar e a seção Sessões; âmbar = foco,
//! seleção, dicas de teclado e a seção Notas. Verde-água e vermelho seguem só como estado (ok / erro).

/// painel principal (terminal e notas): carvão

/// barra lateral

/// barra lateral translúcida (RRGGBBAA, 90% opaca = 10% de transparência): a área de trabalho aparece de leve por trás. O desfoque
/// vem do sistema: o app pede ao compositor (funciona no KDE); no GNOME, pela extensão Blur my Shell
/// aplicada ao app "brasa"

/// linha selecionada, chips

/// linhas finas de separação

/// hover de linhas de lista

/// hover de botões





/// brasa: o acento principal

/// pêssego: o miolo claro da brasa (gráfico de CPU, logo)

/// verde-água: só estado "ok" (carga baixa, processo rodando)
pub const TEAL: u32 = 0x7cc7b4;
/// fundos do seletor da barra lateral, um por seção (a cor da seção bem escurecida)


/// âmbar: a parte amarela da brasa (foco, seleção, dicas, Notas)






/// contorno de cartões e menus flutuantes

/// linhas divisórias entre painéis

/// faixa da linha de comando de cada bloco (como a linha do prompt destacada)


/// bloco com erro: faixa vermelha e fundo levemente avermelhado
pub const ERROR_BG: u32 = 0x2a1517;





/// linhas que separam blocos na grade crua



// paleta ANSI (base do Warp, AGPL-3.0, veja NOTICE.md)
pub const GREEN: u32 = 0xb4fa72;
pub const YELLOW: u32 = 0xfefdc2;
pub const PURPLE: u32 = 0xff8ffd;
pub const RED: u32 = 0xff8272;

pub const WIN_CLOSE_HOVER: u32 = 0xe81123;

// ---------------------------------------------------------------------------------------------
// Cor do tema: a paleta acima é a vermelha (brasa). Azul, verde e roxo giram o matiz de todas as
// cores com tom (fundos, textos, bordas, acentos), mantendo brilho e saturação; verde-água, vermelho
// de erro e a paleta ANSI ficam como estão. Cada cor é uma função `c_*` que lê a escolha atual.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// (nome do arquivo/logo, rótulo, giro de matiz em graus, cor da bolinha no seletor)
pub const ACCENTS: [(&str, &str, f32, u32); 4] = [
    ("red", "Vermelho", 0.0, 0xe8805f),
    ("blue", "Azul", 210.0, 0x5f9be8),
    ("green", "Verde", 120.0, 0x5fe88a),
    ("purple", "Roxo", 270.0, 0xa55fe8),
];

/// índice de "cor própria" (hexadecimal ou mapa de matizes), depois das quatro prontas
pub const CUSTOM: usize = 4;

/// (nome salvo, rótulo, fator de brilho aplicado só aos fundos); depois deles, o índice 3 é a cor própria
pub const BACKGROUNDS: [(&str, &str, f32); 3] = [("charcoal", "Carvão", 1.0), ("black", "Preto", 0.5), ("graphite", "Grafite", 1.8)];
pub const BG_CUSTOM: usize = 3;
/// fundo padrão (carvão), base para o ganho por canal do fundo próprio
const BG_BASE: u32 = 0x161213;
/// o coral original: a cor própria gira o matiz a partir dele
const CORAL_BASE: u32 = 0xe8805f;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static CURRENT_BG: AtomicUsize = AtomicUsize::new(0);
static CUSTOM_ACCENT: AtomicU32 = AtomicU32::new(0xe8805f);
static CUSTOM_BG: AtomicU32 = AtomicU32::new(0x161213);

pub fn bg_index() -> usize {
    CURRENT_BG.load(Ordering::Relaxed)
}

pub fn set_bg(i: usize) {
    CURRENT_BG.store(i.min(BG_CUSTOM), Ordering::Relaxed);
    save();
}

pub fn custom_bg() -> u32 {
    CUSTOM_BG.load(Ordering::Relaxed)
}

pub fn set_custom_bg(hex: u32) {
    CUSTOM_BG.store(hex & 0xff_ffff, Ordering::Relaxed);
    CURRENT_BG.store(BG_CUSTOM, Ordering::Relaxed);
    save();
}

pub fn accent_index() -> usize {
    CURRENT.load(Ordering::Relaxed)
}

pub fn custom_accent() -> u32 {
    CUSTOM_ACCENT.load(Ordering::Relaxed)
}

pub fn set_accent(i: usize) {
    CURRENT.store(i.min(ACCENTS.len() - 1), Ordering::Relaxed);
    save();
}

pub fn set_custom_accent(hex: u32) {
    CUSTOM_ACCENT.store(hex & 0xff_ffff, Ordering::Relaxed);
    CURRENT.store(CUSTOM, Ordering::Relaxed);
    save();
}

/// Giro de matiz (graus) da cor atual em relação ao coral original.
fn accent_deg() -> f32 {
    let i = accent_index();
    if i == CUSTOM {
        (hue_of(custom_accent()) - hue_of(CORAL_BASE)).rem_euclid(360.0)
    } else {
        ACCENTS[i].2
    }
}

/// Logo na cor atual: há uma imagem a cada 30° de matiz; a cor própria usa a mais próxima.
pub fn logo_path() -> String {
    let step = ((accent_deg() / 30.0).round() as u32 % 12) * 30;
    format!("brand/brasa-logo-h{step}.png")
}

/// Matiz (0–360) de uma cor RRGGBB; cinzas dão 0.
pub fn hue_of(rgb: u32) -> f32 {
    let (r, g, b) = (((rgb >> 16) & 0xff) as f32 / 255.0, ((rgb >> 8) & 0xff) as f32 / 255.0, (rgb & 0xff) as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    if d < 1e-6 {
        return 0.0;
    }
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    h * 60.0
}

/// Cor RRGGBB a partir de matiz (0–360), saturação e luminosidade (0–1).
pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> u32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    (to(r) << 16) | (to(g) << 8) | to(b)
}

/// Lê "#rrggbb", "rrggbb" ou "#rgb".
pub fn parse_hex(s: &str) -> Option<u32> {
    let h = s.trim().trim_start_matches('#');
    match h.len() {
        6 => u32::from_str_radix(h, 16).ok(),
        3 => {
            let v = u32::from_str_radix(h, 16).ok()?;
            let (r, g, b) = ((v >> 8) & 0xf, (v >> 4) & 0xf, v & 0xf);
            Some(((r * 17) << 16) | ((g * 17) << 8) | (b * 17))
        }
        _ => None,
    }
}

pub fn hex_string(rgb: u32) -> String {
    format!("#{:06x}", rgb & 0xff_ffff)
}

fn save() {
    let accent = if accent_index() == CUSTOM { "custom" } else { ACCENTS[accent_index()].0 };
    let bg = if bg_index() == BG_CUSTOM { "custom" } else { BACKGROUNDS[bg_index()].0 };
    let text = format!("accent={accent}\naccent_hex={}\nbg={bg}\nbg_hex={}\n", hex_string(custom_accent()), hex_string(custom_bg()));
    let _ = std::fs::write(crate::config_dir().join("theme.txt"), text);
}

/// Lê a escolha salva (`~/.config/brasa/theme.txt`); sem arquivo, fica no vermelho com fundo carvão.
pub fn load_accent() {
    let saved = std::fs::read_to_string(crate::config_dir().join("theme.txt")).unwrap_or_default();
    // formato antigo: duas linhas, nome da cor e nome do fundo
    let legacy = !saved.contains('=');
    let mut plain = saved.lines().map(str::trim);
    let (mut accent, mut bg) = (String::new(), String::new());
    if legacy {
        accent = plain.next().unwrap_or("").to_string();
        bg = plain.next().unwrap_or("").to_string();
    }
    for line in saved.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        match k.trim() {
            "accent" => accent = v.trim().to_string(),
            "bg" => bg = v.trim().to_string(),
            "accent_hex" => {
                if let Some(c) = parse_hex(v) {
                    CUSTOM_ACCENT.store(c, Ordering::Relaxed);
                }
            }
            "bg_hex" => {
                if let Some(c) = parse_hex(v) {
                    CUSTOM_BG.store(c, Ordering::Relaxed);
                }
            }
            _ => {}
        }
    }
    if accent == "custom" {
        CURRENT.store(CUSTOM, Ordering::Relaxed);
    } else if let Some(i) = ACCENTS.iter().position(|a| a.0 == accent) {
        CURRENT.store(i, Ordering::Relaxed);
    }
    if bg == "custom" {
        CURRENT_BG.store(BG_CUSTOM, Ordering::Relaxed);
    } else if let Some(i) = BACKGROUNDS.iter().position(|b| b.0 == bg) {
        CURRENT_BG.store(i, Ordering::Relaxed);
    }
}

/// Cor de fundo que cada opção teria (para os mosaicos de escolha), sem trocar a atual.
pub fn bg_preview(i: usize) -> u32 {
    if i == BG_CUSTOM {
        custom_bg()
    } else {
        let c = shift(BG_BASE);
        let f = BACKGROUNDS[i.min(BACKGROUNDS.len() - 1)].2;
        let ch = |s: u32| ((((c >> s) & 0xff) as f32 * f).round().min(255.0)) as u32;
        (ch(16) << 16) | (ch(8) << 8) | ch(0)
    }
}

/// Fundos: preto / carvão / grafite escalam o brilho; a cor própria troca o tom de todos os fundos
/// (cada canal cresce na proporção entre a cor escolhida e o carvão padrão).
fn shift_bg(c: u32) -> u32 {
    let (rgb, alpha) = if c > 0xff_ffff { (c >> 8, Some(c & 0xff)) } else { (c, None) };
    let out = if bg_index() == BG_CUSTOM {
        let want = custom_bg();
        let ch = |s: u32| {
            let (base, own, v) = ((BG_BASE >> s) & 0xff, (want >> s) & 0xff, (rgb >> s) & 0xff);
            ((v as f32 * own.max(1) as f32 / base.max(1) as f32).round().min(255.0)) as u32
        };
        (ch(16) << 16) | (ch(8) << 8) | ch(0)
    } else {
        let c = shift(rgb);
        let f = BACKGROUNDS[bg_index()].2;
        let ch = |s: u32| ((((c >> s) & 0xff) as f32 * f).round().min(255.0)) as u32;
        (ch(16) << 16) | (ch(8) << 8) | ch(0)
    };
    match alpha {
        Some(a) => (out << 8) | a,
        None => out,
    }
}

/// Gira o matiz de `c` (RRGGBB ou RRGGBBAA) pelo giro da cor escolhida.
fn shift(c: u32) -> u32 {
    let deg = accent_deg();
    if deg == 0.0 {
        return c;
    }
    let (rgb, alpha) = if c > 0xff_ffff { (c >> 8, Some(c & 0xff)) } else { (c, None) };
    let (r, g, b) = (((rgb >> 16) & 0xff) as f32 / 255.0, ((rgb >> 8) & 0xff) as f32 / 255.0, (rgb & 0xff) as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    let out = if d < 1e-6 {
        rgb
    } else {
        let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
        let h = if max == r {
            ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        } * 60.0;
        let h = (h + deg).rem_euclid(360.0);
        let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
        let p = 2.0 * l - q;
        let ch = |t: f32| {
            let t = t.rem_euclid(1.0);
            let v = if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 0.5 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            };
            (v * 255.0).round() as u32
        };
        let hk = h / 360.0;
        (ch(hk + 1.0 / 3.0) << 16) | (ch(hk) << 8) | ch(hk - 1.0 / 3.0)
    };
    match alpha {
        Some(a) => (out << 8) | a,
        None => out,
    }
}

pub fn c_bg() -> u32 { shift_bg(0x161213) }
pub fn c_panel() -> u32 { shift_bg(0x211a1a) }
pub fn c_panel_glass() -> u32 { shift_bg(0x211a1ae6) }
pub fn c_surface() -> u32 { shift(0x3a2b27) }
pub fn c_border() -> u32 { shift(0x2c2322) }
pub fn c_hover() -> u32 { shift(0x2a201f) }
pub fn c_btn_hover() -> u32 { shift(0x3d2e2a) }
pub fn c_text() -> u32 { shift(0xefe6e2) }
pub fn c_muted() -> u32 { shift(0xa3938e) }
pub fn c_disabled() -> u32 { shift(0x6f625e) }
pub fn c_coral() -> u32 { shift(0xe8805f) }
pub fn c_peach() -> u32 { shift(0xf5b48e) }
pub fn c_coral_dim() -> u32 { shift(0x4a2a21) }
pub fn c_amber_dim() -> u32 { shift(0x3d2c16) }
pub fn c_amber() -> u32 { shift(0xe9b25f) }
pub fn c_run_bg() -> u32 { shift(0xc0593f) }
pub fn c_run_hover() -> u32 { shift(0xd2694d) }
pub fn c_code_bg() -> u32 { shift_bg(0x1f1817) }
pub fn c_card_border() -> u32 { shift(0x40312d) }
pub fn c_divider() -> u32 { shift(0x3a2d2a) }
pub fn c_cmd_bg() -> u32 { shift_bg(0x251c1b) }
pub fn c_out_text() -> u32 { shift(0xe8deda) }
pub fn c_selected_block_bg() -> u32 { shift(0x2a1d19) }
pub fn c_term_fg() -> u32 { shift(0xefe6e2) }
pub fn c_selection() -> u32 { shift(0x5a3427) }
pub fn c_sep_active() -> u32 { shift(0x7a4433) }
pub fn c_deep_bg() -> u32 { shift_bg(0x0f0b0b) }
pub fn c_amber_glow() -> u32 { shift(0xe9b25f33) }
pub fn c_accent() -> u32 { c_amber() }
pub fn c_term_bg() -> u32 { c_bg() }
pub fn c_sep() -> u32 { c_border() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors_are_parsed_and_printed() {
        assert_eq!(parse_hex("#e8805f"), Some(0xe8805f));
        assert_eq!(parse_hex("E8805F"), Some(0xe8805f));
        assert_eq!(parse_hex("  #fa0 "), Some(0xffaa00));
        assert_eq!(parse_hex("#12345"), None);
        assert_eq!(parse_hex("#gg0000"), None);
        assert_eq!(hex_string(0x161213), "#161213");
        assert_eq!(hex_string(0xff), "#0000ff");
    }

    #[test]
    fn hue_and_hsl_agree() {
        assert_eq!(hue_of(0xff0000) as u32, 0);
        assert_eq!(hue_of(0x00ff00) as u32, 120);
        assert_eq!(hue_of(0x0000ff) as u32, 240);
        assert_eq!(hue_of(0x808080), 0.0);
        for h in [0.0, 30.0, 95.0, 180.0, 250.0, 330.0] {
            let back = hue_of(hsl_to_rgb(h, 0.72, 0.58));
            assert!((back - h).abs() < 2.0, "{h} voltou como {back}");
        }
        assert_eq!(hsl_to_rgb(0.0, 0.0, 1.0), 0xffffff);
    }

    #[test]
    fn red_theme_keeps_the_original_palette() {
        // sem trocar a cor (o teste não grava nada em ~/.config), o vermelho é a paleta de fábrica
        assert_eq!(c_bg(), 0x161213);
        assert_eq!(c_coral(), 0xe8805f);
        assert_eq!(c_panel_glass(), 0x211a1ae6);
        assert_eq!(logo_path(), "brand/brasa-logo-h0.png");
    }

    #[test]
    fn custom_background_scales_each_channel() {
        let want = 0x203040;
        let ch = |s: u32| ((BG_BASE >> s) & 0xff, (want >> s) & 0xff);
        let (base, own) = ch(0);
        let scaled = (0x161213u32 & 0xff) as f32 * own as f32 / base as f32;
        assert!((scaled - 0x40 as f32).abs() < 1.0);
        // o fundo próprio na cor do carvão não muda nada
        assert_eq!(bg_preview(0), 0x161213);
        assert!(bg_preview(1) < bg_preview(0) && bg_preview(2) > bg_preview(0));
    }

    #[test]
    fn logos_exist_for_every_thirty_degrees() {
        for deg in (0..360).step_by(30) {
            let path = format!("brand/brasa-logo-h{deg}.png");
            assert!(crate::assets::Assets.load_path(&path).is_some(), "falta {path}");
        }
    }
}
