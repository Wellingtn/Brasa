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
        Self::with_history(std::sync::Arc::new(std::sync::RwLock::new(History::from_shell_files())))
    }

    /// Fontes embutidas usando um histórico compartilhado (o app grava nele cada comando executado).
    pub fn with_history(history: SharedHistory) -> Self {
        let mut e = Self::new();
        e.register(Box::new(HistoryProvider::new(history)));
        e.register(Box::new(SpecProvider::builtin()));
        e.register(Box::new(SshProvider::from_default_config()));
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

/// Um comando já executado: quando (segundos desde 1970; 0 se o histórico não guarda) e em que pasta
/// (só o log do próprio Brasa sabe a pasta).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub cmd: String,
    pub ts: u64,
    pub cwd: Option<String>,
}

/// Um comando do histórico com a nota que recebeu para a pasta atual.
#[derive(Debug, Clone, PartialEq)]
pub struct Ranked {
    pub cmd: String,
    pub score: f32,
    /// quantas vezes foi executado
    pub count: usize,
    /// quando foi a última vez (0 se não se sabe)
    pub last_ts: u64,
}

/// Histórico de comandos, do mais antigo ao mais novo. Ordena por recência, frequência e pasta.
#[derive(Debug, Default, Clone)]
pub struct History {
    entries: Vec<HistoryEntry>,
}

/// Compartilhado entre o motor de sugestões e a busca (Ctrl+R) do app.
pub type SharedHistory = std::sync::Arc<std::sync::RwLock<History>>;

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_entries(entries: Vec<HistoryEntry>) -> Self {
        Self { entries }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Linhas simples (sem hora nem pasta), do mais antigo ao mais novo.
    pub fn from_lines(lines: impl IntoIterator<Item = String>) -> Self {
        Self::from_entries(lines.into_iter().map(|cmd| HistoryEntry { cmd, ts: 0, cwd: None }).collect())
    }

    /// `~/.bash_history` e `~/.zsh_history` (com a hora, quando o zsh a guarda), mais o log do Brasa.
    pub fn load(brasa_log: &Path) -> Self {
        let mut h = Self::from_shell_files();
        h.entries.extend(Self::read_log(brasa_log));
        h
    }

    fn from_shell_files() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let mut entries = Vec::new();
        for f in [".bash_history", ".zsh_history"] {
            if let Ok(bytes) = std::fs::read(home.join(f)) {
                // zsh pode ter bytes não-UTF8 (metafied); lossy basta
                for l in String::from_utf8_lossy(&bytes).lines() {
                    // formato estendido do zsh: ": 1700000000:0;comando"
                    let (ts, cmd) = match l.strip_prefix(": ").and_then(|r| r.split_once(';')) {
                        Some((meta, c)) => (meta.split(':').next().and_then(|t| t.parse().ok()).unwrap_or(0), c),
                        None => (0, l),
                    };
                    let cmd = cmd.trim();
                    if !cmd.is_empty() {
                        entries.push(HistoryEntry { cmd: cmd.to_string(), ts, cwd: None });
                    }
                }
            }
        }
        Self { entries }
    }

    /// Lê o log do Brasa: uma linha por comando, "hora<TAB>pasta<TAB>comando". Linhas inválidas são ignoradas.
    pub fn read_log(path: &Path) -> Vec<HistoryEntry> {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        text.lines()
            .rev()
            .take(20_000)
            .filter_map(|l| {
                let mut it = l.splitn(3, '\t');
                let (ts, cwd, cmd) = (it.next()?.parse().ok()?, it.next()?, it.next()?.trim());
                (!cmd.is_empty()).then(|| HistoryEntry { cmd: cmd.to_string(), ts, cwd: Some(cwd.to_string()).filter(|c| !c.is_empty()) })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    /// Anota um comando executado, na memória e no arquivo do log. Comandos vazios e os que começam
    /// com espaço (convenção de "não guardar") são ignorados.
    pub fn record(&mut self, log: &Path, cmd: &str, cwd: &str, ts: u64) {
        let clean: String = cmd.trim().chars().map(|c| if c == '\t' || c == '\n' || c == '\r' { ' ' } else { c }).collect();
        if clean.is_empty() || cmd.starts_with(' ') {
            return;
        }
        use std::io::Write;
        if let Some(dir) = log.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
            let _ = writeln!(f, "{ts}\t{cwd}\t{clean}");
        }
        self.entries.push(HistoryEntry { cmd: clean, ts, cwd: Some(cwd.to_string()) });
    }

    /// Cada comando uma vez, com nota: recência (a posição entre os comandos distintos), frequência (cresce
    /// devagar, para um comando repetido muitas vezes não esmagar os demais) e um bônus se já foi usado
    /// nesta pasta. Do melhor para o pior.
    pub fn ranked(&self, cwd: &str) -> Vec<Ranked> {
        struct Acc {
            count: usize,
            last_index: usize,
            last_ts: u64,
            here: bool,
        }
        let mut map: HashMap<&str, Acc> = HashMap::new();
        for (i, e) in self.entries.iter().enumerate() {
            let a = map.entry(e.cmd.as_str()).or_insert(Acc { count: 0, last_index: i, last_ts: 0, here: false });
            a.count += 1;
            a.last_index = i;
            a.last_ts = e.ts;
            a.here |= e.cwd.as_deref() == Some(cwd);
        }
        let n = self.entries.len().max(1) as f32;
        let mut out: Vec<Ranked> = map
            .into_iter()
            .map(|(cmd, a)| {
                let recency = a.last_index as f32 / n;
                let frequency = ((a.count as f32 + 1.0).ln() / 21f32.ln()).min(1.0);
                let score = 1.0 + 0.6 * recency + 0.4 * frequency + if a.here { 0.5 } else { 0.0 };
                Ranked { cmd: cmd.to_string(), score, count: a.count, last_ts: a.last_ts }
            })
            .collect();
        out.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.cmd.cmp(&b.cmd)));
        out
    }

    /// Busca para o Ctrl+R: todos os pedaços digitados (separados por espaço) precisam aparecer no comando,
    /// sem diferenciar maiúsculas. Quem começa com o texto vem na frente; depois a nota do histórico.
    /// Sem texto, devolve os melhores.
    pub fn search(&self, query: &str, cwd: &str, limit: usize) -> Vec<Ranked> {
        let q = query.trim().to_lowercase();
        let words: Vec<&str> = q.split_whitespace().collect();
        let mut out: Vec<Ranked> = self
            .ranked(cwd)
            .into_iter()
            .filter(|r| {
                let c = r.cmd.to_lowercase();
                words.iter().all(|w| c.contains(w))
            })
            .map(|mut r| {
                if !q.is_empty() && r.cmd.to_lowercase().starts_with(&q) {
                    r.score += 1.0;
                }
                r
            })
            .collect();
        out.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.cmd.cmp(&b.cmd)));
        out.truncate(limit);
        out
    }
}

