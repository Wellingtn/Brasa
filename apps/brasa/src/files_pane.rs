//! Gerenciador de arquivos em painel, como o "preview" de pasta do Wave: tabela com nome,
//! permissões, data, tamanho e tipo; navegação com histórico; visualizador de imagens com zoom.
//! Aqui fica só o modelo (listar, formatar, ordenar); o desenho está em `pane_view.rs`.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Col {
    Name,
    Modified,
    Size,
    Kind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub perm: String,
    /// segundos desde 1970 (UTC)
    pub mtime: i64,
    pub size: u64,
    pub kind: String,
}

/// Estado do painel: pasta atual, histórico para o "voltar", ordenação, seleção e o arquivo aberto.
pub struct FilesPane {
    pub dir: PathBuf,
    pub back: Vec<PathBuf>,
    pub show_hidden: bool,
    pub sort: Col,
    pub asc: bool,
    pub rows: Vec<Row>,
    /// índice selecionado em `rows` (o ".." não está em `rows`; `None` = nada)
    pub selected: Option<usize>,
    pub scroll: usize,
    pub wheel: f32,
    /// imagem aberta no visualizador (no lugar da tabela)
    pub viewing: Option<PathBuf>,
    pub zoom: f32,
    pub error: Option<String>,
}

impl FilesPane {
    pub fn new(dir: PathBuf) -> Self {
        let mut p = FilesPane { dir, back: Vec::new(), show_hidden: false, sort: Col::Name, asc: true, rows: Vec::new(), selected: None, scroll: 0, wheel: 0.0, viewing: None, zoom: 1.0, error: None };
        p.reload();
        p
    }

    pub fn reload(&mut self) {
        match list(&self.dir, self.show_hidden) {
            Ok(mut rows) => {
                sort_rows(&mut rows, self.sort, self.asc);
                self.rows = rows;
                self.error = None;
            }
            Err(e) => {
                self.rows.clear();
                self.error = Some(format!("Não deu para abrir a pasta: {e}"));
            }
        }
        self.selected = self.selected.filter(|i| *i < self.rows.len());
    }

    /// Entra numa pasta, guardando a atual para o "voltar".
    pub fn open_dir(&mut self, dir: PathBuf) {
        if dir != self.dir {
            self.back.push(std::mem::replace(&mut self.dir, dir));
        }
        self.viewing = None;
        self.selected = None;
        self.scroll = 0;
        self.reload();
    }

    /// Voltar: fecha a imagem aberta ou volta para a pasta anterior (ou sobe um nível).
    pub fn go_back(&mut self) {
        if self.viewing.take().is_some() {
            return;
        }
        let prev = self.back.pop().or_else(|| self.dir.parent().map(Path::to_path_buf));
        if let Some(prev) = prev {
            self.dir = prev;
            self.selected = None;
            self.scroll = 0;
            self.reload();
        }
    }

    pub fn set_sort(&mut self, col: Col) {
        if self.sort == col {
            self.asc = !self.asc;
        } else {
            self.sort = col;
            self.asc = true;
        }
        sort_rows(&mut self.rows, self.sort, self.asc);
    }
}

pub fn is_image(path: &Path) -> bool {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico")
}

/// Lê a pasta. Ocultos (começam com ".") só com `hidden`.
pub fn list(dir: &Path, hidden: bool) -> std::io::Result<Vec<Row>> {
    let mut rows = Vec::new();
    for e in std::fs::read_dir(dir)?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !hidden && name.starts_with('.') {
            continue;
        }
        let path = e.path();
        // metadados do destino para links (pasta ligada aparece como pasta); do próprio link se quebrado
        let meta = std::fs::metadata(&path).or_else(|_| std::fs::symlink_metadata(&path));
        let Ok(meta) = meta else { continue };
        let is_dir = meta.is_dir();
        let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
        rows.push(Row {
            kind: kind_of(&name, is_dir),
            perm: perm_string(meta.permissions().mode(), is_dir),
            size: if is_dir { 0 } else { meta.len() },
            name,
            path,
            is_dir,
            mtime,
        });
    }
    Ok(rows)
}

/// Pastas primeiro; dentro de cada grupo, pela coluna escolhida.
pub fn sort_rows(rows: &mut [Row], col: Col, asc: bool) {
    rows.sort_by(|a, b| {
        let ord = match col {
            Col::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            Col::Modified => a.mtime.cmp(&b.mtime),
            Col::Size => a.size.cmp(&b.size),
            Col::Kind => a.kind.cmp(&b.kind),
        };
        b.is_dir.cmp(&a.is_dir).then(if asc { ord } else { ord.reverse() })
    });
}

/// "drwxr-xr-x", "-rw-r--r--".
pub fn perm_string(mode: u32, is_dir: bool) -> String {
    let mut s = String::with_capacity(10);
    s.push(if is_dir { 'd' } else { '-' });
    for shift in [6, 3, 0] {
        let b = (mode >> shift) & 7;
        s.push(if b & 4 != 0 { 'r' } else { '-' });
        s.push(if b & 2 != 0 { 'w' } else { '-' });
        s.push(if b & 1 != 0 { 'x' } else { '-' });
    }
    s
}

/// Tamanho curto como no Wave: "925b", "13.8k", "4.78m", "1.2g".
pub fn size_string(size: u64) -> String {
    let units = [("g", 1u64 << 30), ("m", 1 << 20), ("k", 1 << 10)];
    for (u, n) in units {
        if size >= n {
            let v = size as f64 / n as f64;
            return if v >= 100.0 { format!("{v:.0}{u}") } else if v >= 10.0 { format!("{v:.1}{u}") } else { format!("{v:.2}{u}") };
        }
    }
    format!("{size}b")
}

