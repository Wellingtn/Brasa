//! Cliente MCP (Model Context Protocol) pelo transporte stdio.
//!
//! Cada servidor é um processo; as mensagens são JSON-RPC 2.0, uma por linha, no `stdin` e no `stdout` dele.
//! Sem dependência de janela. Este crate abre o servidor, faz o `initialize`, lista ferramentas, recursos e
//! prompts, chama ferramentas e guarda o `stderr`. Toda pergunta tem tempo limite, para um servidor travado
//! não travar quem chamou.
//!
//! Fora do escopo por enquanto: o transporte HTTP, `sampling` e `roots` pedidos pelo servidor (respondemos
//! "método não encontrado") e assinaturas de recursos.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// Versão do protocolo que pedimos; o servidor responde com a que aceita.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// Maior linha aceita de um servidor (uma mensagem inteira).
pub const MAX_LINE: usize = 16 * 1024 * 1024;
/// Linhas de `stderr` guardadas por servidor.
pub const MAX_LOG_LINES: usize = 1000;
const MAX_PAGES: usize = 50;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("não consegui abrir o servidor: {0}")]
    Spawn(String),
    #[error("o servidor não respondeu a tempo ({0})")]
    Timeout(String),
    #[error("o servidor fechou a conexão")]
    Closed,
    #[error("erro do servidor ({code}): {message}")]
    Rpc { code: i64, message: String },
    #[error("resposta fora do protocolo: {0}")]
    Protocol(String),
    #[error("configuração inválida: {0}")]
    Config(String),
}

// ---------------------------------------------------------------------------------------------
// Configuração (`mcp.json`)

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

/// Um item do arquivo: servidor por stdio (o que sabemos abrir) ou remoto por endereço (ainda não).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Stdio(ServerConfig),
    Remote { name: String, url: String },
}

impl Entry {
    pub fn name(&self) -> &str {
        match self {
            Entry::Stdio(c) => &c.name,
            Entry::Remote { name, .. } => name,
        }
    }
}

/// Lê o formato do ecossistema: `{"mcpServers": {"nome": {"command": ..., "args": [...], "env": {...}}}}`.
/// Mantém a ordem do arquivo. Texto vazio vale como "nenhum servidor".
pub fn parse_config(text: &str) -> Result<Vec<Entry>, Error> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let root: Value = serde_json::from_str(text).map_err(|e| Error::Config(e.to_string()))?;
    let Some(servers) = root.get("mcpServers") else { return Ok(Vec::new()) };
    let servers = servers.as_object().ok_or_else(|| Error::Config("\"mcpServers\" deve ser um objeto".into()))?;
    let mut out = Vec::new();
    for (name, v) in servers {
        if let Some(url) = v.get("url").and_then(Value::as_str) {
            out.push(Entry::Remote { name: name.clone(), url: url.to_string() });
            continue;
        }
        let command = v.get("command").and_then(Value::as_str).filter(|c| !c.trim().is_empty());
        let command = command.ok_or_else(|| Error::Config(format!("o servidor \"{name}\" não tem \"command\"")))?;
        let args = match v.get("args") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(a)) => a.iter().map(|x| x.as_str().map(str::to_string).ok_or_else(|| Error::Config(format!("\"args\" de \"{name}\" só aceita texto")))).collect::<Result<_, _>>()?,
            Some(_) => return Err(Error::Config(format!("\"args\" de \"{name}\" deve ser uma lista"))),
        };
        let env = match v.get("env") {
            None | Some(Value::Null) => BTreeMap::new(),
            Some(Value::Object(o)) => o.iter().map(|(k, x)| (k.clone(), x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string()))).collect(),
            Some(_) => return Err(Error::Config(format!("\"env\" de \"{name}\" deve ser um objeto"))),
        };
        out.push(Entry::Stdio(ServerConfig { name: name.clone(), command: command.to_string(), args, env }));
    }
    Ok(out)
}

