//! Autocomplete extensível: cada fonte implementa [`SuggestionProvider`] e o [`Engine`] mescla e ranqueia.
//!
//! Convenção: `Suggestion::text` é a linha de comando COMPLETA já com a sugestão aplicada.
//! O "texto fantasma" (estilo zsh-autosuggestions) é `text` sem o prefixo já digitado.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

pub struct Context<'a> {
    pub input: &'a str,
    pub cwd: &'a Path,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Suggestion {
    pub text: String,
    pub score: f32,
    pub source: &'static str,
    pub description: Option<String>,
}

impl Suggestion {
    /// Parte a ser exibida em cinza após o cursor.
    pub fn ghost<'a>(&'a self, input: &str) -> &'a str {
        self.text.strip_prefix(input).unwrap_or("")
    }
}

/// Ponto de extensão: plugins (Fig specs, LLM, histórico remoto...) implementam isto.
pub trait SuggestionProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn suggest(&self, ctx: &Context) -> Vec<Suggestion>;
}

#[derive(Default)]
pub struct Engine {
    providers: Vec<Box<dyn SuggestionProvider>>,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Engine com as fontes embutidas: histórico do shell, specs de subcomandos e caminhos.
    pub fn with_defaults() -> Self {
        let mut e = Self::new();
        e.register(Box::new(HistoryProvider::from_shell_history()));
        e.register(Box::new(SpecProvider::builtin()));
        e.register(Box::new(PathProvider));
        e
    }

    pub fn register(&mut self, p: Box<dyn SuggestionProvider>) {
        self.providers.push(p);
    }

    pub fn suggest(&self, ctx: &Context, limit: usize) -> Vec<Suggestion> {
        if ctx.input.trim().is_empty() {
            return Vec::new();
        }
        let mut best: HashMap<String, Suggestion> = HashMap::new();
        for s in self.providers.iter().flat_map(|p| p.suggest(ctx)) {
            if s.text == ctx.input || !s.text.starts_with(ctx.input) {
                continue;
            }
            match best.get(&s.text) {
                Some(old) if old.score >= s.score => {}
                _ => {
                    best.insert(s.text.clone(), s);
                }
            }
        }
        let mut out: Vec<_> = best.into_values().collect();
        out.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.text.cmp(&b.text)));
        out.truncate(limit);
        out
    }
}

// ---------- Histórico ----------

pub struct HistoryProvider {
    /// do mais antigo ao mais recente, sem duplicatas
    lines: Vec<String>,
}

impl HistoryProvider {
    pub fn from_lines(lines: impl IntoIterator<Item = String>) -> Self {
        let all: Vec<String> = lines.into_iter().collect();
        // dedup mantendo a ocorrência mais recente
        let mut seen = std::collections::HashSet::new();
        let mut rev: Vec<String> = all.into_iter().rev().filter(|l| seen.insert(l.clone())).collect();
        rev.reverse();
        Self { lines: rev }
    }

    pub fn from_shell_history() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let mut lines = Vec::new();
        for f in [".bash_history", ".zsh_history"] {
            if let Ok(bytes) = std::fs::read(home.join(f)) {
                // zsh pode ter bytes não-UTF8 (metafied); lossy basta
                for l in String::from_utf8_lossy(&bytes).lines() {
                    // formato estendido do zsh: ": 1700000000:0;comando"
                    let cmd = match l.strip_prefix(": ").and_then(|r| r.split_once(';')) {
                        Some((_, c)) => c,
                        None => l,
                    };
                    let cmd = cmd.trim();
                    if !cmd.is_empty() {
                        lines.push(cmd.to_string());
                    }
                }
            }
        }
        Self::from_lines(lines)
    }
}

impl SuggestionProvider for HistoryProvider {
    fn name(&self) -> &'static str {
        "history"
    }
    fn suggest(&self, ctx: &Context) -> Vec<Suggestion> {
        let n = self.lines.len().max(1) as f32;
        self.lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.starts_with(ctx.input))
            .map(|(i, l)| Suggestion {
                text: l.clone(),
                score: 1.0 + i as f32 / n, // mais recente = maior
                source: "history",
                description: None,
            })
            .collect()
    }
}

// ---------- Specs de subcomandos (estilo Fig, versão mínima) ----------

pub struct CommandSpec {
    pub name: &'static str,
    pub subcommands: &'static [(&'static str, &'static str)],
}

pub struct SpecProvider {
    specs: Vec<CommandSpec>,
}

impl SpecProvider {
    pub fn new(specs: Vec<CommandSpec>) -> Self {
        Self { specs }
    }