/// Tipo legível a partir da extensão.
pub fn kind_of(name: &str, is_dir: bool) -> String {
    if is_dir {
        return "pasta".into();
    }
    let ext = name.rsplit_once('.').map(|(b, e)| if b.is_empty() { "" } else { e }).unwrap_or("").to_lowercase();
    let k = match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico" => return format!("imagem {}", ext.to_uppercase()),
        "md" | "markdown" => "Markdown",
        "txt" | "log" => "texto",
        "rs" => "Rust",
        "py" => "Python",
        "js" | "mjs" | "cjs" => "JavaScript",
        "ts" | "tsx" => "TypeScript",
        "json" => "JSON",
        "toml" => "TOML",
        "yml" | "yaml" => "YAML",
        "html" | "htm" => "HTML",
        "css" => "CSS",
        "sh" | "bash" | "zsh" => "script de shell",
        "zip" | "gz" | "tgz" | "xz" | "bz2" | "7z" | "rar" | "tar" => "arquivo compactado",
        "pdf" => "PDF",
        "mp3" | "ogg" | "wav" | "flac" => "áudio",
        "mp4" | "mkv" | "webm" | "mov" => "vídeo",
        "deb" | "rpm" | "appimage" => "pacote",
        "" => "arquivo",
        _ => return format!("arquivo .{ext}"),
    };
    k.to_string()
}

const MONTHS: [&str; 12] = ["jan", "fev", "mar", "abr", "mai", "jun", "jul", "ago", "set", "out", "nov", "dez"];

/// (ano, mês 1–12, dia) a partir de dias desde 1970 (algoritmo "civil_from_days" de H. Hinnant).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// "4 nov 21:48" neste ano; "13 mar 2025" em anos anteriores. `offset` = fuso em segundos.
pub fn date_string(mtime: i64, now: i64, offset: i64) -> String {
    let local = mtime + offset;
    let (y, m, d) = civil(local.div_euclid(86_400));
    let (ny, _, _) = civil((now + offset).div_euclid(86_400));
    let secs = local.rem_euclid(86_400);
    if y == ny {
        format!("{d} {} {:02}:{:02}", MONTHS[m as usize - 1], secs / 3600, secs % 3600 / 60)
    } else {
        format!("{d} {} {y}", MONTHS[m as usize - 1])
    }
}

/// Fuso local em segundos (de `date +%z`, ex.: "-0300"); 0 se não der para saber.
pub fn local_offset() -> i64 {
    let out = std::process::Command::new("date").arg("+%z").output().ok();
    let s = out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    parse_offset(&s)
}

fn parse_offset(s: &str) -> i64 {
    let (sign, rest) = match s.chars().next() {
        Some('-') => (-1, &s[1..]),
        Some('+') => (1, &s[1..]),
        _ => return 0,
    };
    if rest.len() != 4 {
        return 0;
    }
    let h: i64 = rest[..2].parse().unwrap_or(0);
    let m: i64 = rest[2..].parse().unwrap_or(0);
    sign * (h * 3600 + m * 60)
}

/// "~ (/home/ana)" ou o caminho inteiro fora da pasta pessoal, como no cabeçalho do Wave.
pub fn path_label(dir: &Path, home: &Path) -> String {
    match dir.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => format!("~ ({})", dir.display()),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => dir.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_wave() {
        assert_eq!(perm_string(0o755, true), "drwxr-xr-x");
        assert_eq!(perm_string(0o600, false), "-rw-------");
        assert_eq!(size_string(925), "925b");
        assert_eq!(size_string(14_131), "13.8k");
        assert_eq!(size_string(5_012_000), "4.78m");
        assert_eq!(kind_of("foto.PNG", false), "imagem PNG");
        assert_eq!(kind_of(".bashrc", false), "arquivo");
        assert_eq!(kind_of("x.zip", false), "arquivo compactado");
        assert_eq!(kind_of("docs", true), "pasta");
    }

    #[test]
    fn dates_in_local_time() {
        // 2026-11-05 00:48 UTC = 4 nov 21:48 em -03:00
        let t = 1_793_839_680;
        assert_eq!(date_string(t, t, -3 * 3600), "4 nov 21:48");
        // ano anterior mostra o ano
        assert_eq!(date_string(1_741_867_320, t, -3 * 3600), "13 mar 2025");
        assert_eq!(parse_offset("-0300"), -10_800);
        assert_eq!(parse_offset("+0530"), 19_800);
        assert_eq!(parse_offset("x"), 0);
        assert_eq!(civil(0), (1970, 1, 1));
    }

    #[test]
    fn lists_sorts_and_navigates() {
        let root = std::env::temp_dir().join(format!("oxiterm-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("b-pasta")).unwrap();
        std::fs::create_dir_all(root.join(".oculta")).unwrap();
        std::fs::write(root.join("a.txt"), "12345").unwrap();
        std::fs::write(root.join("c.png"), "1").unwrap();
        let mut p = FilesPane::new(root.clone());
        let names: Vec<_> = p.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["b-pasta", "a.txt", "c.png"]);
        p.show_hidden = true;
        p.reload();
        assert_eq!(p.rows[0].name, ".oculta");
        p.set_sort(Col::Size);
        p.set_sort(Col::Size); // segundo clique: maior primeiro
        assert_eq!(p.rows.iter().filter(|r| !r.is_dir).map(|r| r.name.as_str()).collect::<Vec<_>>(), ["a.txt", "c.png"]);
        p.open_dir(root.join("b-pasta"));
        assert!(p.rows.is_empty());
        p.go_back();
        assert_eq!(p.dir, root);
        assert_eq!(path_label(&root.join("x"), &root), "~/x");
        let _ = std::fs::remove_dir_all(&root);
    }
}