/// Texto do arquivo para estes servidores (o inverso de [`parse_config`], para os servidores por stdio).
pub fn render_config(entries: &[Entry]) -> String {
    let mut servers = serde_json::Map::new();
    for e in entries {
        match e {
            Entry::Stdio(c) => {
                let mut o = serde_json::Map::new();
                o.insert("command".into(), json!(c.command));
                o.insert("args".into(), json!(c.args));
                if !c.env.is_empty() {
                    o.insert("env".into(), json!(c.env));
                }
                servers.insert(c.name.clone(), Value::Object(o));
            }
            Entry::Remote { name, url } => {
                servers.insert(name.clone(), json!({ "url": url }));
            }
        }
    }
    serde_json::to_string_pretty(&json!({ "mcpServers": servers })).unwrap_or_default() + "\n"
}

/// Grava o arquivo só para o dono (0600): ele guarda chaves de API.
pub fn save_config(path: &Path, entries: &[Entry]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        let mut f = opts.open(&tmp)?;
        f.write_all(render_config(entries).as_bytes())?;
    }
    std::fs::rename(tmp, path)
}

// ---------------------------------------------------------------------------------------------
// Tipos do protocolo

#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    pub uri: String,
    pub name: String,
    pub description: String,
    pub mime_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub name: String,
    pub description: String,
    pub arguments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    pub protocol_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub is_error: bool,
    /// o texto do resultado; conteúdo que não é texto aparece como `[image image/png]`
    pub text: String,
}

fn text_of(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn parse_tool(v: &Value) -> Option<Tool> {
    Some(Tool { name: v.get("name")?.as_str()?.to_string(), description: text_of(v, "description"), input_schema: v.get("inputSchema").cloned().unwrap_or(Value::Null) })
}

fn parse_resource(v: &Value) -> Option<Resource> {
    let uri = v.get("uri")?.as_str()?.to_string();
    let name = v.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| uri.clone());
    Some(Resource { uri, name, description: text_of(v, "description"), mime_type: text_of(v, "mimeType") })
}

fn parse_prompt(v: &Value) -> Option<Prompt> {
    let arguments = v.get("arguments").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.get("name").and_then(Value::as_str).map(str::to_string)).collect()).unwrap_or_default();
    Some(Prompt { name: v.get("name")?.as_str()?.to_string(), description: text_of(v, "description"), arguments })
}

fn parse_tool_result(v: &Value) -> ToolResult {
    let is_error = v.get("isError").and_then(Value::as_bool).unwrap_or(false);
    let parts: Vec<String> = v
        .get("content")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|c| match c.get("type").and_then(Value::as_str) {
                    Some("text") => text_of(c, "text"),
                    Some(t) => format!("[{t} {}]", text_of(c, "mimeType")).replace(" ]", "]"),
                    None => String::new(),
                })
                .collect()
        })
        .unwrap_or_default();
    ToolResult { is_error, text: parts.join("\n") }
}

// ---------------------------------------------------------------------------------------------
// Cliente

type Reply = Result<Value, Error>;

struct Shared {
    pending: Mutex<HashMap<u64, Sender<Reply>>>,
    logs: Mutex<VecDeque<String>>,
    alive: AtomicBool,
}

pub struct Client {
    pub config: ServerConfig,
    pub info: ServerInfo,
    pub tools: Vec<Tool>,
    pub resources: Vec<Resource>,
    pub prompts: Vec<Prompt>,
    child: Mutex<Child>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    shared: Arc<Shared>,
    next_id: AtomicU64,
}

fn push_log(shared: &Shared, line: String) {
    let mut l = shared.logs.lock().unwrap();
    if l.len() >= MAX_LOG_LINES {
        l.pop_front();
    }
    l.push_back(line);
}

/// Lê uma linha de no máximo `MAX_LINE` bytes; `Ok(None)` no fim do fluxo, erro se passar do limite.
fn read_line_capped<R: BufRead>(r: &mut R) -> std::io::Result<Option<Vec<u8>>> {
    let mut buf = Vec::new();
    let n = r.by_ref().take(MAX_LINE as u64 + 1).read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(None);
    }
    if buf.len() > MAX_LINE {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "mensagem grande demais"));
    }
    Ok(Some(buf))
}