    pub fn builtin() -> Self {
        Self::new(vec![
            CommandSpec {
                name: "git",
                subcommands: &[
                    ("status", "Mostra o estado da árvore de trabalho"),
                    ("commit", "Registra mudanças"),
                    ("checkout", "Troca de branch ou restaura arquivos"),
                    ("switch", "Troca de branch"),
                    ("push", "Envia commits ao remoto"),
                    ("pull", "Busca e integra do remoto"),
                    ("rebase", "Reaplica commits sobre outra base"),
                    ("log", "Mostra o histórico"),
                    ("diff", "Mostra diferenças"),
                    ("stash", "Guarda mudanças temporariamente"),
                ],
            },
            CommandSpec {
                name: "cargo",
                subcommands: &[
                    ("build", "Compila o pacote"),
                    ("run", "Compila e executa"),
                    ("test", "Executa os testes"),
                    ("check", "Verifica sem gerar binário"),
                    ("clippy", "Lints"),
                    ("fmt", "Formata o código"),
                ],
            },
            CommandSpec {
                name: "docker",
                subcommands: &[
                    ("ps", "Lista contêineres"),
                    ("run", "Executa um contêiner"),
                    ("build", "Constrói uma imagem"),
                    ("compose", "Orquestra múltiplos contêineres"),
                    ("logs", "Mostra logs"),
                ],
            },
            CommandSpec {
                name: "npm",
                subcommands: &[("install", "Instala dependências"), ("run", "Executa um script"), ("test", "Executa testes")],
            },
        ])
    }
}

impl SuggestionProvider for SpecProvider {
    fn name(&self) -> &'static str {
        "spec"
    }
    fn suggest(&self, ctx: &Context) -> Vec<Suggestion> {
        let Some((cmd, rest)) = ctx.input.split_once(' ') else { return Vec::new() };
        if rest.contains(' ') {
            return Vec::new(); // só o primeiro argumento por enquanto
        }
        let Some(spec) = self.specs.iter().find(|s| s.name == cmd) else { return Vec::new() };
        spec.subcommands
            .iter()
            .filter(|(sub, _)| sub.starts_with(rest))
            .map(|(sub, desc)| Suggestion {
                text: format!("{cmd} {sub}"),
                score: 0.8,
                source: "spec",
                description: Some((*desc).to_string()),
            })
            .collect()
    }
}

// ---------- Caminhos ----------

pub struct PathProvider;

impl SuggestionProvider for PathProvider {
    fn name(&self) -> &'static str {
        "path"
    }
    fn suggest(&self, ctx: &Context) -> Vec<Suggestion> {
        // só completa argumentos (depois do primeiro espaço)
        let Some(split) = ctx.input.rfind(' ') else { return Vec::new() };
        let token = &ctx.input[split + 1..];
        let (dir_part, base) = match token.rfind('/') {
            Some(i) => (&token[..=i], &token[i + 1..]),
            None => ("", token),
        };
        let dir: PathBuf = if dir_part.starts_with('/') {
            PathBuf::from(dir_part)
        } else if let Some(rest) = dir_part.strip_prefix("~/") {
            std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(rest)
        } else {
            ctx.cwd.join(dir_part)
        };
        let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
        let prefix = &ctx.input[..ctx.input.len() - base.len()];
        let mut out: Vec<Suggestion> = rd
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if !name.starts_with(base) || (name.starts_with('.') && !base.starts_with('.')) {
                    return None;
                }
                let is_dir = e.path().is_dir();
                Some(Suggestion {
                    text: format!("{prefix}{name}{}", if is_dir { "/" } else { "" }),
                    score: 0.5,
                    source: "path",
                    description: None,
                })
            })
            .collect();
        out.sort_by(|a, b| a.text.cmp(&b.text));
        out.truncate(50);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(input: &'a str, cwd: &'a Path) -> Context<'a> {
        Context { input, cwd }
    }

    #[test]
    fn history_prefers_most_recent_and_dedups() {
        let h = HistoryProvider::from_lines(
            ["git status", "git stash", "git status"].map(String::from),
        );
        let mut e = Engine::new();
        e.register(Box::new(h));
        let r = e.suggest(&ctx("git st", Path::new(".")), 5);
        assert_eq!(r[0].text, "git status"); // reapareceu por último
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].ghost("git st"), "atus");
    }

    #[test]
    fn spec_completes_subcommand_with_description() {
        let mut e = Engine::new();
        e.register(Box::new(SpecProvider::builtin()));
        let r = e.suggest(&ctx("cargo te", Path::new(".")), 5);
        assert_eq!(r[0].text, "cargo test");
        assert!(r[0].description.is_some());
    }

    #[test]
    fn path_completes_dirs_with_slash() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("projetos")).unwrap();
        std::fs::write(d.path().join("pom.xml"), "").unwrap();
        let mut e = Engine::new();
        e.register(Box::new(PathProvider));
        let r = e.suggest(&ctx("cd pro", d.path()), 5);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].text, "cd projetos/");
    }

    #[test]
    fn plugins_are_extensible() {
        struct Fixed;
        impl SuggestionProvider for Fixed {
            fn name(&self) -> &'static str { "fixed" }
            fn suggest(&self, _: &Context) -> Vec<Suggestion> {
                vec![Suggestion { text: "deploy --prod".into(), score: 9.0, source: "fixed", description: None }]
            }
        }
        let mut e = Engine::new();
        e.register(Box::new(Fixed));
        assert_eq!(e.suggest(&ctx("dep", Path::new(".")), 3)[0].text, "deploy --prod");
        assert!(e.suggest(&ctx("", Path::new(".")), 3).is_empty());
    }
}
