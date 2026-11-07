//! Camada de PTY: abre um shell em um pseudo-terminal e transmite a saída via callbacks.
//! Não conhece GUI; o front-end só precisa implementar [`PtyEvents`].

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

pub type SessionId = u64;

#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("sessão {0} não existe")]
    NoSession(SessionId),
    #[error("pty: {0}")]
    Pty(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

fn pty_err(e: impl std::fmt::Display) -> PtyError {
    PtyError::Pty(e.to_string())
}

/// Receptor de eventos; chamado a partir da thread leitora de cada sessão.
pub trait PtyEvents: Send + Sync + 'static {
    fn on_output(&self, id: SessionId, bytes: &[u8]);
    fn on_exit(&self, id: SessionId);
}

/// Prompt mínimo que substitui o `user@host:~$` do shell (a pasta e o branch ficam nos chips da UI).
#[derive(Debug, Clone)]
pub struct PromptConfig {
    /// texto do prompt; vazio = sem prompt algum
    pub text: String,
    pub color: Option<(u8, u8, u8)>,
}

#[derive(Debug, Clone)]
pub struct SpawnConfig {
    pub shell: Option<String>,
    pub cwd: Option<PathBuf>,
    pub cols: u16,
    pub rows: u16,
    /// `None` mantém o prompt original do shell. Só tem efeito em bash, zsh e fish.
    pub prompt: Option<PromptConfig>,
}

impl Default for SpawnConfig {
    fn default() -> Self {
        Self { shell: None, cwd: None, cols: 80, rows: 24, prompt: None }
    }
}

// ---------- prompt mínimo ----------
//
// O usuário continua com seu .bashrc/.zshrc: o arquivo de integração carrega o original primeiro
// e só depois troca o prompt (e o reaplica a cada comando, caso venv/starship o alterem).

/// Aspas simples para sh/bash.
fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Aspas $'...' para zsh.
fn dollar_q(s: &str) -> String {
    format!("$'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

// Marcadores (caracteres de uso privado) que o prompt imprime na primeira linha:
//   <PROMPT_START>~/pasta<PROMPT_BRANCH>main<PROMPT_VENV>venv
// A UI reconhece essa linha e a desenha como chips no lugar do texto do prompt.
pub const PROMPT_START: char = '\u{e000}';
pub const PROMPT_BRANCH: char = '\u{e001}';
pub const PROMPT_VENV: char = '\u{e003}';
/// código de saída do comando anterior (número) e hora local "HH:MM" em que o prompt foi desenhado
pub const PROMPT_STATUS: char = '\u{e004}';
pub const PROMPT_TIME: char = '\u{e005}';

/// O que o prompt informou: pasta, branch git e ambiente virtual.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PromptInfo {
    pub path: String,
    pub branch: Option<String>,
    pub venv: Option<String>,
    /// código de saída do comando que terminou antes deste prompt
    pub status: Option<i32>,
    /// hora local "HH:MM" em que este prompt apareceu
    pub time: Option<String>,
}

/// Interpreta o texto de uma linha da tela; `None` se não for a linha de prompt do Brasa.
pub fn parse_prompt_row(row: &str) -> Option<PromptInfo> {
    let rest = row.strip_prefix(PROMPT_START)?;
    let mut field = 0; // 0 pasta, 1 branch, 2 venv, 3 status, 4 hora
    let mut bufs: [String; 5] = Default::default();
    for c in rest.chars() {
        match c {
            PROMPT_BRANCH => field = 1,
            PROMPT_VENV => field = 2,
            PROMPT_STATUS => field = 3,
            PROMPT_TIME => field = 4,
            c => bufs[field].push(c),
        }
    }
    let [path, branch, venv, status, time] = bufs.map(|b| b.trim().to_string());
    let some = |v: String| Some(v).filter(|v| !v.is_empty());
    Some(PromptInfo { path, branch: some(branch), venv: some(venv), status: status.parse().ok(), time: some(time) })
}

/// Procura o branch sem chamar o git: sobe até achar `.git/HEAD` (bash e zsh).
const SH_BRANCH_FN: &str = r#"__brasa_branch() {
  local d=$PWD h
  while [ -n "$d" ]; do
    if [ -f "$d/.git/HEAD" ]; then
      h=$(<"$d/.git/HEAD")
      case $h in "ref: refs/heads/"*) printf '%s' "${h#ref: refs/heads/}";; *) printf '%.7s' "$h";; esac
      return
    fi
    d=${d%/*}
  done
}
"#;

fn bash_line2(p: &PromptConfig) -> String {
    if p.text.is_empty() {
        return String::new();
    }
    match p.color {
        Some((r, g, b)) => format!("\\[\\e[38;2;{r};{g};{b}m\\]{}\\[\\e[0m\\] ", p.text),
        None => format!("{} ", p.text),
    }
}

pub fn bash_rc(p: &PromptConfig) -> String {
    format!(
        "[ -f /etc/bash.bashrc ] && . /etc/bash.bashrc\n\
         [ -f \"$HOME/.bashrc\" ] && . \"$HOME/.bashrc\"\n\
         {SH_BRANCH_FN}\
         __brasa_prompt() {{\n\
         \x20 local st=$__brasa_st t b v s0=$'\\ue000' s1=$'\\ue001' s3=$'\\ue003' s4=$'\\ue004' s5=$'\\ue005' nl=$'\\n'\n\
         \x20 printf -v t '%(%H:%M)T' -1\n\
         \x20 b=$(__brasa_branch); b=${{b//[^A-Za-z0-9._\\/+@-]/}}\n\
         \x20 v=${{VIRTUAL_ENV##*/}}; v=${{v//[^A-Za-z0-9._+@-]/}}\n\
         \x20 PS1='\\[\\e[0m\\]'\"$s0\"'\\w'\"${{b:+$s1$b}}${{v:+$s3$v}}${{s4}}${{st}}${{s5}}${{t}}${{nl}}\"{}\n\
         }}\n\
         PROMPT_COMMAND='__brasa_st=$?'\"${{PROMPT_COMMAND:+;$PROMPT_COMMAND}}\"';__brasa_prompt'\n\
         __brasa_prompt\n",
        sq(&bash_line2(p))
    )
}