fn fail_all(shared: &Shared) {
    shared.alive.store(false, Ordering::SeqCst);
    for (_, tx) in shared.pending.lock().unwrap().drain() {
        let _ = tx.send(Err(Error::Closed));
    }
}

impl Client {
    /// Abre o servidor, faz o `initialize` e lista o que ele oferece. `timeout` vale para cada pergunta.
    pub fn start(config: &ServerConfig, timeout: Duration) -> Result<Client, Error> {
        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args).envs(&config.env).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let mut child = cmd.spawn().map_err(|e| Error::Spawn(format!("{}: {e}", config.command)))?;
        let stdin = child.stdin.take().ok_or_else(|| Error::Spawn("sem stdin".into()))?;
        let stdout = child.stdout.take().ok_or_else(|| Error::Spawn("sem stdout".into()))?;
        let stderr = child.stderr.take().ok_or_else(|| Error::Spawn("sem stderr".into()))?;
        let shared = Arc::new(Shared { pending: Mutex::new(HashMap::new()), logs: Mutex::new(VecDeque::new()), alive: AtomicBool::new(true) });

        let sh = shared.clone();
        std::thread::spawn(move || {
            let mut r = BufReader::new(stderr);
            while let Ok(Some(line)) = read_line_capped(&mut r) {
                push_log(&sh, String::from_utf8_lossy(&line).trim_end().to_string());
            }
        });

