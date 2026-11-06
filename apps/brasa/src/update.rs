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
    Available { version: String, url: String },
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
        UpdateState::Available { version: tag, url }
    } else {
        UpdateState::UpToDate
    }
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
        assert_eq!(classify("1.1.0", json), UpdateState::Available { version: "v1.2.0".into(), url: "https://github.com/Wellingtn/brasa/releases/tag/v1.2.0".into() });
        assert_eq!(classify("1.2.0", json), UpdateState::UpToDate);
        assert_eq!(classify("1.1.0", "{}"), UpdateState::Failed);
        assert_eq!(classify("1.1.0", r#"{"message":"Not Found"}"#), UpdateState::Failed);
    }
}
