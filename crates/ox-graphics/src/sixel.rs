//! Decodificador de Sixel (`ESC P <p1;p2;p3> q <dados> ESC \`), o formato de imagem em bitmap de terminais DEC
//! (`img2sixel`, `chafa -f sixel`, gnuplot...). Só lógica: recebe o corpo da sequência e devolve pixels RGBA.
//!
//! Cada caractere de dados `?`..`~` desenha 6 pixels em coluna (6 bits) na cor escolhida; `!n` repete o caractere
//! n vezes; `$` volta ao começo da faixa; `-` desce uma faixa (6 linhas); `#n` escolhe a cor n e `#n;2;r;g;b`
//! (0–100) ou `#n;1;h;l;s` a define; `"pan;pad;largura;altura` declara o tamanho. Pixel não pintado fica
//! transparente (o fundo do terminal aparece).

use crate::{Error, Image, MAX_PIXELS};

/// Maior largura ou altura aceita, para uma imagem maluca não pedir gigabytes enquanto cresce.
const MAX_SIDE: usize = 16_384;

/// As 16 cores iniciais do VT340.
const DEFAULT_PALETTE: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (51, 51, 204),
    (204, 33, 33),
    (51, 204, 51),
    (204, 51, 204),
    (51, 204, 204),
    (204, 204, 51),
    (135, 135, 135),
    (66, 66, 66),
    (84, 84, 153),
    (153, 66, 66),
    (84, 153, 84),
    (153, 84, 153),
    (84, 153, 153),
    (153, 153, 84),
    (204, 204, 204),
];

fn pct(v: u32) -> u8 {
    ((v.min(100) * 255 + 50) / 100) as u8
}

/// HSL do Sixel (matiz 0–360 com 0 = azul, luz e saturação 0–100) para RGB.
fn hsl(h: u32, l: u32, s: u32) -> (u8, u8, u8) {
    let (h, l, s) = (((h + 240) % 360) as f32, l.min(100) as f32 / 100.0, s.min(100) as f32 / 100.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let f = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (f(r), f(g), f(b))
}

/// Números separados por `;` que começam em `i`; devolve-os e o índice do primeiro byte depois.
fn numbers(d: &[u8], mut i: usize) -> (Vec<u32>, usize) {
    let (mut out, mut cur, mut any) = (Vec::new(), 0u32, false);
    while i < d.len() {
        match d[i] {
            b'0'..=b'9' => {
                cur = cur.saturating_mul(10).saturating_add((d[i] - b'0') as u32);
                any = true;
            }
            b';' => {
                out.push(cur);
                cur = 0;
                any = false;
            }
            _ => break,
        }
        i += 1;
    }
    if any || !out.is_empty() {
        out.push(cur);
    }
    (out, i)
}

pub fn decode(body: &[u8], uid: u64) -> Result<Image, Error> {
    // parâmetros (P1;P2;P3) e o `q`
    let q = body.iter().position(|&b| !(b.is_ascii_digit() || b == b';')).filter(|&i| body[i] == b'q').ok_or(Error::Data)?;
    let d = &body[q + 1..];
    let mut palette = [(0u8, 0u8, 0u8); 256];
    palette[..16].copy_from_slice(&DEFAULT_PALETTE);
    let (mut color, mut x, mut band) = (0usize, 0usize, 0usize);
    let (mut w, mut h) = (0usize, 0usize);
    // pixels como índice de cor + 1 (0 = não pintado); cresce conforme necessário
    let mut px: Vec<u16> = Vec::new();
    let mut stride = 0usize;
    let mut declared: Option<(usize, usize)> = None;

    let mut ensure = |px: &mut Vec<u16>, stride: &mut usize, need_w: usize, need_h: usize| -> Result<(), Error> {
        if need_w > MAX_SIDE || need_h > MAX_SIDE || (need_w as u64) * (need_h as u64) > MAX_PIXELS {
            return Err(Error::TooBig);
        }
        let new_stride = (*stride).max(need_w);
        let rows = (px.len() / (*stride).max(1)).max(need_h);
        if new_stride != *stride && *stride != 0 {
            let mut grown = vec![0u16; new_stride * rows];
            for r in 0..px.len() / *stride {
                grown[r * new_stride..r * new_stride + *stride].copy_from_slice(&px[r * *stride..(r + 1) * *stride]);
            }
            *px = grown;
        } else if new_stride * rows > px.len() {
            px.resize(new_stride * rows, 0);
        }
        *stride = new_stride;
        Ok(())
    };

    let mut i = 0;
    while i < d.len() {
        match d[i] {
            b'"' => {
                let (n, next) = numbers(d, i + 1);
                if n.len() >= 4 && n[2] > 0 && n[3] > 0 {
                    declared = Some((n[2] as usize, n[3] as usize));
                    ensure(&mut px, &mut stride, n[2] as usize, n[3] as usize)?;
                }
                i = next;
                continue;
            }
            b'#' => {
                let (n, next) = numbers(d, i + 1);
                if let Some(&c) = n.first() {
                    color = (c as usize).min(255);
                    if n.len() >= 5 {
                        palette[color] = match n[1] {
                            1 => hsl(n[2], n[3], n[4]),
                            _ => (pct(n[2]), pct(n[3]), pct(n[4])),
                        };
                    }
                }
                i = next;
                continue;
            }
            b'!' => {
                let (n, next) = numbers(d, i + 1);
                let count = n.first().copied().unwrap_or(1).max(1) as usize;
                if let Some(&c) = d.get(next).filter(|c| (b'?'..=b'~').contains(c)) {
                    paint(&mut px, &mut stride, &mut ensure, x, band, count, c - b'?', color)?;
                    x += count;
                    w = w.max(x);
                    h = h.max(band * 6 + 6);
                    i = next + 1;
                } else {
                    i = next;
                }
                continue;
            }
            b'$' => x = 0,
            b'-' => {
                x = 0;
                band += 1;
            }
            c @ b'?'..=b'~' => {
                paint(&mut px, &mut stride, &mut ensure, x, band, 1, c - b'?', color)?;
                x += 1;
                w = w.max(x);
                h = h.max(band * 6 + 6);
            }
            _ => {}
        }
        i += 1;
    }
    if let Some((dw, dh)) = declared {
        w = w.max(dw);
        h = h.max(dh);
    }
    if w == 0 || h == 0 {
        return Err(Error::Data);
    }
    ensure(&mut px, &mut stride, w, h)?;
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for xx in 0..w {
            let v = px.get(y * stride + xx).copied().unwrap_or(0);
            if v != 0 {
                let (r, g, b) = palette[(v - 1) as usize];
                rgba[(y * w + xx) * 4..][..4].copy_from_slice(&[r, g, b, 255]);
            }
        }
    }
    Ok(Image { uid, width: w as u32, height: h as u32, rgba })
}