        let stdin = Arc::new(Mutex::new(Some(stdin)));
        let me_shared = shared.clone();
        let sh = shared.clone();
        // as respostas do servidor a perguntas dele (ping) saem pelo mesmo stdin
        let (answer_tx, answer_rx) = channel::<String>();
        std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            loop {
                match read_line_capped(&mut r) {
                    Ok(Some(line)) => {
                        let Ok(msg) = serde_json::from_slice::<Value>(&line) else {
                            push_log(&sh, format!("[mcp] linha que não é JSON: {}", String::from_utf8_lossy(&line).trim_end().chars().take(200).collect::<String>()));
                            continue;
                        };
                        let id = msg.get("id").cloned();
                        let is_response = msg.get("result").is_some() || msg.get("error").is_some();
                        match (id, is_response, msg.get("method").and_then(Value::as_str)) {
                            (Some(id), true, _) => {
                                let reply = match msg.get("error") {
                                    Some(e) => Err(Error::Rpc { code: e.get("code").and_then(Value::as_i64).unwrap_or(0), message: text_of(e, "message") }),
                                    None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                                };
                                if let Some(tx) = id.as_u64().and_then(|n| sh.pending.lock().unwrap().remove(&n)) {
                                    let _ = tx.send(reply);
                                }
                            }
                            // pergunta do servidor: só `ping` tem resposta; o resto é "método não encontrado"
                            (Some(id), false, Some(method)) => {
                                let ans = if method == "ping" {
                                    json!({"jsonrpc": "2.0", "id": id, "result": {}})
                                } else {
                                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "método não suportado pelo Brasa"}})
                                };
                                let _ = answer_tx.send(ans.to_string());
                            }
                            _ => {} // notificações (progresso, mudança de lista) ficam para depois
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        push_log(&sh, format!("[mcp] {e}"));
                        break;
                    }
                }
            }
            fail_all(&sh);
        });

        let client = Client {
            config: config.clone(),
            info: ServerInfo { name: String::new(), version: String::new(), protocol_version: String::new() },
            tools: Vec::new(),
            resources: Vec::new(),
            prompts: Vec::new(),
            child: Mutex::new(child),
            stdin,
            shared: me_shared,
            next_id: AtomicU64::new(1),
        };
        // quem escreve as respostas de ping: uma thread própria, para a de leitura nunca esperar a escrita
        {
            let (shared, stdin) = (client.shared.clone(), client.stdin.clone());
            std::thread::spawn(move || {
                for line in answer_rx {
                    if !shared.alive.load(Ordering::SeqCst) || client_write_to(&stdin, &line).is_err() {
                        break;
                    }
                }
            });
        }
        client.handshake(timeout)
    }

    fn handshake(mut self, timeout: Duration) -> Result<Client, Error> {
        let init = self.request(
            "initialize",
            json!({ "protocolVersion": PROTOCOL_VERSION, "capabilities": {}, "clientInfo": { "name": "brasa", "version": env!("CARGO_PKG_VERSION") } }),
            timeout,
        );
        let init = match init {
            Ok(v) => v,
            Err(e) => {
                self.shutdown();
                return Err(e);
            }
        };
        let info = init.get("serverInfo").cloned().unwrap_or(Value::Null);
        self.info = ServerInfo { name: text_of(&info, "name"), version: text_of(&info, "version"), protocol_version: text_of(&init, "protocolVersion") };
        if self.info.protocol_version.is_empty() {
            self.shutdown();
            return Err(Error::Protocol("o servidor não disse a versão do protocolo".into()));
        }
        self.notify("notifications/initialized", json!({}))?;
        let caps = init.get("capabilities").cloned().unwrap_or(Value::Null);
        let step = |cap: &str, method: &str, key: &str| -> Result<Vec<Value>, Error> {
            if caps.get(cap).is_none() {
                return Ok(Vec::new());
            }
            self.list_all(method, key, timeout)
        };
        let (tools, resources, prompts) = (step("tools", "tools/list", "tools"), step("resources", "resources/list", "resources"), step("prompts", "prompts/list", "prompts"));
        match (tools, resources, prompts) {
            (Ok(t), Ok(r), Ok(p)) => {
                self.tools = t.iter().filter_map(parse_tool).collect();
                self.resources = r.iter().filter_map(parse_resource).collect();
                self.prompts = p.iter().filter_map(parse_prompt).collect();
                Ok(self)
            }
            (t, r, p) => {
                self.shutdown();
                Err(t.err().or(r.err()).or(p.err()).unwrap_or(Error::Closed))
            }
        }
    }

    /// Junta as páginas de uma listagem (`nextCursor`), no máximo `MAX_PAGES`.
    fn list_all(&self, method: &str, key: &str, timeout: Duration) -> Result<Vec<Value>, Error> {
        let (mut all, mut cursor) = (Vec::new(), None::<String>);
        for _ in 0..MAX_PAGES {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let res = self.request(method, params, timeout)?;
            if let Some(items) = res.get(key).and_then(Value::as_array) {
                all.extend(items.iter().cloned());
            }
            match res.get("nextCursor").and_then(Value::as_str) {
                Some(c) if !c.is_empty() => cursor = Some(c.to_string()),
                _ => break,
            }
        }
        Ok(all)
    }

    fn write_line(&self, line: &str) -> Result<(), Error> {
        client_write_to(&self.stdin, line)
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), Error> {
        self.write_line(&json!({ "jsonrpc": "2.0", "method": method, "params": params }).to_string())
    }

    /// Uma pergunta ao servidor, com tempo limite.
    pub fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, Error> {
        if !self.shared.alive.load(Ordering::SeqCst) {
            return Err(Error::Closed);
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = channel();
        self.shared.pending.lock().unwrap().insert(id, tx);
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string();
        if let Err(e) = self.write_line(&msg) {
            self.shared.pending.lock().unwrap().remove(&id);
            return Err(e);
        }
        match rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(_) => {
                self.shared.pending.lock().unwrap().remove(&id);
                if !self.shared.alive.load(Ordering::SeqCst) {
                    return Err(Error::Closed);
                }
                // avisa o servidor que já não esperamos a resposta
                let _ = self.notify("notifications/cancelled", json!({ "requestId": id, "reason": "tempo esgotado" }));
                Err(Error::Timeout(method.to_string()))
            }
        }
    }

    pub fn call_tool(&self, name: &str, arguments: Value, timeout: Duration) -> Result<ToolResult, Error> {
        let args = if arguments.is_null() { json!({}) } else { arguments };
        let res = self.request("tools/call", json!({ "name": name, "arguments": args }), timeout)?;
        Ok(parse_tool_result(&res))
    }

    /// Texto de um recurso (`resources/read`); partes binárias aparecem como `[blob tipo]`.
    pub fn read_resource(&self, uri: &str, timeout: Duration) -> Result<String, Error> {
        let res = self.request("resources/read", json!({ "uri": uri }), timeout)?;
        let parts: Vec<String> = res
            .get("contents")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(|c| c.get("text").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("[blob {}]", text_of(c, "mimeType")))).collect())
            .unwrap_or_default();
        Ok(parts.join("\n"))
    }

    pub fn is_alive(&self) -> bool {
        self.shared.alive.load(Ordering::SeqCst)
    }

    /// As últimas linhas que o servidor escreveu no `stderr` (e avisos do próprio cliente).
    pub fn logs(&self) -> Vec<String> {
        self.shared.logs.lock().unwrap().iter().cloned().collect()
    }

    /// Fecha o `stdin` (os servidores saem sozinhos) e, se não saírem em 2 s, mata o grupo de processos.
    pub fn shutdown(&mut self) {
        self.stdin.lock().unwrap().take();
        let mut child = self.child.lock().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
                _ => {
                    #[cfg(unix)]
                    unsafe {
                        libc::kill(-(child.id() as i32), libc::SIGKILL);
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
            }
        }
        self.shared.alive.store(false, Ordering::SeqCst);
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if self.shared.alive.load(Ordering::SeqCst) || self.stdin.lock().map(|s| s.is_some()).unwrap_or(false) {
            self.shutdown();
        }
    }
}

