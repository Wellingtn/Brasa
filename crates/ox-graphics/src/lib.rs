//! Protocolo gráfico do Kitty (`ESC _ G <chaves> ; <dados em base64> ESC \`).
//!
//! Sem dependência de janela: separa essas sequências do fluxo de bytes do terminal ([`Splitter`]),
//! monta as transmissões em partes ([`Gfx`]), decodifica PNG/RGB/RGBA e guarda as imagens e onde cada
//! uma foi posta. Quem desenha e quem escreve na grade é o aplicativo.
//!
//! Limites, para uma imagem ruim ou enorme não travar nem encher a memória: cada transmissão até
//! [`MAX_PAYLOAD`] bytes, imagem até [`MAX_PIXELS`] pixels, todas as imagens juntas até [`MAX_STORE`]
//! bytes (as mais antigas saem primeiro). Só se aceita transmissão direta (`t=d`): ler arquivo ou
//! memória compartilhada a pedido de quem escreve no terminal seria abrir o disco para a máquina remota.

mod sixel;

use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine as _;

pub const MAX_PAYLOAD: usize = 48 * 1024 * 1024;
pub const MAX_PIXELS: u64 = 16 * 1024 * 1024;
pub const MAX_STORE: usize = 256 * 1024 * 1024;
/// Maior altura, em linhas, que uma imagem ocupa na grade.
pub const MAX_ROWS: usize = 60;

const ESC: u8 = 0x1b;

// ---------------------------------------------------------------------------------------------
// Separar as sequências do fluxo

/// Um pedaço do fluxo: texto comum para o terminal, o corpo de uma sequência APC (`ESC _ ... ESC \`) ou o de uma
/// sequência DCS de Sixel (`ESC P ... q ... ESC \`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Text(Vec<u8>),
    /// o que há entre `ESC _` e `ESC \`; vazio se passou do limite e foi descartado
    Apc(Vec<u8>),
    /// o que há entre `ESC P` e `ESC \` de uma imagem Sixel (parâmetros, `q` e dados); vazio se passou do limite.
    /// Outras sequências DCS (tmux, DECRQSS...) seguem como texto para o terminal.
    Sixel(Vec<u8>),
}

#[derive(Default)]
pub struct Splitter {
    /// dentro de uma sequência: `_` (APC) ou `P` (DCS); 0 = fora
    kind: u8,
    /// o último byte do pedaço anterior foi um ESC solto
    esc_pending: bool,
    buf: Vec<u8>,
    overflow: bool,
}

/// Parâmetros de um DCS de Sixel: dígitos e `;` e depois `q`.
fn is_sixel(body: &[u8]) -> bool {
    body.iter().position(|&b| !(b.is_ascii_digit() || b == b';')).is_some_and(|i| body[i] == b'q')
}