pub fn zsh_env() -> &'static str {
    "[[ -f \"$BRASA_ORIG_ZDOTDIR/.zshenv\" ]] && source \"$BRASA_ORIG_ZDOTDIR/.zshenv\"\n"
}

pub fn zsh_rc(p: &PromptConfig) -> String {
    let line2 = if p.text.is_empty() {
        String::new()
    } else {
        match p.color {
            Some((r, g, b)) => format!("%{{\x1b[38;2;{r};{g};{b}m%}}{}%{{\x1b[0m%}} ", p.text),
            None => format!("{} ", p.text),
        }
    };
    format!(
        "export ZDOTDIR=\"$BRASA_ORIG_ZDOTDIR\"\n\
         [[ -f \"$ZDOTDIR/.zshrc\" ]] && source \"$ZDOTDIR/.zshrc\"\n\
         {SH_BRANCH_FN}\
         __brasa_prompt() {{\n\
         \x20 local st=$__brasa_st t b v s0=$'\\ue000' s1=$'\\ue001' s3=$'\\ue003' s4=$'\\ue004' s5=$'\\ue005'\n\
         \x20 strftime -s t '%H:%M' $EPOCHSECONDS\n\
         \x20 b=$(__brasa_branch); b=${{b//[^A-Za-z0-9._\\/+@-]/}}\n\
         \x20 v=${{VIRTUAL_ENV:t}}; v=${{v//[^A-Za-z0-9._+@-]/}}\n\
         \x20 PROMPT=\"%{{\"$'\\e[0m'\"%}}${{s0}}%~${{b:+$s1$b}}${{v:+$s3$v}}${{s4}}${{st}}${{s5}}${{t}}\"$'\\n'{}\n\
         \x20 RPROMPT=''\n\
         }}\n\
         __brasa_capture() {{ __brasa_st=$? }}\n\
         zmodload zsh/datetime 2>/dev/null\n\
         precmd_functions=(__brasa_capture ${{precmd_functions[@]}} __brasa_prompt)\n\
         __brasa_prompt\n",
        dollar_q(&line2)
    )
}