type Ensure<'a> = dyn FnMut(&mut Vec<u16>, &mut usize, usize, usize) -> Result<(), Error> + 'a;

/// Pinta `count` colunas a partir de `x` na faixa `band` com os 6 bits de `bits` na cor `color`.
#[allow(clippy::too_many_arguments)]
fn paint(px: &mut Vec<u16>, stride: &mut usize, ensure: &mut Ensure, x: usize, band: usize, count: usize, bits: u8, color: usize) -> Result<(), Error> {
    ensure(px, stride, x + count, band * 6 + 6)?;
    if bits == 0 {
        return Ok(());
    }
    for k in 0..6 {
        if bits & (1 << k) != 0 {
            let row = (band * 6 + k) * *stride;
            for c in x..x + count {
                px[row + c] = color as u16 + 1;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(img: &Image, x: usize, y: usize) -> [u8; 4] {
        let i = (y * img.width as usize + x) * 4;
        img.rgba[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn paints_columns_in_the_chosen_color_and_leaves_the_rest_transparent() {
        // 2 colunas: toda a coluna (bits 111111 = '~') em vermelho puro, depois só o pixel de cima ('@' = 000001)
        let img = decode(b"0;1;0q#1;2;100;0;0#1~@", 7).unwrap();
        assert_eq!((img.width, img.height, img.uid), (2, 6, 7));
        for y in 0..6 {
            assert_eq!(at(&img, 0, y), [255, 0, 0, 255]);
        }
        assert_eq!(at(&img, 1, 0), [255, 0, 0, 255]);
        assert_eq!(at(&img, 1, 1)[3], 0);
    }

    #[test]
    fn repeat_cr_and_next_band() {
        // faixa 1: 4 colunas azuis ('!4~'), volta ($) e escreve uma verde por cima na 1ª coluna; faixa 2: 1 coluna
        let img = decode(b"q#0;2;0;0;100#1;2;0;100;0#0!4~$#1~-#0~", 1).unwrap();
        assert_eq!((img.width, img.height), (4, 12));
        assert_eq!(at(&img, 0, 0), [0, 255, 0, 255]);
        assert_eq!(at(&img, 3, 5), [0, 0, 255, 255]);
        assert_eq!(at(&img, 0, 6), [0, 0, 255, 255]);
        assert_eq!(at(&img, 1, 6)[3], 0);
    }

    #[test]
    fn raster_attributes_set_the_size_even_when_data_is_smaller() {
        let img = decode(b"q\"1;1;10;8#1;2;100;100;100#1~", 1).unwrap();
        assert_eq!((img.width, img.height), (10, 8));
    }

    #[test]
    fn hsl_and_default_palette_colors() {
        // matiz 120 no Sixel é vermelho; 240 é verde; 0 é azul
        let img = decode(b"q#1;1;120;50;100#1~#2;1;240;50;100#2~#3;1;0;50;100#3~#9~", 1).unwrap();
        assert_eq!(at(&img, 0, 0), [255, 0, 0, 255]);
        assert_eq!(at(&img, 1, 0), [0, 255, 0, 255]);
        assert_eq!(at(&img, 2, 0), [0, 0, 255, 255]);
        assert_eq!(at(&img, 3, 0), [84, 84, 153, 255], "cor 9 da paleta inicial");
    }

    #[test]
    fn broken_or_huge_input_is_refused_not_trusted() {
        assert_eq!(decode(b"sem q", 1).unwrap_err(), Error::Data);
        assert_eq!(decode(b"q", 1).unwrap_err(), Error::Data);
        assert_eq!(decode(b"q\"1;1;60000;60000#0~", 1).unwrap_err(), Error::TooBig);
        assert_eq!(decode(b"q#0;2;1;1;1!999999999~", 1).unwrap_err(), Error::TooBig);
        // lixo no meio é ignorado
        assert!(decode(b"q#0;2;100;0;0\x01~garbage?", 1).is_ok());
    }
}