impl Splitter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Piece> {
        let mut out: Vec<Piece> = Vec::new();
        let mut text: Vec<u8> = Vec::new();
        for &b in bytes {
            if self.kind != 0 {
                if self.esc_pending {
                    self.esc_pending = false;
                    if b == b'\\' {
                        let kind = std::mem::take(&mut self.kind);
                        let body = if self.overflow { Vec::new() } else { std::mem::take(&mut self.buf) };
                        self.buf.clear();
                        self.overflow = false;
                        if !text.is_empty() {
                            out.push(Piece::Text(std::mem::take(&mut text)));
                        }
                        if kind == b'_' {
                            out.push(Piece::Apc(body));
                        } else if is_sixel(&body) || body.is_empty() {
                            out.push(Piece::Sixel(body));
                        } else {
                            // outro DCS: devolve inteiro ao terminal
                            let mut raw = vec![ESC, b'P'];
                            raw.extend_from_slice(&body);
                            raw.extend_from_slice(&[ESC, b'\\']);
                            out.push(Piece::Text(raw));
                        }
                        continue;
                    }
                    // ESC dentro da sequência que não fecha: faz parte dos dados
                    self.push_body(ESC);
                }
                if b == ESC {
                    self.esc_pending = true;
                } else {
                    self.push_body(b);
                }
            } else if self.esc_pending {
                self.esc_pending = false;
                if b == b'_' || b == b'P' {
                    if !text.is_empty() {
                        out.push(Piece::Text(std::mem::take(&mut text)));
                    }
                    self.kind = b;
                } else {
                    text.push(ESC);
                    if b == ESC {
                        self.esc_pending = true;
                    } else {
                        text.push(b);
                    }
                }
            } else if b == ESC {
                self.esc_pending = true;
            } else {
                text.push(b);
            }
        }
        if !text.is_empty() {
            out.push(Piece::Text(text));
        }
        out
    }

    fn push_body(&mut self, b: u8) {
        if self.buf.len() >= MAX_PAYLOAD + 512 {
            self.overflow = true;
        } else {
            self.buf.push(b);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Comando

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// `a=`: t (enviar), T (enviar e mostrar), p (mostrar), d (apagar), q (consultar)
    pub action: char,
    /// `f=`: 24, 32 ou 100 (PNG)
    pub format: u32,
    /// `t=`: d (direto), f, t, s
    pub medium: char,
    /// `i=`
    pub id: u32,
    /// `p=`
    pub placement: u32,
    /// `m=1`: vem mais uma parte
    pub more: bool,
    /// `q=`: 0 responde tudo, 1 só erros, 2 nada
    pub quiet: u8,
    pub width: u32,
    pub height: u32,
    pub cols: u32,
    pub rows: u32,
    /// `o=z`
    pub zlib: bool,
    /// `d=`
    pub delete: char,
    /// `C=1`: o cursor não anda
    pub keep_cursor: bool,
    /// bytes já decodificados do base64; cada parte de uma transmissão é decodificada sozinha, porque há
    /// programas (o `chafa`) que põem o preenchimento `=` no fim de cada parte
    pub payload: Vec<u8>,
}

impl Default for Command {
    fn default() -> Self {
        Self {
            action: 't',
            format: 32,
            medium: 'd',
            id: 0,
            placement: 0,
            more: false,
            quiet: 0,
            width: 0,
            height: 0,
            cols: 0,
            rows: 0,
            zlib: false,
            delete: 'a',
            keep_cursor: false,
            payload: Vec::new(),
        }
    }
}

/// Lê o corpo de uma sequência APC; `None` se não for do protocolo gráfico (não começa com `G`).
pub fn parse(body: &[u8]) -> Option<Command> {
    let rest = body.strip_prefix(b"G")?;
    let (keys, data) = match rest.iter().position(|&b| b == b';') {
        Some(i) => (&rest[..i], &rest[i + 1..]),
        None => (rest, &rest[rest.len()..]),
    };
    let mut c = Command::default();
    for kv in keys.split(|&b| b == b',') {
        let Some(eq) = kv.iter().position(|&b| b == b'=') else { continue };
        let (k, v) = (&kv[..eq], std::str::from_utf8(&kv[eq + 1..]).unwrap_or(""));
        let first = v.chars().next().unwrap_or(' ');
        let num = || v.parse::<u32>().unwrap_or(0);
        match k {
            b"a" => c.action = first,
            b"f" => c.format = num(),
            b"t" => c.medium = first,
            b"i" => c.id = num(),
            b"p" => c.placement = num(),
            b"m" => c.more = num() == 1,
            b"q" => c.quiet = num().min(2) as u8,
            b"s" => c.width = num(),
            b"v" => c.height = num(),
            b"c" => c.cols = num(),
            b"r" => c.rows = num(),
            b"o" => c.zlib = first == 'z',
            b"d" => c.delete = first,
            b"C" => c.keep_cursor = num() == 1,
            _ => {}
        }
    }
    let clean: Vec<u8> = data.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
    c.payload = decode_base64(&clean).unwrap_or_default();
    Some(c)
}

// ---------------------------------------------------------------------------------------------
// Imagens

#[derive(Debug)]
pub struct Image {
    /// número único; o aplicativo guarda nele a versão pronta para desenhar
    pub uid: u64,
    pub width: u32,
    pub height: u32,
    /// RGBA, 8 bits por canal
    pub rgba: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// transmissão que não é direta
    Medium,
    Format,
    TooBig,
    Data,
}

impl Error {
    fn code(&self) -> &'static str {
        match self {
            Error::Medium => "EINVAL:so a transmissao direta (t=d) e aceita",
            Error::Format => "EINVAL:formato nao suportado",
            Error::TooBig => "EFBIG:imagem grande demais",
            Error::Data => "EINVAL:dados da imagem invalidos",
        }
    }
}