fn client_write_to(stdin: &Mutex<Option<ChildStdin>>, line: &str) -> Result<(), Error> {
    let mut g = stdin.lock().unwrap();
    let Some(w) = g.as_mut() else { return Err(Error::Closed) };
    w.write_all(line.as_bytes()).and_then(|_| w.write_all(b"\n")).and_then(|_| w.flush()).map_err(|_| Error::Closed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Servidor MCP de mentira, em Python. O primeiro argumento escolhe o comportamento.
    const MOCK: &str = r#"
import sys, json, time
mode = sys.argv[1] if len(sys.argv) > 1 else "normal"
def send(o): sys.stdout.write(json.dumps(o) + "\n"); sys.stdout.flush()
sys.stderr.write("mock pronto\n"); sys.stderr.flush()
for line in sys.stdin:
    msg = json.loads(line)
    m, i = msg.get("method"), msg.get("id")
    if "result" in msg and i == "srv1":
        sys.stderr.write("ping-ok\n"); sys.stderr.flush(); continue
    if m == "initialize":
        if mode == "crash": sys.exit(3)
        if mode == "noisy": sys.stdout.write("isto nao e json\n"); sys.stdout.flush()
        caps = {} if mode == "nocaps" else {"tools": {}, "resources": {}, "prompts": {}}
        send({"jsonrpc": "2.0", "id": i, "result": {"protocolVersion": "2025-06-18", "capabilities": caps, "serverInfo": {"name": "mock", "version": "1.0"}}})
    elif m == "notifications/initialized":
        send({"jsonrpc": "2.0", "id": "srv1", "method": "ping"})
    elif m == "tools/list":
        if msg["params"].get("cursor") == "p2":
            send({"jsonrpc": "2.0", "id": i, "result": {"tools": [{"name": "boom", "description": "falha", "inputSchema": {"type": "object"}}]}})
        else:
            send({"jsonrpc": "2.0", "id": i, "result": {"tools": [{"name": "echo", "description": "repete", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}}, {"name": "slow"}, {"name": "rpcerr"}, {"name": "img"}], "nextCursor": "p2"}})
    elif m == "resources/list":
        send({"jsonrpc": "2.0", "id": i, "result": {"resources": [{"uri": "file:///a.txt", "name": "a", "mimeType": "text/plain"}]}})
    elif m == "resources/read":
        send({"jsonrpc": "2.0", "id": i, "result": {"contents": [{"uri": msg["params"]["uri"], "text": "conteudo de a"}]}})
    elif m == "prompts/list":
        send({"jsonrpc": "2.0", "id": i, "result": {"prompts": [{"name": "resumo", "description": "resume", "arguments": [{"name": "texto"}]}]}})
    elif m == "tools/call":
        n = msg["params"]["name"]
        if n == "echo": send({"jsonrpc": "2.0", "id": i, "result": {"content": [{"type": "text", "text": msg["params"]["arguments"].get("text", "")}]}})
        elif n == "boom": send({"jsonrpc": "2.0", "id": i, "result": {"isError": True, "content": [{"type": "text", "text": "deu ruim"}]}})
        elif n == "rpcerr": send({"jsonrpc": "2.0", "id": i, "error": {"code": -32602, "message": "argumento invalido"}})
        elif n == "img": send({"jsonrpc": "2.0", "id": i, "result": {"content": [{"type": "image", "mimeType": "image/png", "data": "AAAA"}]}})
        # "slow": nunca responde
if mode == "stubborn": time.sleep(60)
"#;

    fn script(dir: &Path) -> std::path::PathBuf {
        let p = dir.join("mock.py");
        std::fs::write(&p, MOCK).unwrap();
        p
    }

    fn cfg(dir: &Path, mode: &str) -> ServerConfig {
        ServerConfig { name: "mock".into(), command: "python3".into(), args: vec!["-u".into(), script(dir).to_string_lossy().into_owned(), mode.into()], env: BTreeMap::new() }
    }

    const T: Duration = Duration::from_secs(5);

    #[test]
    fn handshake_lists_tools_across_pages_resources_and_prompts() {
        let d = tempfile::tempdir().unwrap();
        let c = Client::start(&cfg(d.path(), "normal"), T).unwrap();
        assert_eq!((c.info.name.as_str(), c.info.version.as_str(), c.info.protocol_version.as_str()), ("mock", "1.0", "2025-06-18"));
        let names: Vec<_> = c.tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["echo", "slow", "rpcerr", "img", "boom"]);
        assert_eq!(c.tools[0].description, "repete");
        assert_eq!(c.tools[0].input_schema["properties"]["text"]["type"], "string");
        assert_eq!(c.resources[0].uri, "file:///a.txt");
        assert_eq!(c.prompts[0].arguments, ["texto"]);
        assert!(c.is_alive());
    }

    #[test]
    fn tool_calls_return_text_errors_and_non_text() {
        let d = tempfile::tempdir().unwrap();
        let c = Client::start(&cfg(d.path(), "normal"), T).unwrap();
        assert_eq!(c.call_tool("echo", json!({"text": "olá"}), T).unwrap(), ToolResult { is_error: false, text: "olá".into() });
        assert_eq!(c.call_tool("boom", Value::Null, T).unwrap(), ToolResult { is_error: true, text: "deu ruim".into() });
        assert_eq!(c.call_tool("img", json!({}), T).unwrap().text, "[image image/png]");
        match c.call_tool("rpcerr", json!({}), T) {
            Err(Error::Rpc { code, message }) => assert_eq!((code, message.as_str()), (-32602, "argumento invalido")),
            other => panic!("esperava erro do servidor, veio {other:?}"),
        }
        assert_eq!(c.read_resource("file:///a.txt", T).unwrap(), "conteudo de a");
    }

    #[test]
    fn a_silent_tool_times_out_and_the_connection_keeps_working() {
        let d = tempfile::tempdir().unwrap();
        let c = Client::start(&cfg(d.path(), "normal"), T).unwrap();
        let started = Instant::now();
        assert!(matches!(c.call_tool("slow", json!({}), Duration::from_millis(300)), Err(Error::Timeout(_))));
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(c.call_tool("echo", json!({"text": "ainda vivo"}), T).unwrap().text, "ainda vivo");
    }

    #[test]
    fn a_server_that_dies_during_the_handshake_is_an_error_not_a_hang() {
        let d = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let r = Client::start(&cfg(d.path(), "crash"), T);
        assert!(matches!(r, Err(Error::Closed)), "veio {:?}", r.err());
        assert!(started.elapsed() < Duration::from_secs(4));
    }

    #[test]
    fn missing_command_is_a_spawn_error() {
        let c = ServerConfig { name: "x".into(), command: "/nao/existe/servidor".into(), args: vec![], env: BTreeMap::new() };
        assert!(matches!(Client::start(&c, T), Err(Error::Spawn(_))));
    }

    #[test]
    fn stderr_is_kept_and_server_pings_are_answered() {
        let d = tempfile::tempdir().unwrap();
        let c = Client::start(&cfg(d.path(), "normal"), T).unwrap();
        // o ping é respondido por outra thread: espera um pouco pelo eco no stderr
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !c.logs().iter().any(|l| l == "ping-ok") {
            std::thread::sleep(Duration::from_millis(25));
        }
        let logs = c.logs();
        assert!(logs.iter().any(|l| l == "mock pronto"), "{logs:?}");
        assert!(logs.iter().any(|l| l == "ping-ok"), "{logs:?}");
    }

    #[test]
    fn noise_on_stdout_is_logged_and_ignored() {
        let d = tempfile::tempdir().unwrap();
        let c = Client::start(&cfg(d.path(), "noisy"), T).unwrap();
        assert!(c.logs().iter().any(|l| l.contains("não é JSON")), "{:?}", c.logs());
        assert_eq!(c.tools.len(), 5);
    }

    #[test]
    fn capabilities_decide_what_is_listed() {
        let d = tempfile::tempdir().unwrap();
        let c = Client::start(&cfg(d.path(), "nocaps"), T).unwrap();
        assert!(c.tools.is_empty() && c.resources.is_empty() && c.prompts.is_empty());
    }

    #[test]
    fn shutdown_kills_a_server_that_ignores_stdin_closing() {
        let d = tempfile::tempdir().unwrap();
        let mut c = Client::start(&cfg(d.path(), "stubborn"), T).unwrap();
        let started = Instant::now();
        c.shutdown();
        assert!(!c.is_alive());
        assert!(started.elapsed() < Duration::from_secs(6), "demorou {:?}", started.elapsed());
        assert!(matches!(c.call_tool("echo", json!({}), T), Err(Error::Closed)));
    }

    #[test]
    fn config_is_read_in_order_and_validated() {
        let text = r#"{"mcpServers": {
            "github": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-github"], "env": {"TOKEN": "abc", "N": 5}},
            "remoto": {"url": "https://exemplo.com/mcp"},
            "simples": {"command": "meu-servidor"}
        }}"#;
        let e = parse_config(text).unwrap();
        assert_eq!(e.iter().map(Entry::name).collect::<Vec<_>>(), ["github", "remoto", "simples"]);
        let Entry::Stdio(g) = &e[0] else { panic!() };
        assert_eq!((g.command.as_str(), g.args.len()), ("npx", 2));
        assert_eq!(g.env["N"], "5");
        assert!(matches!(&e[1], Entry::Remote { url, .. } if url == "https://exemplo.com/mcp"));
        assert_eq!(parse_config("").unwrap(), vec![]);
        assert_eq!(parse_config("{}").unwrap(), vec![]);
        assert!(matches!(parse_config("{ não é json"), Err(Error::Config(_))));
        assert!(matches!(parse_config(r#"{"mcpServers": {"a": {"args": []}}}"#), Err(Error::Config(_))));
        assert!(matches!(parse_config(r#"{"mcpServers": {"a": {"command": "x", "args": "ls"}}}"#), Err(Error::Config(_))));
    }

    #[test]
    fn config_round_trips_and_is_saved_for_the_owner_only() {
        let d = tempfile::tempdir().unwrap();
        let entries = vec![
            Entry::Stdio(ServerConfig { name: "a".into(), command: "x".into(), args: vec!["1".into()], env: BTreeMap::from([("K".to_string(), "v".to_string())]) }),
            Entry::Remote { name: "r".into(), url: "https://e.com".into() },
        ];
        let path = d.path().join("sub").join("mcp.json");
        save_config(&path, &entries).unwrap();
        assert_eq!(parse_config(&std::fs::read_to_string(&path).unwrap()).unwrap(), entries);
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&path).unwrap().permissions()) & 0o777, 0o600);
    }
}
