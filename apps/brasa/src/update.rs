//! Aviso de nova versão: pergunta ao GitHub qual é a release mais recente e compara com a versão em uso.
//! Só avisa (não baixa nem instala nada). Usa o `curl` do sistema; sem ele, ou sem internet, o resultado é
//! "não foi possível verificar". Pode ser desligado nas configurações.

/// Repositório consultado.
const API: &str = "https://api.github.com/repos/Wellingtn/brasa/releases/latest";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateState {
    /// ainda não verificou (ou a verificação está desligada)
    Unknown,
    Checking,
    UpToDate,
    Available { version: String, url: String, assets: Vec<String> },
    /// baixando, conferindo ou instalando a versão nova (o texto do passo atual fica em `update_progress`)
    Installing,
    /// a atualização automática falhou; o texto explica o motivo
    InstallFailed(String),
    /// o GitHub respondeu 404: ainda não há release pública (ou o repositório é privado)
    NoPublicRelease,
    /// sem internet, sem `curl` ou resposta inesperada
    Failed,
}

/// Consulta o GitHub (bloqueia até uns 8 s; rode fora da thread da interface).
pub fn check(current: &str) -> UpdateState {
    let out = std::process::Command::new("curl")
        .args(["-sSL", "--max-time", "8", "-w", "\n%{http_code}", "-H", "Accept: application/vnd.github+json", "-H", "User-Agent: brasa", API])
        .output();
    let Ok(out) = out else { return UpdateState::Failed };
    let text = String::from_utf8_lossy(&out.stdout);
    // a última linha é o código HTTP que o curl acrescentou
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", ""));
    match code.trim() {
        "200" => classify(current, body),
        "404" => UpdateState::NoPublicRelease,
        _ => UpdateState::Failed,
    }
}

fn classify(current: &str, json: &str) -> UpdateState {
    let (Some(tag), Some(url)) = (json_str(json, "tag_name"), json_str(json, "html_url")) else {
        return UpdateState::Failed;
    };
    if newer(current, &tag) {
        UpdateState::Available { version: tag, url, assets: json_all(json, "browser_download_url") }
    } else {
        UpdateState::UpToDate
    }
}

/// Todos os valores de texto de `"chave": "valor"` no JSON, na ordem em que aparecem.
fn json_all(json: &str, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = json;
    while let Some(v) = json_str(rest, key) {
        let at = rest.find(&format!("\"{key}\"")).unwrap() + key.len() + 2;
        out.push(v);
        rest = &rest[at..];
    }
    out
}

/// Primeiro valor de texto de `"chave": "valor"` no JSON (basta para a resposta do GitHub).
fn json_str(json: &str, key: &str) -> Option<String> {
    let at = json.find(&format!("\"{key}\""))?;
    let rest = json[at + key.len() + 2..].trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn parts(v: &str) -> Vec<u64> {
    v.trim().trim_start_matches(['v', 'V']).split('.').map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0)).collect()
}

/// `latest` é mais nova que `current` (compara 1.10.0 > 1.9.0 pelos números, não pelo texto).
pub fn newer(current: &str, latest: &str) -> bool {
    let (a, b) = (parts(current), parts(latest));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return y > x;
        }
    }
    false
}

// ---------------------------------------------------------------------------------------------
// Atualização automática: baixa o instalador da release, confere a soma SHA-256 publicada junto e
// instala. AppImage: troca o próprio arquivo. .deb: instala com `pkexec apt-get` (o sistema pede a
// senha). Quem compilou do código-fonte só recebe o aviso.

use std::path::{Path, PathBuf};

/// Só se baixa daqui: as releases deste repositório.
const DOWNLOAD_PREFIX: &str = "https://github.com/Wellingtn/brasa/releases/download/";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallKind {
    /// rodando de um AppImage (o caminho dele)
    AppImage(PathBuf),
    /// instalado em /usr (pelo .deb)
    Deb,
    /// compilado do código-fonte ou em outro lugar: não se atualiza sozinho
    Source,
}

pub fn install_kind() -> InstallKind {
    kind_for(std::env::var("APPIMAGE").ok().as_deref(), std::env::current_exe().ok().as_deref())
}