/// Base64 de uma parte da transmissão, tolerante como o Kitty: aceita o preenchimento `=` ou a falta dele e bits
/// sobrando no último símbolo.
fn decode_base64(text: &[u8]) -> Result<Vec<u8>, Error> {
    let cfg = base64::engine::GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent);
    base64::engine::GeneralPurpose::new(&base64::alphabet::STANDARD, cfg).decode(text).map_err(|_| Error::Data)
}

fn inflate(data: &[u8]) -> Result<Vec<u8>, Error> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data).take(MAX_PAYLOAD as u64 + 1).read_to_end(&mut out).map_err(|_| Error::Data)?;
    if out.len() > MAX_PAYLOAD {
        return Err(Error::TooBig);
    }
    Ok(out)
}

/// Decodifica os dados completos de uma imagem.
pub fn decode(c: &Command, uid: u64) -> Result<Image, Error> {
    if c.medium != 'd' {
        return Err(Error::Medium);
    }
    let data = if c.zlib { inflate(&c.payload)? } else { c.payload.clone() };
    let (width, height, rgba) = match c.format {
        100 => decode_png(&data)?,
        24 | 32 => {
            let (w, h) = (c.width, c.height);
            if w == 0 || h == 0 || (w as u64) * (h as u64) > MAX_PIXELS {
                return Err(if w == 0 || h == 0 { Error::Data } else { Error::TooBig });
            }
            let n = (w * h) as usize;
            let rgba = if c.format == 32 {
                if data.len() != n * 4 {
                    return Err(Error::Data);
                }
                data
            } else {
                if data.len() != n * 3 {
                    return Err(Error::Data);
                }
                data.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect()
            };
            (w, h, rgba)
        }
        _ => return Err(Error::Format),
    };
    Ok(Image { uid, width, height, rgba })
}

fn decode_png(data: &[u8]) -> Result<(u32, u32, Vec<u8>), Error> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(data));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().map_err(|_| Error::Data)?;
    let (w, h) = (reader.info().width, reader.info().height);
    if w == 0 || h == 0 || (w as u64) * (h as u64) > MAX_PIXELS {
        return Err(Error::TooBig);
    }
    let mut buf = vec![0u8; reader.output_buffer_size().ok_or(Error::TooBig)?];
    let info = reader.next_frame(&mut buf).map_err(|_| Error::Data)?;
    let px = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => px.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err(Error::Data),
    };
    Ok((w, h, rgba))
}

// ---------------------------------------------------------------------------------------------
// Estado: imagens guardadas, posições e transmissões em andamento

/// Uma imagem posta na grade. As células que ela ocupa levam um caractere que diz qual posição e qual linha
/// da imagem são ([`filler`]); assim ela acompanha o texto ao rolar, ao quebrar linha e ao encher o histórico.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub seq: u16,
    pub image: u32,
    pub uid: u64,
    pub cols: usize,
    pub rows: usize,
}

/// Quantas posições ficam guardadas ao mesmo tempo (a mais antiga cede o lugar).
pub const SLOTS: usize = 1024;
const FILL_BASE: u32 = 0xF0000;

/// Caractere (área privada) que enche as células da linha `row` da posição `seq`.
pub fn filler(seq: u16, row: usize) -> char {
    let v = ((seq as u32 % SLOTS as u32) << 6) | (row.min(MAX_ROWS) as u32 & 0x3f);
    char::from_u32(FILL_BASE + v).unwrap_or(' ')
}