pub struct HistoryProvider {
    history: SharedHistory,
}

impl HistoryProvider {
    pub fn new(history: SharedHistory) -> Self {
        Self { history }
    }

    pub fn from_lines(lines: impl IntoIterator<Item = String>) -> Self {
        Self::new(std::sync::Arc::new(std::sync::RwLock::new(History::from_lines(lines))))
    }

    pub fn from_shell_history() -> Self {
        Self::new(std::sync::Arc::new(std::sync::RwLock::new(History::from_shell_files())))
    }
}

impl SuggestionProvider for HistoryProvider {
    fn name(&self) -> &'static str {
        "history"
    }
    fn suggest(&self, ctx: &Context) -> Vec<Suggestion> {
        let Ok(h) = self.history.read() else { return Vec::new() };
        h.ranked(&ctx.cwd.to_string_lossy())
            .into_iter()
            .filter(|r| r.cmd.starts_with(ctx.input))
            .map(|r| Suggestion { text: r.cmd, score: r.score, source: "history", description: None })
            .collect()
    }
}

// ---------- Specs de subcomandos e opções (estilo Fig, versão mínima) ----------

pub struct CommandSpec {
    pub name: &'static str,
    /// (subcomando, descrição)
    pub subcommands: &'static [(&'static str, &'static str)],
    /// (subcomando ou "" para o comando todo, opção, descrição)
    pub flags: &'static [(&'static str, &'static str, &'static str)],
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
                    ("add", "Põe mudanças na área de preparo"),
                    ("commit", "Registra mudanças"),
                    ("checkout", "Troca de branch ou restaura arquivos"),
                    ("switch", "Troca de branch"),
                    ("branch", "Lista, cria ou apaga branches"),
                    ("merge", "Junta outra branch à atual"),
                    ("push", "Envia commits ao remoto"),
                    ("pull", "Busca e integra do remoto"),
                    ("fetch", "Busca do remoto sem integrar"),
                    ("clone", "Copia um repositório"),
                    ("rebase", "Reaplica commits sobre outra base"),
                    ("reset", "Move o HEAD e desfaz preparos"),
                    ("restore", "Restaura arquivos"),
                    ("log", "Mostra o histórico"),
                    ("diff", "Mostra diferenças"),
                    ("show", "Mostra um commit"),
                    ("stash", "Guarda mudanças temporariamente"),
                    ("tag", "Lista ou cria tags"),
                    ("remote", "Gerencia os remotos"),
                    ("cherry-pick", "Aplica um commit de outra branch"),
                    ("init", "Cria um repositório"),
                ],
                flags: &[
                    ("commit", "-m", "Mensagem do commit"),
                    ("commit", "-a", "Inclui todos os arquivos modificados"),
                    ("commit", "--amend", "Altera o último commit"),
                    ("commit", "--no-edit", "Mantém a mensagem"),
                    ("push", "-u", "Define o ramo remoto padrão"),
                    ("push", "--force-with-lease", "Força com segurança"),
                    ("push", "--tags", "Envia as tags"),
                    ("pull", "--rebase", "Reaplica em vez de juntar"),
                    ("log", "--oneline", "Uma linha por commit"),
                    ("log", "--graph", "Desenha o grafo"),
                    ("log", "--stat", "Mostra os arquivos alterados"),
                    ("checkout", "-b", "Cria a branch"),
                    ("switch", "-c", "Cria a branch"),
                    ("status", "--short", "Formato curto"),
                    ("status", "-s", "Formato curto"),
                    ("diff", "--staged", "Só o que está no preparo"),
                    ("diff", "--stat", "Resumo por arquivo"),
                    ("branch", "-d", "Apaga uma branch já integrada"),
                    ("branch", "-D", "Apaga a branch à força"),
                    ("branch", "-a", "Inclui as remotas"),
                    ("reset", "--hard", "Descarta as mudanças"),
                    ("reset", "--soft", "Mantém as mudanças preparadas"),
                    ("clone", "--depth", "Só os últimos commits"),
                    ("", "--help", "Ajuda"),
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
                    ("add", "Adiciona uma dependência"),
                    ("update", "Atualiza o Cargo.lock"),
                    ("doc", "Gera a documentação"),
                    ("clean", "Apaga a pasta target"),
                    ("bench", "Executa os benchmarks"),
                    ("install", "Instala um programa"),
                ],
                flags: &[
                    ("build", "--release", "Compila otimizado"),
                    ("run", "--release", "Executa otimizado"),
                    ("test", "--release", "Testa otimizado"),
                    ("", "-p", "Escolhe o pacote"),
                    ("", "--workspace", "Todos os pacotes do workspace"),
                    ("", "--locked", "Usa o Cargo.lock como está"),
                    ("", "--all-targets", "Todos os alvos"),
                    ("", "--features", "Ativa recursos"),
                    ("", "--offline", "Sem acessar a rede"),
                ],
            },
            CommandSpec {
                name: "docker",
                subcommands: &[
                    ("ps", "Lista contêineres"),
                    ("run", "Executa um contêiner"),
                    ("exec", "Executa um comando num contêiner"),
                    ("build", "Constrói uma imagem"),
                    ("compose", "Orquestra múltiplos contêineres"),
                    ("logs", "Mostra logs"),
                    ("images", "Lista imagens"),
                    ("pull", "Baixa uma imagem"),
                    ("push", "Envia uma imagem"),
                    ("stop", "Para contêineres"),
                    ("start", "Inicia contêineres"),
                    ("restart", "Reinicia contêineres"),
                    ("rm", "Remove contêineres"),
                    ("rmi", "Remove imagens"),
                    ("volume", "Gerencia volumes"),
                    ("network", "Gerencia redes"),
                ],
                flags: &[
                    ("run", "-it", "Interativo com terminal"),
                    ("run", "--rm", "Remove ao sair"),
                    ("run", "-d", "Em segundo plano"),
                    ("run", "-p", "Publica uma porta"),
                    ("run", "-v", "Monta um volume"),
                    ("run", "--name", "Dá um nome"),
                    ("exec", "-it", "Interativo com terminal"),
                    ("ps", "-a", "Inclui os parados"),
                    ("logs", "-f", "Acompanha"),
                    ("logs", "--tail", "Só as últimas linhas"),
                    ("build", "-t", "Dá um nome à imagem"),
                    ("build", "--no-cache", "Sem usar o cache"),
                ],
            },
            CommandSpec {
                name: "apt",
                subcommands: &[
                    ("install", "Instala pacotes"),
                    ("remove", "Remove pacotes"),
                    ("purge", "Remove pacotes e configurações"),
                    ("update", "Atualiza a lista de pacotes"),
                    ("upgrade", "Atualiza os pacotes"),
                    ("full-upgrade", "Atualiza e resolve dependências"),
                    ("autoremove", "Remove o que não é mais usado"),
                    ("search", "Procura pacotes"),
                    ("show", "Mostra um pacote"),
                    ("list", "Lista pacotes"),
                ],
                flags: &[("", "-y", "Responde sim a tudo"), ("install", "--no-install-recommends", "Sem os recomendados"), ("list", "--installed", "Só os instalados"), ("list", "--upgradable", "Só os que têm atualização")],
            },
            CommandSpec {
                name: "systemctl",
                subcommands: &[
                    ("status", "Mostra o estado de um serviço"),
                    ("start", "Inicia"),
                    ("stop", "Para"),
                    ("restart", "Reinicia"),
                    ("reload", "Recarrega a configuração"),
                    ("enable", "Liga ao iniciar o sistema"),
                    ("disable", "Desliga ao iniciar o sistema"),
                    ("is-active", "Diz se está rodando"),
                    ("daemon-reload", "Relê os arquivos de serviço"),
                    ("list-units", "Lista as unidades"),
                    ("mask", "Impede de iniciar"),
                ],
                flags: &[("", "--user", "Serviços do usuário"), ("", "--now", "Liga ou desliga já"), ("", "--no-pager", "Sem paginador"), ("enable", "--now", "Liga e inicia")],
            },
            CommandSpec {
                name: "journalctl",
                subcommands: &[],
                flags: &[("", "-u", "De uma unidade"), ("", "-f", "Acompanha"), ("", "-n", "Só as últimas linhas"), ("", "-b", "Desde o último boot"), ("", "--since", "A partir de uma hora"), ("", "--user", "Do usuário"), ("", "-xe", "Mostra o fim com explicações")],
            },
            CommandSpec {
                name: "ssh",
                subcommands: &[],
                flags: &[("", "-i", "Arquivo de chave"), ("", "-p", "Porta"), ("", "-L", "Encaminha uma porta local"), ("", "-R", "Encaminha uma porta remota"), ("", "-N", "Sem executar comando"), ("", "-v", "Detalhado"), ("", "-A", "Encaminha o agente de chaves"), ("", "-J", "Passa por outra máquina")],
            },
            CommandSpec {
                name: "npm",
                subcommands: &[
                    ("install", "Instala dependências"),
                    ("run", "Executa um script"),
                    ("test", "Executa testes"),
                    ("start", "Inicia o app"),
                    ("init", "Cria um package.json"),
                    ("update", "Atualiza dependências"),
                    ("uninstall", "Remove uma dependência"),
                ],
                flags: &[("install", "--save-dev", "Dependência de desenvolvimento"), ("install", "-g", "Instala globalmente")],
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
        let Some(spec) = self.specs.iter().find(|s| s.name == cmd) else { return Vec::new() };
        let last = ctx.input.rsplit(' ').next().unwrap_or("");
        let before = &ctx.input[..ctx.input.len() - last.len()];
        // o subcomando é a primeira palavra depois do comando, se for um dos conhecidos
        let sub = rest.split(' ').next().filter(|s| spec.subcommands.iter().any(|(n, _)| n == s)).unwrap_or("");
        if last.starts_with('-') {
            return spec
                .flags
                .iter()
                .filter(|(s, f, _)| (s.is_empty() || *s == sub) && f.starts_with(last))
                .map(|(_, f, d)| Suggestion { text: format!("{before}{f}"), score: 0.75, source: "spec", description: Some((*d).to_string()) })
                .collect();
        }
        if rest.contains(' ') {
            return Vec::new(); // depois do subcomando só se completam opções e caminhos
        }
        spec.subcommands
            .iter()
            .filter(|(sub, _)| sub.starts_with(rest))
            .map(|(sub, desc)| Suggestion { text: format!("{cmd} {sub}"), score: 0.8, source: "spec", description: Some((*desc).to_string()) })
            .collect()
    }
}