pub fn fish_init(p: &PromptConfig) -> String {
    let text = p.text.replace('\\', "\\\\").replace('\'', "\\'");
    let color = p.color.map(|(r, g, b)| format!("set_color {r:02x}{g:02x}{b:02x}; ")).unwrap_or_default();
    let line2 = if p.text.is_empty() { String::new() } else { format!("{color}echo -n '{text} '; set_color normal") };
    format!(
        "function fish_prompt; \
         set -l st $status; \
         set -l b (command git branch --show-current 2>/dev/null); \
         set -l v ''; test -n \"$VIRTUAL_ENV\"; and set v (basename $VIRTUAL_ENV); \
         echo -n \\ue000(string replace -r \"^$HOME\" '~' -- $PWD); \
         test -n \"$b\"; and echo -n \\ue001$b; \
         test -n \"$v\"; and echo -n \\ue003$v; \
         echo -n \\ue004$st\\ue005(command date +%H:%M); \
         echo; {line2}; end; \
         function fish_right_prompt; end"
    )
}

/// Pasta por usuário (0700 em XDG_RUNTIME_DIR; senão ~/.cache) para os arquivos de integração.
fn integration_dir() -> Option<PathBuf> {
    // no Flatpak só esta pasta aparece igual para a caixa e para o computador
    let shared = std::env::var_os("FLATPAK_ID")
        .filter(|_| in_flatpak())
        .and_then(|id| std::env::var_os("XDG_RUNTIME_DIR").map(|r| PathBuf::from(r).join("app").join(id)));
    let base = shared
        .or_else(|| std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from))
        .or_else(|| dirs_home().map(|h| h.join(".cache")))?;
    let dir = base.join("brasa");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn apply_prompt(cmd: &mut CommandBuilder, shell: &str, p: &PromptConfig) {
    let name = Path::new(shell).file_name().and_then(|n| n.to_str()).unwrap_or("");
    let Some(dir) = integration_dir() else { return };
    match name {
        "bash" => {
            let rc = dir.join("bashrc");
            if std::fs::write(&rc, bash_rc(p)).is_ok() {
                cmd.arg("--rcfile");
                cmd.arg(rc);
            }
        }
        "zsh" => {
            let zdir = dir.join("zsh");
            let ok = std::fs::create_dir_all(&zdir).is_ok()
                && std::fs::write(zdir.join(".zshenv"), zsh_env()).is_ok()
                && std::fs::write(zdir.join(".zshrc"), zsh_rc(p)).is_ok();
            if ok {
                let orig = std::env::var_os("ZDOTDIR").map(PathBuf::from).or_else(dirs_home).unwrap_or_default();
                cmd.env("BRASA_ORIG_ZDOTDIR", orig);
                cmd.env("ZDOTDIR", zdir);
            }
        }
        "fish" => {
            cmd.arg("--init-command");
            cmd.arg(fish_init(p));
        }
        _ => {}
    }
}

/// Dentro de um Flatpak o terminal precisa abrir o shell do computador, não o da caixa.
pub fn in_flatpak() -> bool {
    cfg!(unix) && Path::new("/.flatpak-info").exists()
}

/// Troca o comando por `flatpak-spawn --host ...`, levando junto pasta e variáveis.
fn host_wrapped(argv: &[std::ffi::OsString], cwd: Option<&std::ffi::OsStr>, envs: &[(String, String)]) -> Vec<std::ffi::OsString> {
    let mut out: Vec<std::ffi::OsString> = vec!["flatpak-spawn".into(), "--host".into(), "--watch-bus".into()];
    if let Some(c) = cwd {
        let mut a = std::ffi::OsString::from("--directory=");
        a.push(c);
        out.push(a);
    }
    for (k, v) in envs {
        out.push(format!("--env={k}={v}").into());
    }
    out.push("--".into());
    out.extend(argv.iter().cloned());
    out
}