/// Inverso de [`filler`]: (posição, linha da imagem), ou `None` se `c` não é um desses caracteres.
pub fn decode_filler(c: char) -> Option<(u16, usize)> {
    let v = (c as u32).checked_sub(FILL_BASE)?;
    (v < (SLOTS as u32) << 6).then(|| ((v >> 6) as u16, (v & 0x3f) as usize))
}

/// O que o aplicativo precisa fazer depois de um comando.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// resposta para escrever de volta no terminal
    pub reply: Option<String>,
    /// mostrar a imagem `image` na posição do cursor, ocupando esta área da grade (em células)
    pub show: Option<Show>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Show {
    pub image: u32,
    pub uid: u64,
    pub cols: usize,
    pub rows: usize,
    pub keep_cursor: bool,
}

#[derive(Default)]
pub struct Gfx {
    images: HashMap<u32, Arc<Image>>,
    /// ordem de chegada, para soltar as mais antigas
    order: Vec<u32>,
    bytes: usize,
    pending: Option<Command>,
    next_uid: u64,
    next_auto_id: u32,
    placements: HashMap<u16, Placement>,
    next_seq: u16,
}

fn reply(c: &Command, ok: bool, msg: &str) -> Option<String> {
    if c.id == 0 || (ok && c.quiet >= 1) || c.quiet >= 2 {
        return None;
    }
    let p = if c.placement != 0 { format!(",p={}", c.placement) } else { String::new() };
    Some(format!("\x1b_Gi={}{};{}\x1b\\", c.id, p, if ok { "OK" } else { msg }))
}

/// Tamanho, em células, que uma imagem ocupa. `c`/`r`: o que o programa pediu (0 = automático). Respeita a
/// largura que sobra na linha e a altura máxima, mantendo a proporção quando só um dos dois é pedido.
pub fn fit_cells(img_w: u32, img_h: u32, c: u32, r: u32, cell: (f32, f32), room_cols: usize) -> (usize, usize) {
    let (cw, ch) = (cell.0.max(1.0), cell.1.max(1.0));
    let room = room_cols.max(1);
    let (img_w, img_h) = (img_w.max(1) as f32, img_h.max(1) as f32);
    let (mut cols, mut rows) = match (c, r) {
        (0, 0) => (((img_w / cw).ceil() as usize).max(1), ((img_h / ch).ceil() as usize).max(1)),
        (c, 0) => {
            let cols = c as usize;
            (cols, ((cols as f32 * cw * img_h / img_w / ch).ceil() as usize).max(1))
        }
        (0, r) => {
            let rows = r as usize;
            (((rows as f32 * ch * img_w / img_h / cw).ceil() as usize).max(1), rows)
        }
        (c, r) => (c as usize, r as usize),
    };
    if cols > room {
        rows = ((rows as f32 * room as f32 / cols as f32).ceil() as usize).max(1);
        cols = room;
    }
    if rows > MAX_ROWS {
        cols = ((cols as f32 * MAX_ROWS as f32 / rows as f32).ceil() as usize).max(1);
        rows = MAX_ROWS;
    }
    (cols.max(1), rows.max(1))
}

impl Gfx {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn image(&self, id: u32) -> Option<&Arc<Image>> {
        self.images.get(&id)
    }