// ---------- Máquinas do ssh ----------

// ---------- Hosts SSH ----------

/// Um destino do `~/.ssh/config`: o apelido (`Host`) e o que o arquivo diz dele.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshHost {
    pub alias: String,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
}

impl SshHost {
    /// "usuario@maquina:porta", só com o que o arquivo informa; vazio se não diz nada.
    pub fn detail(&self) -> String {
        let mut out = String::new();
        if let Some(u) = &self.user {
            out.push_str(u);
            out.push('@');
        }
        if let Some(h) = &self.hostname {
            out.push_str(h);
        }
        if let Some(p) = self.port {
            out.push_str(&format!(":{p}"));
        }
        out
    }
}

/// Lê os blocos `Host` de um `ssh_config`. Apelidos com `*`, `?` ou `!` (padrões) são ignorados; `HostName`,
/// `User` e `Port` valem para os apelidos do bloco em que aparecem; `Match` encerra o bloco.
pub fn parse_ssh_config(text: &str) -> Vec<SshHost> {
    let mut hosts: Vec<SshHost> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = match line.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((k, v)) => (k.to_ascii_lowercase(), v.trim_start_matches(|c: char| c.is_whitespace() || c == '=').trim().trim_matches('"')),
            None => continue,
        };
        match key.as_str() {
            "host" => {
                current.clear();
                for alias in value.split_whitespace().filter(|a| !a.contains(['*', '?', '!'])) {
                    if let Some(i) = hosts.iter().position(|h| h.alias == alias) {
                        current.push(i);
                    } else {
                        hosts.push(SshHost { alias: alias.to_string(), hostname: None, user: None, port: None });
                        current.push(hosts.len() - 1);
                    }
                }
            }
            "match" => current.clear(),
            "hostname" | "user" | "port" => {
                for &i in &current {
                    let h = &mut hosts[i];
                    match key.as_str() {
                        "hostname" if h.hostname.is_none() => h.hostname = Some(value.to_string()),
                        "user" if h.user.is_none() => h.user = Some(value.to_string()),
                        "port" if h.port.is_none() => h.port = value.parse().ok(),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    hosts
}

/// Destino que o Brasa aceita digitar à mão: `maquina` ou `usuario@maquina`, sem espaço nem aspas, e sem
/// começar com `-` (que o `ssh` leria como opção).
pub fn valid_ssh_target(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && !s.starts_with('-')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@' | ':' | '%' | '[' | ']'))
        && s.matches('@').count() <= 1
}

/// Completa o destino de `ssh` com os `Host` do `~/.ssh/config` e os nomes do `~/.ssh/known_hosts`.
pub struct SshProvider {
    hosts: Vec<String>,
}

impl SshProvider {
    pub fn from_default_config() -> Self {
        let dir = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".ssh");
        let config = std::fs::read_to_string(dir.join("config")).unwrap_or_default();
        let known = std::fs::read_to_string(dir.join("known_hosts")).unwrap_or_default();
        Self::from_texts(&config, &known)
    }

    /// `config`: texto do arquivo de configuração; `known`: texto do known_hosts.
    pub fn from_texts(config: &str, known: &str) -> Self {
        let mut hosts: Vec<String> = Vec::new();
        for l in config.lines() {
            let l = l.trim();
            if l.len() > 5 && l[..5].eq_ignore_ascii_case("host ") {
                hosts.extend(l[5..].split_whitespace().filter(|h| !h.contains(['*', '?', '!'])).map(str::to_string));
            }
        }
        for l in known.lines() {
            let Some(field) = l.split_whitespace().next() else { continue };
            if field.starts_with('|') || field.starts_with('#') || field.starts_with('@') {
                continue; // nomes embaralhados, comentários, marcadores
            }
            for h in field.split(',') {
                let h = h.trim_start_matches('[').split("]:").next().unwrap_or(h);
                if !h.is_empty() && !h.contains(['*', '?']) {
                    hosts.push(h.to_string());
                }
            }
        }
        hosts.sort();
        hosts.dedup();
        Self { hosts }
    }
}

impl SuggestionProvider for SshProvider {
    fn name(&self) -> &'static str {
        "ssh"
    }
    fn suggest(&self, ctx: &Context) -> Vec<Suggestion> {
        let Some(rest) = ctx.input.strip_prefix("ssh ") else { return Vec::new() };
        let mut words: Vec<&str> = rest.split(' ').collect();
        let last = words.pop().unwrap_or("");
        // opções que pedem um valor: a palavra seguinte é o valor, não o destino
        const VALUE_OPTS: [&str; 8] = ["-i", "-p", "-L", "-R", "-l", "-o", "-F", "-J"];
        if last.starts_with('-') || words.last().is_some_and(|w| VALUE_OPTS.contains(w)) {
            return Vec::new();
        }
        // já existe um destino antes (ssh host comando...): nada a completar
        let has_target = words.iter().enumerate().any(|(i, w)| !w.starts_with('-') && !(i > 0 && VALUE_OPTS.contains(&words[i - 1])));
        if has_target {
            return Vec::new();
        }
        let (user, host_part) = match last.split_once('@') {
            Some((u, h)) => (format!("{u}@"), h),
            None => (String::new(), last),
        };
        let before = &ctx.input[..ctx.input.len() - last.len()];
        self.hosts
            .iter()
            .filter(|h| h.starts_with(host_part))
            .map(|h| Suggestion { text: format!("{before}{user}{h}"), score: 0.9, source: "ssh", description: Some("Máquina conhecida".to_string()) })
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
    #[test]
    fn ssh_config_is_parsed_per_block() {
        let cfg = "# meus servidores\nHost web db\n  HostName 10.0.0.5\n  User deploy\n  Port 2222\n\nHost *\n  User ninguem\n\nHost casa\n  HostName=casa.exemplo.com\nMatch host x\n  User escapou\nHost web\n  User outro\n";
        let h = parse_ssh_config(cfg);
        let aliases: Vec<_> = h.iter().map(|x| x.alias.as_str()).collect();
        assert_eq!(aliases, ["web", "db", "casa"]);
        assert_eq!(h[0].detail(), "deploy@10.0.0.5:2222");
        assert_eq!(h[1].detail(), "deploy@10.0.0.5:2222");
        assert_eq!(h[2].detail(), "casa.exemplo.com");
        assert_eq!(h[2].user, None);
        assert_eq!(h[0].user.as_deref(), Some("deploy")); // o primeiro valor vale, como no ssh
    }

    #[test]
    fn typed_ssh_targets_are_checked() {
        assert!(valid_ssh_target("servidor"));
        assert!(valid_ssh_target("ana@10.0.0.1"));
        assert!(valid_ssh_target("host.exemplo.com"));
        assert!(!valid_ssh_target(""));
        assert!(!valid_ssh_target("-oProxyCommand=x"));
        assert!(!valid_ssh_target("a b"));
        assert!(!valid_ssh_target("a;rm -rf"));
        assert!(!valid_ssh_target("a@b@c"));
    }

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

    fn entry(cmd: &str, ts: u64, cwd: Option<&str>) -> HistoryEntry {
        HistoryEntry { cmd: cmd.into(), ts, cwd: cwd.map(String::from) }
    }

    #[test]
    fn frequent_commands_beat_a_single_recent_one() {
        let mut v = vec![];
        for i in 0..12 {
            v.push(entry("git status", i, None));
        }
        v.push(entry("git stash", 100, None));
        let h = History::from_entries(v);
        let r = h.ranked("/x");
        // o mais usado ganha do mais recente, mas a diferença é pequena (a frequência cresce devagar)
        assert_eq!(r[0].cmd, "git status");
        assert_eq!(r[0].count, 12);
        assert_eq!(r[1].cmd, "git stash");
    }

    #[test]
    fn commands_used_in_this_folder_come_first() {
        let h = History::from_entries(vec![
            entry("cargo run", 1, Some("/proj/a")),
            entry("cargo test", 2, Some("/proj/b")),
            entry("cargo build", 3, Some("/proj/b")),
        ]);
        // em /proj/a só "cargo run" foi usado ali, mesmo sendo o mais antigo
        assert_eq!(h.ranked("/proj/a")[0].cmd, "cargo run");
        assert_eq!(h.ranked("/proj/b")[0].cmd, "cargo build");
        let mut e = Engine::new();
        e.register(Box::new(HistoryProvider::new(std::sync::Arc::new(std::sync::RwLock::new(h)))));
        assert_eq!(e.suggest(&ctx("cargo ", Path::new("/proj/a")), 3)[0].text, "cargo run");
    }

    #[test]
    fn history_search_needs_every_word_and_ranks_prefixes_first() {
        let h = History::from_lines(["docker compose up -d", "git commit -m docker", "docker ps", "ls"].map(String::from));
        let r = h.search("docker", "/", 10);
        assert_eq!(r.len(), 3);
        assert!(r[0].cmd.starts_with("docker"), "quem começa com o texto vem na frente");
        let r = h.search("up DOCKER", "/", 10);
        assert_eq!(r.iter().map(|x| x.cmd.as_str()).collect::<Vec<_>>(), ["docker compose up -d"]);
        assert!(h.search("nada disso", "/", 10).is_empty());
        // sem texto: os melhores, limitados
        assert_eq!(h.search("", "/", 2).len(), 2);
    }

    #[test]
    fn brasa_log_is_written_and_read_back() {
        let d = tempfile::tempdir().unwrap();
        let log = d.path().join("config/history.tsv");
        let mut h = History::new();
        h.record(&log, "ls -la\tx", "/home/a", 10);
        h.record(&log, "  ", "/home/a", 11);
        h.record(&log, " segredo", "/home/a", 12); // começa com espaço: não guarda
        h.record(&log, "cargo test", "/home/a", 13);
        assert_eq!(h.len(), 2);
        let back = History::read_log(&log);
        assert_eq!(back, vec![entry("ls -la x", 10, Some("/home/a")), entry("cargo test", 13, Some("/home/a"))]);
        std::fs::write(&log, "lixo\n5\t/p\tok\nabc\t/p\tbad\n").unwrap();
        assert_eq!(History::read_log(&log), vec![entry("ok", 5, Some("/p"))]);
    }

    #[test]
    fn spec_completes_flags_per_subcommand() {
        let mut e = Engine::new();
        e.register(Box::new(SpecProvider::builtin()));
        let r = e.suggest(&ctx("git commit --am", Path::new(".")), 5);
        assert_eq!(r[0].text, "git commit --amend");
        // opção de outro subcomando não aparece
        assert!(e.suggest(&ctx("git push --amend", Path::new(".")), 5).is_empty());
        // opção do comando todo vale em qualquer subcomando
        assert_eq!(e.suggest(&ctx("cargo build --loc", Path::new(".")), 5)[0].text, "cargo build --locked");
        assert_eq!(e.suggest(&ctx("docker run -", Path::new(".")), 20).len(), 6);
        // comando sem subcomandos, só opções
        assert!(e.suggest(&ctx("journalctl -", Path::new(".")), 20).iter().any(|s| s.text == "journalctl -u"));
        // o que já está digitado por inteiro não é sugerido de novo
        assert!(e.suggest(&ctx("journalctl -u", Path::new(".")), 5).is_empty());
        assert!(e.suggest(&ctx("systemctl res", Path::new(".")), 5).iter().any(|s| s.text == "systemctl restart"));
    }

    #[test]
    fn ssh_completes_known_hosts() {
        let config = "Host *\n  User x\nHost prod staging\n  HostName 10.0.0.1\nhost casa\nHost *.interno\n";
        let known = "github.com,140.82.1.1 ssh-ed25519 AAAA\n|1|abc= ssh-rsa BBB\n[meu-nas]:2222 ssh-ed25519 CCC\n";
        let mut e = Engine::new();
        e.register(Box::new(SshProvider::from_texts(config, known)));
        let t = |s: &str| e.suggest(&ctx(s, Path::new(".")), 10).into_iter().map(|x| x.text).collect::<Vec<_>>();
        assert_eq!(t("ssh p"), ["ssh prod"]);
        assert_eq!(t("ssh -p 2222 st"), ["ssh -p 2222 staging"]);
        assert_eq!(t("ssh root@ca"), ["ssh root@casa"]);
        assert_eq!(t("ssh me"), ["ssh meu-nas"]);
        assert!(t("ssh prod l").is_empty(), "depois do destino não completa");
        assert!(t("ssh -p 2").is_empty(), "valor de opção não é destino");
        assert!(t("scp p").is_empty());
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