fn kind_for(appimage_env: Option<&str>, exe: Option<&Path>) -> InstallKind {
    if let Some(p) = appimage_env.filter(|p| !p.is_empty() && Path::new(p).is_file()) {
        return InstallKind::AppImage(PathBuf::from(p));
    }
    match exe {
        Some(e) if e.starts_with("/usr/") => InstallKind::Deb,
        _ => InstallKind::Source,
    }
}

/// Endereço do arquivo de `assets` cujo nome termina com `suffix`; só aceita endereços do repositório.
fn asset(assets: &[String], suffix: &str) -> Option<String> {
    assets.iter().find(|u| u.starts_with(DOWNLOAD_PREFIX) && u.ends_with(suffix)).cloned()
}

fn download(url: &str, dest: &Path) -> Result<(), String> {
    let status = std::process::Command::new("curl").args(["-fsSL", "--max-time", "600", "-o"]).arg(dest).arg(url).status().map_err(|_| "o curl não está instalado".to_string())?;
    if status.success() { Ok(()) } else { Err("não foi possível baixar o arquivo".into()) }
}

/// Soma SHA-256 de um arquivo (usa o `sha256sum` do sistema).
fn sha256(path: &Path) -> Result<String, String> {
    let out = std::process::Command::new("sha256sum").arg(path).output().map_err(|_| "o sha256sum não está instalado".to_string())?;
    String::from_utf8_lossy(&out.stdout).split_whitespace().next().map(str::to_lowercase).ok_or_else(|| "não foi possível calcular a soma do arquivo".into())
}

/// Confere o arquivo contra o `SHA256SUMS` da release (linhas "soma  nome" ou "soma *nome").
fn verify(sums: &str, name: &str, path: &Path) -> Result<(), String> {
    let want = sums
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let (hash, file) = (it.next()?, it.next()?.trim_start_matches('*'));
            (file == name).then(|| hash.to_lowercase())
        })
        .next()
        .ok_or_else(|| format!("a lista de somas da release não tem {name}"))?;
    if sha256(path)? == want { Ok(()) } else { Err("a soma do arquivo baixado não bate com a da release; nada foi instalado".into()) }
}

fn file_name(url: &str) -> &str {
    url.rsplit('/').next().unwrap_or(url)
}