    /// uids das imagens que ainda existem (para o aplicativo soltar o que já saiu)
    pub fn live_uids(&self) -> impl Iterator<Item = u64> + '_ {
        self.images.values().map(|i| i.uid)
    }

    fn store(&mut self, id: u32, img: Image) {
        self.remove_image(id);
        self.bytes += img.rgba.len();
        self.images.insert(id, Arc::new(img));
        self.order.push(id);
        while self.bytes > MAX_STORE && self.order.len() > 1 {
            let oldest = self.order[0];
            self.remove_image(oldest);
        }
    }

    fn remove_image(&mut self, id: u32) {
        if let Some(img) = self.images.remove(&id) {
            self.bytes = self.bytes.saturating_sub(img.rgba.len());
            self.order.retain(|&x| x != id);
            self.placements.retain(|_, p| p.uid != img.uid);
        }
    }

    /// Trata o corpo de uma sequência APC. `cell`: tamanho de uma célula em pixels; `room_cols`: colunas livres
    /// da posição do cursor até a borda direita.
    pub fn apc(&mut self, body: &[u8], cell: (f32, f32), room_cols: usize) -> Outcome {
        let Some(mut cmd) = parse(body) else { return Outcome::default() };
        // partes seguintes de uma transmissão: só `m` e os dados importam
        if let Some(mut first) = self.pending.take() {
            if first.payload.len() + cmd.payload.len() > MAX_PAYLOAD {
                return Outcome { reply: reply(&first, false, "EFBIG:imagem grande demais"), show: None };
            }
            first.payload.extend_from_slice(&cmd.payload);
            first.more = cmd.more;
            if first.more {
                self.pending = Some(first);
                return Outcome::default();
            }
            cmd = first;
        } else if cmd.more && matches!(cmd.action, 't' | 'T' | 'q') {
            self.pending = Some(cmd);
            return Outcome::default();
        }

        match cmd.action {
            't' | 'T' | 'q' => {
                self.next_uid += 1;
                let img = match decode(&cmd, self.next_uid) {
                    Ok(i) => i,
                    Err(e) => return Outcome { reply: reply(&cmd, false, e.code()), show: None },
                };
                if cmd.action == 'q' {
                    return Outcome { reply: reply(&cmd, true, ""), show: None };
                }
                let id = if cmd.id != 0 {
                    cmd.id
                } else {
                    self.next_auto_id = self.next_auto_id.wrapping_add(1).max(1);
                    // fora da faixa que os programas escolhem
                    0x8000_0000 | self.next_auto_id
                };
                let (w, h, uid) = (img.width, img.height, img.uid);
                self.store(id, img);
                let show = (cmd.action == 'T').then(|| {
                    let (cols, rows) = fit_cells(w, h, cmd.cols, cmd.rows, cell, room_cols);
                    Show { image: id, uid, cols, rows, keep_cursor: cmd.keep_cursor }
                });
                Outcome { reply: reply(&cmd, true, ""), show }
            }
            'p' => match self.images.get(&cmd.id) {
                Some(img) => {
                    let (cols, rows) = fit_cells(img.width, img.height, cmd.cols, cmd.rows, cell, room_cols);
                    Outcome { reply: reply(&cmd, true, ""), show: Some(Show { image: cmd.id, uid: img.uid, cols, rows, keep_cursor: cmd.keep_cursor }) }
                }
                None => Outcome { reply: reply(&cmd, false, "ENOENT:imagem nao encontrada"), show: None },
            },
            'd' => {
                match cmd.delete {
                    'a' | 'A' => {
                        self.placements.clear();
                        if cmd.delete == 'A' {
                            self.images.clear();
                            self.order.clear();
                            self.bytes = 0;
                        }
                    }
                    'i' | 'I' => {
                        self.placements.retain(|_, p| p.image != cmd.id);
                        if cmd.delete == 'I' {
                            self.remove_image(cmd.id);
                        }
                    }
                    _ => {}
                }
                Outcome::default()
            }
            _ => Outcome::default(),
        }
    }

    /// Uma imagem Sixel (corpo de `ESC P ... ESC \\`): decodifica, guarda e manda mostrar na posição do cursor, como
    /// o Kitty com `a=T`. Imagem inválida ou grande demais é ignorada.
    pub fn sixel(&mut self, body: &[u8], cell: (f32, f32), room_cols: usize) -> Outcome {
        self.next_uid += 1;
        let Ok(img) = sixel::decode(body, self.next_uid) else { return Outcome::default() };
        self.next_auto_id = self.next_auto_id.wrapping_add(1).max(1);
        let id = 0x8000_0000 | self.next_auto_id;
        let (w, h, uid) = (img.width, img.height, img.uid);
        self.store(id, img);
        let (cols, rows) = fit_cells(w, h, 0, 0, cell, room_cols);
        Outcome { reply: None, show: Some(Show { image: id, uid, cols, rows, keep_cursor: false }) }
    }

    /// Guarda uma imagem posta e devolve o número da posição, para [`filler`].
    pub fn place(&mut self, show: &Show) -> u16 {
        let seq = self.next_seq;
        self.next_seq = ((self.next_seq as usize + 1) % SLOTS) as u16;
        self.placements.insert(seq, Placement { seq, image: show.image, uid: show.uid, cols: show.cols, rows: show.rows });
        seq
    }

    pub fn placement(&self, seq: u16) -> Option<&Placement> {
        self.placements.get(&seq)
    }

    pub fn placement_count(&self) -> usize {
        self.placements.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(data: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(data)
    }

    fn apc(keys: &str, data: &[u8]) -> Vec<u8> {
        format!("G{keys};{}", b64(data)).into_bytes()
    }

    fn png_2x1() -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, 2, 1);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&[255, 0, 0, 255, 0, 255, 0, 128]).unwrap();
        drop(w);
        out
    }

    #[test]
    fn splitter_hands_sixel_over_and_other_dcs_back_to_the_terminal() {
        let mut s = Splitter::new();
        let p = s.feed(b"a\x1bP0;1q#0~\x1b\\b\x1bP$q\"q\x1b\\c");
        assert_eq!(p[0], Piece::Text(b"a".to_vec()));
        assert_eq!(p[1], Piece::Sixel(b"0;1q#0~".to_vec()));
        assert_eq!(p[2], Piece::Text(b"b".to_vec()));
        assert_eq!(p[3], Piece::Text(b"\x1bP$q\"q\x1b\\".to_vec()));
        assert_eq!(p[4], Piece::Text(b"c".to_vec()));
        // em qualquer fronteira de pedaço
        let all = b"x\x1bPq#0~\x1b\\y";
        for cut in 1..all.len() {
            let mut s = Splitter::new();
            let mut got = s.feed(&all[..cut]);
            got.extend(s.feed(&all[cut..]));
            let sixels: Vec<_> = got.iter().filter(|p| matches!(p, Piece::Sixel(_))).collect();
            assert_eq!(sixels, [&Piece::Sixel(b"q#0~".to_vec())], "corte em {cut}");
        }
    }

    #[test]
    fn a_sixel_becomes_a_shown_image_like_kitty() {
        let mut g = Gfx::new();
        let out = g.sixel(b"q#1;2;100;0;0#1!16~$-!16~", (8.0, 16.0), 80);
        let show = out.show.expect("mostra");
        assert!(show.cols >= 1 && show.rows >= 1 && !show.keep_cursor);
        assert!(g.image(show.image).is_some());
        assert!(g.sixel(b"lixo", (8.0, 16.0), 80).show.is_none());
    }

    #[test]
    fn splitter_separates_text_and_apc() {
        let mut s = Splitter::new();
        let p = s.feed(b"ola\x1b_Gi=1;QQ==\x1b\\depois");
        assert_eq!(p, vec![Piece::Text(b"ola".to_vec()), Piece::Apc(b"Gi=1;QQ==".to_vec()), Piece::Text(b"depois".to_vec())]);
    }

    #[test]
    fn splitter_handles_any_chunk_boundary() {
        let whole = b"a\x1b[31mb\x1b_Gi=7;QUJD\x1b\\c";
        for cut in 1..whole.len() {
            let mut s = Splitter::new();
            let mut pieces = s.feed(&whole[..cut]);
            pieces.extend(s.feed(&whole[cut..]));
            let text: Vec<u8> = pieces.iter().filter_map(|p| if let Piece::Text(t) = p { Some(t.clone()) } else { None }).flatten().collect();
            let apcs: Vec<_> = pieces.iter().filter_map(|p| if let Piece::Apc(a) = p { Some(a.clone()) } else { None }).collect();
            assert_eq!(text, b"a\x1b[31mbc", "corte em {cut}");
            assert_eq!(apcs, vec![b"Gi=7;QUJD".to_vec()], "corte em {cut}");
        }
    }

    #[test]
    fn plain_escape_sequences_pass_through() {
        let mut s = Splitter::new();
        assert_eq!(s.feed(b"\x1b[2J\x1b]0;t\x07"), vec![Piece::Text(b"\x1b[2J\x1b]0;t\x07".to_vec())]);
    }

    #[test]
    fn huge_apc_is_discarded_not_stored() {
        let mut s = Splitter::new();
        s.feed(b"\x1b_G");
        let chunk = vec![b'A'; 1 << 20];
        let mut last = Vec::new();
        for _ in 0..(MAX_PAYLOAD >> 20) + 2 {
            last = s.feed(&chunk);
        }
        assert!(last.is_empty());
        let end = s.feed(b"\x1b\\ok");
        assert_eq!(end, vec![Piece::Apc(Vec::new()), Piece::Text(b"ok".to_vec())]);
    }

    #[test]
    fn command_keys_are_read() {
        let c = parse(&apc("a=T,f=100,i=5,c=10,r=4,q=2,m=0", b"xyz")).unwrap();
        assert_eq!((c.action, c.format, c.id, c.cols, c.rows, c.quiet), ('T', 100, 5, 10, 4, 2));
        assert_eq!(c.payload, b"xyz");
        assert!(parse(b"Zi=1").is_none());
        assert_eq!(parse(b"Ga=d,d=i,i=3").unwrap().delete, 'i');
    }

    #[test]
    fn png_and_raw_formats_decode() {
        let c = parse(&apc("f=100,a=t", &png_2x1())).unwrap();
        let i = decode(&c, 1).unwrap();
        assert_eq!((i.width, i.height), (2, 1));
        assert_eq!(i.rgba, [255, 0, 0, 255, 0, 255, 0, 128]);
        let raw = parse(&apc("f=24,s=2,v=1", &[1, 2, 3, 4, 5, 6])).unwrap();
        assert_eq!(decode(&raw, 2).unwrap().rgba, [1, 2, 3, 255, 4, 5, 6, 255]);
        let bad = parse(&apc("f=32,s=2,v=2", &[0; 5])).unwrap();
        assert_eq!(decode(&bad, 3).unwrap_err(), Error::Data);
        let huge = parse(&apc("f=32,s=9000,v=9000", &[0; 4])).unwrap();
        assert_eq!(decode(&huge, 4).unwrap_err(), Error::TooBig);
    }

    #[test]
    fn only_direct_transmission_is_accepted() {
        let file = parse(&apc("a=T,f=100,t=f", b"/etc/passwd")).unwrap();
        assert_eq!(decode(&file, 1).unwrap_err(), Error::Medium);
    }

    #[test]
    fn zlib_payload_is_inflated() {
        use std::io::Write;
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&[9, 8, 7, 255]).unwrap();
        let comp = z.finish().unwrap();
        let c = parse(&apc("f=32,s=1,v=1,o=z", &comp)).unwrap();
        assert_eq!(decode(&c, 1).unwrap().rgba, [9, 8, 7, 255]);
    }

    #[test]
    fn chunked_transmission_is_assembled_and_shown() {
        let mut g = Gfx::new();
        let data = png_2x1();
        let (a, b) = data.split_at(data.len() / 2);
        let o1 = g.apc(&apc("a=T,f=100,i=9,m=1", a), (8.0, 16.0), 80);
        assert_eq!(o1, Outcome::default());
        let o2 = g.apc(&apc("m=0", b), (8.0, 16.0), 80);
        let show = o2.show.unwrap();
        assert_eq!((show.image, show.cols, show.rows), (9, 1, 1));
        assert_eq!(o2.reply.as_deref(), Some("\x1b_Gi=9;OK\x1b\\"));
        assert!(g.image(9).is_some());
    }

    #[test]
    fn chunks_padded_one_by_one_like_chafa_are_accepted() {
        // 8x2 RGBA = 64 bytes, enviado em partes de 20 bytes, cada uma com o seu base64 e o seu `=`
        let raw: Vec<u8> = (0..64u8).collect();
        let mut g = Gfx::new();
        let parts: Vec<&[u8]> = raw.chunks(20).collect();
        let first = format!("Ga=T,f=32,s=8,v=2,i=3,m=1");
        assert_eq!(g.apc(first.as_bytes(), (8.0, 16.0), 80), Outcome::default());
        for (n, p) in parts.iter().enumerate() {
            let m = if n + 1 < parts.len() { 1 } else { 0 };
            let body = format!("Gm={m};{}", b64(p));
            let o = g.apc(body.as_bytes(), (8.0, 16.0), 80);
            assert_eq!(o.show.is_some(), m == 0, "parte {n}");
        }
        assert_eq!(g.image(3).unwrap().rgba, raw);
    }

    #[test]
    fn quiet_and_missing_id_suppress_replies() {
        let mut g = Gfx::new();
        let png = png_2x1();
        assert_eq!(g.apc(&apc("a=T,f=100,i=1,q=1", &png), (8.0, 16.0), 80).reply, None);
        assert_eq!(g.apc(&apc("a=T,f=100", &png), (8.0, 16.0), 80).reply, None);
        let err = g.apc(&apc("a=T,f=77,i=2", &png), (8.0, 16.0), 80).reply.unwrap();
        assert!(err.starts_with("\x1b_Gi=2;EINVAL"));
        assert_eq!(g.apc(&apc("a=T,f=77,i=2,q=2", &png), (8.0, 16.0), 80).reply, None);
    }

    #[test]
    fn query_replies_ok_without_storing() {
        let mut g = Gfx::new();
        let o = g.apc(&apc("a=q,i=31,s=1,v=1,t=d,f=24", &[0, 0, 0]), (8.0, 16.0), 80);
        assert_eq!(o.reply.as_deref(), Some("\x1b_Gi=31;OK\x1b\\"));
        assert!(g.image(31).is_none());
    }

    #[test]
    fn size_in_cells_follows_the_cell_size_and_the_room() {
        // 160x64 px em células de 8x16: 20x4
        assert_eq!(fit_cells(160, 64, 0, 0, (8.0, 16.0), 80), (20, 4));
        // largura pedida: a altura segue a proporção
        assert_eq!(fit_cells(160, 64, 10, 0, (8.0, 16.0), 80), (10, 2));
        // não passa da largura que sobra, e a altura encolhe junto
        assert_eq!(fit_cells(1600, 800, 0, 0, (8.0, 16.0), 40), (40, 10));
        // nunca passa do máximo de linhas
        let (c, r) = fit_cells(100, 10_000, 0, 0, (8.0, 16.0), 200);
        assert_eq!(r, MAX_ROWS);
        assert!(c >= 1);
    }

    #[test]
    fn filler_characters_round_trip() {
        for (seq, row) in [(0u16, 0usize), (1, 5), (1023, MAX_ROWS), (512, 33)] {
            assert_eq!(decode_filler(filler(seq, row)), Some((seq, row)));
        }
        assert_eq!(decode_filler('a'), None);
        assert_eq!(decode_filler('\u{E000}'), None);
        assert_eq!(decode_filler('\u{10FFFD}'), None);
    }

    #[test]
    fn delete_and_eviction() {
        let mut g = Gfx::new();
        let png = png_2x1();
        let o = g.apc(&apc("a=T,f=100,i=4", &png), (8.0, 16.0), 80);
        let seq = g.place(&o.show.unwrap());
        assert!(g.placement(seq).is_some());
        g.apc(b"Ga=d,d=i,i=4", (8.0, 16.0), 80);
        assert!(g.placement(seq).is_none());
        assert!(g.image(4).is_some());
        g.apc(b"Ga=d,d=I,i=4", (8.0, 16.0), 80);
        assert!(g.image(4).is_none());
    }

    #[test]
    fn replacing_an_image_drops_its_old_placements() {
        let mut g = Gfx::new();
        let png = png_2x1();
        let o = g.apc(&apc("a=T,f=100,i=1", &png), (8.0, 16.0), 80);
        let seq = g.place(&o.show.unwrap());
        g.apc(&apc("a=t,f=100,i=1", &png), (8.0, 16.0), 80);
        assert!(g.placement(seq).is_none());
        assert_eq!(g.placement_count(), 0);
    }
}