fn host_shell() -> String {
    std::process::Command::new("flatpak-spawn")
        .args(["--host", "printenv", "SHELL"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| s.starts_with('/'))
        .unwrap_or_else(|| "/bin/bash".into())
}

pub fn default_shell() -> String {
    if in_flatpak() {
        host_shell()
    } else if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "powershell.exe".into())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
    }
}

struct Session {
    pid: Option<u32>,
    master: Box<dyn MasterPty + Send>,
    /// fila de escrita: uma thread própria grava no terminal, para quem chama nunca ficar preso
    input: mpsc::Sender<Vec<u8>>,
    child: Box<dyn Child + Send + Sync>,
}

#[derive(Default)]
pub struct PtyManager {
    /// tamanho de uma célula em pixels (largura, altura): vai no tamanho do terminal para os programas
    /// que desenham imagens acertarem a proporção
    cell_px: Mutex<(u16, u16)>,
    next_id: AtomicU64,
    sessions: Mutex<HashMap<SessionId, Session>>,
}

impl PtyManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn spawn(&self, cfg: SpawnConfig, events: Arc<dyn PtyEvents>) -> Result<SessionId, PtyError> {
        let pair = native_pty_system()
            .openpty(self.size(cfg.cols, cfg.rows))
            .map_err(pty_err)?;