/// Baixa, confere e instala a versão nova. Devolve o programa a abrir de novo. `progress` recebe o passo atual.
pub fn install(kind: &InstallKind, assets: &[String], progress: &dyn Fn(&str)) -> Result<PathBuf, String> {
    if *kind == InstallKind::Source {
        return Err("esta cópia foi compilada do código-fonte; atualize com git pull e cargo build".into());
    }
    let suffix = if matches!(kind, InstallKind::Deb) { ".deb" } else { ".AppImage" };
    let url = asset(assets, suffix).ok_or("a release não tem o instalador para esta instalação")?;
    let sums_url = asset(assets, "SHA256SUMS").ok_or("a release não publicou as somas de verificação (SHA256SUMS); por segurança não instalo")?;
    let dir = crate::config_dir().join("update");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let sums_path = dir.join("SHA256SUMS");
    progress("Baixando…");
    download(&sums_url, &sums_path)?;
    let sums = std::fs::read_to_string(&sums_path).map_err(|e| e.to_string())?;
    match kind {
        InstallKind::AppImage(target) => {
            // baixa ao lado do arquivo atual para poder trocá-lo de uma vez (rename no mesmo disco)
            let new = target.with_extension("AppImage.novo");
            download(&url, &new)?;
            progress("Conferindo…");
            if let Err(e) = verify(&sums, file_name(&url), &new) {
                let _ = std::fs::remove_file(&new);
                return Err(e);
            }
            progress("Instalando…");
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
            std::fs::rename(&new, target).map_err(|e| format!("não consegui trocar o AppImage ({e})"))?;
            Ok(target.clone())
        }
        InstallKind::Deb => {
            let deb = dir.join(file_name(&url));
            download(&url, &deb)?;
            progress("Conferindo…");
            verify(&sums, file_name(&url), &deb)?;
            progress("Instalando… (o sistema vai pedir a sua senha)");
            let status = std::process::Command::new("pkexec").args(["apt-get", "install", "-y"]).arg(&deb).status().map_err(|_| "o pkexec não está instalado".to_string())?;
            let _ = std::fs::remove_file(&deb);
            if status.success() { Ok(PathBuf::from("/usr/bin/brasa")) } else { Err("a instalação foi cancelada ou falhou".into()) }
        }
        InstallKind::Source => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("1.1.0", "v1.2.0"));
        assert!(newer("1.9.0", "1.10.0"));
        assert!(newer("1.1.0", "1.1.1"));
        assert!(newer("1.1", "1.1.1"));
        assert!(!newer("1.1.0", "v1.1.0"));
        assert!(!newer("1.2.0", "1.1.9"));
        assert!(!newer("2.0.0", "1.99.99"));
    }

    #[test]
    fn github_answer_is_classified() {
        let json = r#"{"url":"x","html_url": "https://github.com/Wellingtn/brasa/releases/tag/v1.2.0","tag_name":"v1.2.0","name":"v1.2.0"}"#;
        assert_eq!(classify("1.1.0", json), UpdateState::Available { version: "v1.2.0".into(), url: "https://github.com/Wellingtn/brasa/releases/tag/v1.2.0".into(), assets: Vec::new() });
        assert_eq!(classify("1.2.0", json), UpdateState::UpToDate);
        assert_eq!(classify("1.1.0", "{}"), UpdateState::Failed);
        assert_eq!(classify("1.1.0", r#"{"message":"Not Found"}"#), UpdateState::Failed);
    }

    #[test]
    fn asset_links_are_collected_and_filtered() {
        let json = r#"{"assets":[{"name":"a","browser_download_url":"https://github.com/Wellingtn/brasa/releases/download/v1.2.0/brasa_1.2.0-1_amd64.deb"},{"browser_download_url": "https://github.com/Wellingtn/brasa/releases/download/v1.2.0/SHA256SUMS"},{"browser_download_url":"https://evil.example/Brasa.AppImage"}]}"#;
        let all = json_all(json, "browser_download_url");
        assert_eq!(all.len(), 3);
        assert!(asset(&all, ".deb").unwrap().ends_with("brasa_1.2.0-1_amd64.deb"));
        assert!(asset(&all, "SHA256SUMS").is_some());
        // endereço de outro lugar nunca é usado
        assert_eq!(asset(&all, ".AppImage"), None);
        assert_eq!(file_name("https://x/y/SHA256SUMS"), "SHA256SUMS");
    }

    #[test]
    fn install_kind_is_detected() {
        assert_eq!(kind_for(None, Some(Path::new("/usr/bin/brasa"))), InstallKind::Deb);
        assert_eq!(kind_for(None, Some(Path::new("/home/x/Projetos/brasa/target/release/brasa"))), InstallKind::Source);
        assert_eq!(kind_for(Some("/nao/existe.AppImage"), Some(Path::new("/tmp/.mount_x/usr/bin/brasa"))), InstallKind::Source);
        let me = std::env::current_exe().unwrap();
        assert_eq!(kind_for(me.to_str(), None), InstallKind::AppImage(me));
    }

    #[test]
    fn downloaded_file_is_checked_against_the_sums() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("brasa.deb");
        std::fs::write(&file, "abc").unwrap();
        // SHA-256 de "abc"
        let good = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(verify(&format!("{good}  brasa.deb\n"), "brasa.deb", &file).is_ok());
        assert!(verify(&format!("{}  *brasa.deb\n", good.to_uppercase()), "brasa.deb", &file).is_ok());
        assert!(verify(&format!("{}  brasa.deb\n", "0".repeat(64)), "brasa.deb", &file).unwrap_err().contains("não bate"));
        assert!(verify(&format!("{good}  outro.deb\n"), "brasa.deb", &file).unwrap_err().contains("não tem"));
        // download por file:// para o teste não depender de internet
        let copy = dir.path().join("copia");
        download(&format!("file://{}", file.display()), &copy).unwrap();
        assert_eq!(std::fs::read_to_string(copy).unwrap(), "abc");
        assert!(download("file:///nao/existe/mesmo", &dir.path().join("x")).is_err());
    }

    #[test]
    fn source_builds_never_self_update() {
        assert!(install(&InstallKind::Source, &[], &|_| {}).unwrap_err().contains("código-fonte"));
        // sem soma publicada, não instala
        let only_deb = vec![format!("{DOWNLOAD_PREFIX}v1.2.0/brasa.deb")];
        assert!(install(&InstallKind::Deb, &only_deb, &|_| {}).unwrap_err().contains("SHA256SUMS"));
    }
}