        let shell = cfg.shell.clone().unwrap_or_else(default_shell);
        let mut cmd = CommandBuilder::new(&shell);
        if let Some(p) = &cfg.prompt {
            apply_prompt(&mut cmd, &shell, p);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        if let Some(cwd) = cfg.cwd.or_else(dirs_home) {
            cmd.cwd(cwd);
        }

        if in_flatpak() {
            let envs: Vec<(String, String)> = cmd.iter_extra_env_as_str().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            let wrapped = host_wrapped(cmd.get_argv(), cmd.get_cwd().map(|c| c.as_os_str()), &envs);
            cmd = CommandBuilder::from_argv(wrapped);
        }
        let child = pair.slave.spawn_command(cmd).map_err(pty_err)?;
        let pid = child.process_id();
        drop(pair.slave); // sem isso o leitor nunca recebe EOF
        let mut reader = pair.master.try_clone_reader().map_err(pty_err)?;
        let mut writer = pair.master.take_writer().map_err(pty_err)?;
        // Escrever direto trava quando o programa despeja saída e não lê a entrada (psql, make...): a interface
        // parava de consumir a saída, o leitor parava, o programa ficava preso na saída e nunca lia. Uma thread
        // só para escrever desfaz o nó; a ordem dos bytes é mantida pela fila.
        let (input, queue) = mpsc::channel::<Vec<u8>>();
        std::thread::Builder::new().name(format!("pty-writer-{}", self.next_id.load(Ordering::Relaxed) + 1)).spawn(move || {
            while let Ok(data) = queue.recv() {
                if writer.write_all(&data).and_then(|_| writer.flush()).is_err() {
                    break;
                }
            }
        })?;

        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        self.sessions
            .lock()
            .unwrap()
            .insert(id, Session { pid, master: pair.master, input, child });

        std::thread::Builder::new()
            .name(format!("pty-reader-{id}"))
            .spawn(move || {
                let mut buf = [0u8; 16 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => events.on_output(id, &buf[..n]),
                    }
                }
                events.on_exit(id);
            })?;
        Ok(id)
    }

    pub fn write(&self, id: SessionId, data: &[u8]) -> Result<(), PtyError> {
        let map = self.sessions.lock().unwrap();
        let s = map.get(&id).ok_or(PtyError::NoSession(id))?;
        s.input.send(data.to_vec()).map_err(|_| PtyError::Io(std::io::Error::from(std::io::ErrorKind::BrokenPipe)))
    }

    /// Injeta um comando (possivelmente multilinha) como se tivesse sido digitado e confirmado com Enter.
    pub fn run_command(&self, id: SessionId, command: &str) -> Result<(), PtyError> {
        let mut payload = command.trim_end().replace("\r\n", "\n").replace('\n', "\r");
        payload.push('\r');
        self.write(id, payload.as_bytes())
    }

    pub fn resize(&self, id: SessionId, cols: u16, rows: u16) -> Result<(), PtyError> {
        let map = self.sessions.lock().unwrap();
        let s = map.get(&id).ok_or(PtyError::NoSession(id))?;
        s.master.resize(self.size(cols, rows)).map_err(pty_err)
    }

    /// Tamanho de uma célula em pixels, usado nos próximos `spawn` e `resize`.
    pub fn set_cell_px(&self, w: f32, h: f32) {
        *self.cell_px.lock().unwrap() = (w.round().clamp(0.0, 255.0) as u16, h.round().clamp(0.0, 255.0) as u16);
    }

    fn size(&self, cols: u16, rows: u16) -> PtySize {
        let (w, h) = *self.cell_px.lock().unwrap();
        PtySize { rows, cols, pixel_width: cols.saturating_mul(w), pixel_height: rows.saturating_mul(h) }
    }

    /// Pasta atual do processo do shell (Linux: via /proc). `None` em outras plataformas.
    pub fn cwd(&self, id: SessionId) -> Option<PathBuf> {
        let pid = self.sessions.lock().unwrap().get(&id)?.pid?;
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
    }

    /// Nome do processo do shell (ex.: "bash", "zsh"), via /proc.
    pub fn shell_name(&self, id: SessionId) -> Option<String> {
        let pid = self.sessions.lock().unwrap().get(&id)?.pid?;
        let name = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
        Some(name.trim().to_string()).filter(|n| !n.is_empty())
    }

    pub fn kill(&self, id: SessionId) -> Result<(), PtyError> {
        let mut s = self.sessions.lock().unwrap().remove(&id).ok_or(PtyError::NoSession(id))?;
        let _ = s.child.kill();
        Ok(())
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn host_wrapped_leva_pasta_e_variaveis() {
        let argv = vec!["/bin/bash".into(), "--rcfile".into(), "/run/x".into()];
        let w = host_wrapped(&argv, Some(std::ffi::OsStr::new("/home/u")), &[("TERM".into(), "xterm-256color".into())]);
        let w: Vec<String> = w.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(w, ["flatpak-spawn", "--host", "--watch-bus", "--directory=/home/u", "--env=TERM=xterm-256color", "--", "/bin/bash", "--rcfile", "/run/x"]);
    }

    use std::sync::mpsc;
    use std::time::Duration;

    struct Collect(Mutex<mpsc::Sender<Vec<u8>>>);
    impl PtyEvents for Collect {
        fn on_output(&self, _: SessionId, b: &[u8]) {
            let _ = self.0.lock().unwrap().send(b.to_vec());
        }
        fn on_exit(&self, _: SessionId) {}
    }

    /// Consumidor lento, como uma interface ocupada: cada pedaço de saída demora a ser aceito.
    struct Slow;
    impl PtyEvents for Slow {
        fn on_output(&self, _: SessionId, _: &[u8]) {
            std::thread::sleep(Duration::from_millis(5));
        }
        fn on_exit(&self, _: SessionId) {}
    }

    /// Regressão do travamento no psql: o programa despeja saída sem ler a entrada, e escrever nele (colar, respostas
    /// do terminal) não pode prender quem chama.
    #[test]
    fn writing_to_a_busy_program_never_blocks_the_caller() {
        let mgr = Arc::new(PtyManager::new());
        let id = mgr.spawn(SpawnConfig { shell: Some("/bin/sh".into()), ..Default::default() }, Arc::new(Slow)).unwrap();
        mgr.run_command(id, "stty raw -echo; yes | head -c 300000000").unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let (done_tx, done_rx) = mpsc::channel();
        let m2 = mgr.clone();
        std::thread::spawn(move || {
            // 1 MB: bem mais do que cabe na fila do terminal
            let r = m2.write(id, &vec![b'x'; 1 << 20]);
            let _ = done_tx.send(r.is_ok());
        });
        assert_eq!(done_rx.recv_timeout(Duration::from_secs(2)), Ok(true), "write travou");
        // as outras chamadas também seguem livres
        assert!(mgr.resize(id, 100, 30).is_ok());
        mgr.kill(id).unwrap();
    }

    #[test]
    fn run_command_reaches_shell_and_echoes_output() {
        let (tx, rx) = mpsc::channel();
        let mgr = PtyManager::new();
        let id = mgr
            .spawn(
                SpawnConfig { shell: Some("/bin/sh".into()), ..Default::default() },
                Arc::new(Collect(Mutex::new(tx))),
            )
            .unwrap();
        mgr.run_command(id, "echo oxi_$((40+2))").unwrap();

        let mut seen = String::new();
        while let Ok(chunk) = rx.recv_timeout(Duration::from_secs(5)) {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains("oxi_42") {
                break;
            }
        }
        mgr.kill(id).unwrap();
        assert!(seen.contains("oxi_42"), "saída: {seen:?}");
    }

    fn prompt(text: &str) -> PromptConfig {
        PromptConfig { text: text.into(), color: Some((179, 146, 240)) }
    }

    #[test]
    fn generated_scripts_load_user_rc_first_and_set_prompt_last() {
        let b = bash_rc(&prompt("❯"));
        assert!(b.find(".bashrc").unwrap() < b.find("PS1=").unwrap());
        assert!(b.contains("38;2;179;146;240"));
        assert!(b.contains("PROMPT_COMMAND")); // reaplica a cada comando
        assert!(b.contains(r"\w")); // pasta na primeira linha
        assert!(b.contains("__brasa_st=$?") && b.contains("%(%H:%M)T")); // status e hora

        let z = zsh_rc(&prompt("❯"));
        assert!(z.find(".zshrc").unwrap() < z.find("PROMPT=").unwrap());
        assert!(z.contains("precmd_functions") && z.contains("%~"));
        assert!(z.contains("__brasa_capture") && z.contains("strftime"));

        let f = fish_init(&prompt("❯"));
        assert!(f.contains("function fish_prompt") && f.contains("b392f0") && f.contains("ue000"));
        assert!(f.contains("set -l st $status") && f.contains("ue004"));
    }

    #[test]
    fn prompt_text_is_quoted_safely() {
        let evil = prompt("a'b$(touch x)\\");
        assert!(bash_rc(&evil).contains(r#"a'\''b$(touch x)"#)); // aspas simples fechadas e reabertas
        assert!(zsh_rc(&evil).contains(r"a\'b$(touch x)"));
        assert!(fish_init(&evil).contains(r"a\'b$(touch x)"));
        assert_eq!(bash_line2(&PromptConfig { text: String::new(), color: None }), "");
    }

    #[test]
    fn parse_prompt_row_extracts_path_branch_and_venv() {
        let row = format!("{PROMPT_START}~/proj{PROMPT_BRANCH}feat/x{PROMPT_VENV}venv{PROMPT_STATUS}130{PROMPT_TIME}14:21   ");
        assert_eq!(
            parse_prompt_row(&row),
            Some(PromptInfo {
                path: "~/proj".into(),
                branch: Some("feat/x".into()),
                venv: Some("venv".into()),
                status: Some(130),
                time: Some("14:21".into()),
            })
        );
        // status vazio (primeiro prompt da sessão) vira None
        let first = format!("{PROMPT_START}~{PROMPT_STATUS}{PROMPT_TIME}09:05");
        let info = parse_prompt_row(&first).unwrap();
        assert_eq!((info.status, info.time.as_deref()), (None, Some("09:05")));
        let only_path = format!("{PROMPT_START}/tmp   ");
        assert_eq!(parse_prompt_row(&only_path), Some(PromptInfo { path: "/tmp".into(), ..Default::default() }));
        assert_eq!(parse_prompt_row("user@host:~$ "), None);
        assert_eq!(parse_prompt_row(""), None);
    }

    #[test]
    fn bash_prompt_prints_markers_with_path_and_branch() {
        if !Path::new("/bin/bash").exists() {
            return;
        }
        let repo = std::env::temp_dir().join(format!("oxi-git-{}", std::process::id()));
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/feat/x\n").unwrap();
        let repo = repo.canonicalize().unwrap();

        let (tx, rx) = mpsc::channel();
        let mgr = PtyManager::new();
        let id = mgr
            .spawn(
                SpawnConfig {
                    shell: Some("/bin/bash".into()),
                    cwd: Some(repo.clone()),
                    prompt: Some(prompt("OXI>")),
                    ..Default::default()
                },
                Arc::new(Collect(Mutex::new(tx))),
            )
            .unwrap();
        let mut seen = String::new();
        while let Ok(chunk) = rx.recv_timeout(Duration::from_secs(5)) {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains("OXI>") {
                break;
            }
        }
        // roda `false` e confere que o PRÓXIMO prompt informa o código de saída 1
        mgr.run_command(id, "false").unwrap();
        let mut after = String::new();
        let started = std::time::Instant::now();
        while started.elapsed() < Duration::from_secs(5) {
            if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(200)) {
                after.push_str(&String::from_utf8_lossy(&chunk));
            }
            if after.matches(PROMPT_START).count() >= 1 && after.contains("OXI>") {
                break;
            }
        }
        let second = after.lines().find(|l| l.contains(PROMPT_START)).expect(&format!("sem 2º prompt: {after:?}"));
        let second: String = second.chars().filter(|c| !c.is_control()).collect();
        let second = parse_prompt_row(second.trim_start_matches(|c| c != PROMPT_START)).unwrap();
        assert_eq!(second.status, Some(1), "{second:?}");
        assert!(second.time.as_deref().is_some_and(|t| t.len() == 5 && t.contains(':')), "{second:?}");

        mgr.kill(id).unwrap();
        let _ = std::fs::remove_dir_all(&repo);

        let line = seen.lines().find(|l| l.contains(PROMPT_START)).expect(&format!("sem linha de prompt: {seen:?}"));
        let clean: String = line.chars().filter(|c| !c.is_control()).collect();
        // remove sequências de cor ESC[...m que sobraram como texto
        let info = parse_prompt_row(clean.trim_start_matches(|c| c != PROMPT_START)).unwrap();
        assert!(info.path.ends_with(repo.file_name().unwrap().to_str().unwrap()), "{info:?}");
        assert_eq!(info.branch.as_deref(), Some("feat/x"));
        assert!(seen.contains("OXI>"));
    }

    #[test]
    fn bash_starts_with_minimal_prompt_instead_of_user_at_host() {
        if !Path::new("/bin/bash").exists() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let mgr = PtyManager::new();
        let id = mgr
            .spawn(
                SpawnConfig { shell: Some("/bin/bash".into()), prompt: Some(prompt("OXI>")), ..Default::default() },
                Arc::new(Collect(Mutex::new(tx))),
            )
            .unwrap();
        let mut seen = String::new();
        while let Ok(chunk) = rx.recv_timeout(Duration::from_secs(5)) {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains("OXI>") {
                break;
            }
        }
        mgr.kill(id).unwrap();
        assert!(seen.contains("OXI>"), "saída: {seen:?}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cwd_follows_the_shell() {
        let (tx, _rx) = mpsc::channel();
        let mgr = PtyManager::new();
        let dir = std::env::temp_dir().canonicalize().unwrap();
        let id = mgr
            .spawn(
                SpawnConfig { shell: Some("/bin/sh".into()), cwd: Some(dir.clone()), ..Default::default() },
                Arc::new(Collect(Mutex::new(tx))),
            )
            .unwrap();
        // logo após o spawn o processo filho pode ainda não ter feito exec do shell: espera um pouco
        let wait_for = |ok: &dyn Fn() -> bool| {
            for _ in 0..50 {
                if ok() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        wait_for(&|| mgr.shell_name(id).as_deref() == Some("sh"));
        assert_eq!(mgr.cwd(id), Some(dir));
        assert_eq!(mgr.shell_name(id).as_deref(), Some("sh"));
        mgr.run_command(id, "cd /").unwrap();
        wait_for(&|| mgr.cwd(id) == Some(PathBuf::from("/")));
        assert_eq!(mgr.cwd(id), Some(PathBuf::from("/")));
        mgr.kill(id).unwrap();
    }
}
