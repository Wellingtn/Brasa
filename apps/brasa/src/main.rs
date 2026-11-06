mod assets;
mod block_view;
mod blocks;
mod chrome;
mod complete;
mod files_pane;
mod folders;
mod gsearch;
mod histsearch;
mod icons;
mod keys;
mod note_editor;
mod noterun;
mod pane_view;
mod panes;
mod scrollbar;
mod settings;
mod sidebar;
mod sysmon;
mod syntax;
mod term_view;
mod update;
mod terminal;
mod theme;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    div, prelude::*, px, rgb, size, App, Application, Bounds, ClipboardItem, Context, Div, ElementId, Entity, FocusHandle, Font,
    FontFeatures, FontStyle, FontWeight, Hsla, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ScrollWheelEvent, SharedString, Stateful, TitlebarOptions, Window, WindowBackgroundAppearance, WindowBounds,
    WindowDecorations, WindowOptions,
};
use block_view::{BLOCK_PAD_X, INPUT_H};
use note_editor::{NoteEditor, NoteMode, ED_FONT};
use ox_files::{FileTree, Preview};
use ox_markdown::{Block, NoteStore};
use ox_pty::{PromptConfig, PtyEvents, PtyManager, SessionId, SpawnConfig};
use panes::{Node, PaneKind, Rect};
use ox_suggest::{Context as SuggestContext, Engine, History, SharedHistory, Suggestion};
use terminal::{key_bytes, mouse_report, Match, TermView};
use theme::*;

pub const DEFAULT_FONT: f32 = 15.0;
/// fonte da interface (a mesma do Warp)
pub const UI_FONT: &str = "Roboto";
/// larguras iniciais da barra lateral e do painel de notas (arrastando a borda muda)
pub const SIDEBAR_W: f32 = 280.0;
pub const NOTES_W: f32 = 460.0;
pub const TITLE_H: f32 = 44.0;
/// raio dos cantos da janela
pub const WINDOW_RADIUS: f32 = 12.0;
/// meio período do pisca-pisca do cursor (como nos editores comuns)
pub const CARET_BLINK: std::time::Duration = std::time::Duration::from_millis(530);
/// por quanto tempo o cursor pisca depois da última tecla
pub const CARET_BLINK_FOR: std::time::Duration = std::time::Duration::from_secs(10);
pub const STATUS_H: f32 = 30.0;
pub const MARGIN: f32 = 8.0;
pub const TERM_PAD: f32 = 10.0;
pub const MONO_CANDIDATES: &[&str] = &["Hack", "JetBrains Mono", "JetBrainsMono Nerd Font", "DejaVu Sans Mono", "Fira Mono", "Liberation Mono"];

const WELCOME_NOTE: &str = "# Bem-vindo à Brasa\n\n\
> [!NOTE]\n> Digite `/` numa linha vazia para abrir o menu de blocos (títulos, listas, tarefas, comando...).\n\n\
## Atalhos\n\n\
- Ctrl+Shift+T / W: novo terminal / fechar\n\
- Ctrl+Shift+C / V: copiar / colar\n\
- Ctrl+= / Ctrl+- / Ctrl+0: zoom da fonte\n\
- Ctrl+Shift+F: buscar no histórico\n\
- Ctrl+Shift+E / N: barra lateral / notas\n\
- Ctrl+Shift+D / B: dividir lado a lado / embaixo; Ctrl+Shift+X: tirar o painel da tela\n\
- Ctrl+Shift+M: desempenho (CPU, memória, processos)\n\n\
## Blocos de comando\n\n\
```bash\npwd && ls -la\n```\n\n\
> [!WARNING]\n> Comandos rodam de verdade na sua sessão. Leia antes de executar.\n";

// ---------- PTY -> UI ----------

enum PtyMsg {
    Output(Vec<u8>),
    Exit,
}

/// Quantas leituras do PTY cabem na fila entre o leitor e a tela (e quantas a tela processa por rodada).
const PTY_QUEUE: usize = 1024;

/// Junta leituras seguidas do mesmo terminal numa só, para o terminal processar a saída uma vez por rodada
/// (o PTY entrega pedaços pequenos, e analisar os blocos a cada pedaço custava mais que a própria saída).
fn coalesce_output(batch: Vec<(SessionId, PtyMsg)>) -> Vec<(SessionId, PtyMsg)> {
    let mut out: Vec<(SessionId, PtyMsg)> = Vec::with_capacity(batch.len().min(8));
    for (id, msg) in batch {
        if let (PtyMsg::Output(bytes), Some((last_id, PtyMsg::Output(acc)))) = (&msg, out.last_mut()) {
            if *last_id == id {
                acc.extend_from_slice(bytes);
                continue;
            }
        }
        out.push((id, msg));
    }
    out
}

struct ChannelEvents(async_channel::Sender<(SessionId, PtyMsg)>);

impl PtyEvents for ChannelEvents {
    fn on_output(&self, id: SessionId, bytes: &[u8]) {
        let _ = self.0.send_blocking((id, PtyMsg::Output(bytes.to_vec())));
    }
    fn on_exit(&self, id: SessionId) {
        let _ = self.0.send_blocking((id, PtyMsg::Exit));
    }
}

// ---------- estado ----------

/// Geometria da lista de blocos no último desenho (para rolagem, mouse e busca).
pub struct BlockLayout {
    pub boxes: Vec<blocks::BlockBox>,
    /// primeira linha da grade da saída de cada bloco mostrado
    pub out_starts: Vec<i32>,
    pub total: f32,
    pub view_h: f32,
}

pub struct Tab {
    pub session: SessionId,
    /// pasta da aba Sessões onde o terminal está (None = solto na lista)
    pub folder: Option<u64>,
    /// nome dado pelo usuário (None = mostra o último comando)
    pub name: Option<String>,
    /// área do terminal na janela (dentro do painel, abaixo do cabeçalho dele); atualizada a cada desenho
    pub rect: Rect,
    pub term: TermView,
    pub title: String,
    /// último comando executado (heurística pela linha digitada)
    pub last_cmd: String,
    /// `Some` enquanto a sessão é desenhada em blocos; `None` na grade crua (vim, htop...)
    pub layout: Option<BlockLayout>,
    pub scroll_y: f32,
    /// acompanha o fim da lista enquanto chega saída nova
    pub follow: bool,
    pub hidden: HashSet<i64>,
    pub collapsed: HashSet<i64>,
    pub bookmarks: HashSet<i64>,
    pub selected_block: Option<i64>,
    pub hovered_block: Option<i64>,
    /// instantes dos Enter ainda sem prompt de volta (fila: comandos podem ser enfileirados)
    pub cmd_starts: std::collections::VecDeque<std::time::Instant>,
    /// quantos prompts já foram vistos (para saber quando um comando terminou)
    pub prompts_seen: usize,
    pub durations: std::collections::HashMap<i64, f32>,
    scroll_acc: f32,
    dragging: bool,
    /// botão (já codificado) pressionado enquanto um app TUI recebe o mouse
    mouse_btn: Option<u8>,
    last_cell: Option<(usize, usize)>,
}

pub struct Search {
    pub query: String,
    pub matches: Vec<Match>,
    pub current: usize,
}

/// Borda sendo arrastada para mudar uma largura.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Resize {
    Sidebar,
    Notes,
    /// divisão entre painéis: (número da divisão, lado a lado?, área da divisão)
    Split(usize, bool, Rect),
}

/// Seções da barra lateral, alternadas pelo seletor no topo.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Terminals,
    Notes,
}

/// Cartão de completar caminhos (Tab): opções, a selecionada e o texto antes do cursor quando abriu.
pub struct CompletionMenu {
    pub base: String,
    pub items: Vec<complete::Item>,
    pub sel: usize,
    pub before: String,
    /// rolagem do mouse acumulada
    pub wheel: f32,
}

pub enum PreviewView {
    None,
    Text { name: String, path: PathBuf, content: String, truncated: bool },
    Image { name: String, path: PathBuf },
    Info(String),
}

/// Campo de nome aberto na aba Notas (nova nota / nova pasta).
pub struct Naming {
    pub folder: bool,
    pub text: String,
    /// renomeando esta pasta existente (em vez de criar uma nova)
    pub rename: Option<String>,
    /// pasta da aba Sessões (não do bloco de notas)
    pub sessions: bool,
    /// renomeando um terminal (a sessão), em vez de uma pasta ou nota
    pub terminal: Option<SessionId>,
}

/// Pasta criada pelo usuário na aba Sessões para organizar terminais.
pub struct SessionFolder {
    pub id: u64,
    pub name: String,
    pub open: bool,
}

/// Sessão sendo arrastada para uma pasta.
#[derive(Clone)]
pub struct DraggedSession {
    pub session: SessionId,
    pub label: String,
}

impl DraggedSession {
    pub fn preview(&self, cx: &mut App) -> Entity<DragPreview> {
        let label = self.label.clone();
        cx.new(|_| DragPreview { label, icon: "terminal" })
    }
}

/// Nota ou pasta sendo arrastada na barra lateral.
#[derive(Clone)]
pub struct DraggedNote {
    pub rel: String,
    pub is_dir: bool,
    pub label: String,
}

/// O que aparece embaixo do mouse enquanto arrasta.
pub struct DragPreview {
    label: String,
    icon: &'static str,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py(px(5.))
            .rounded_md()
            .bg(rgb(c_surface()))
            .border_1()
            .border_color(rgb(c_amber()))
            .shadow_lg()
            .font_family(UI_FONT)
            .text_size(px(14.))
            .text_color(rgb(c_text()))
            .child(icons::wicon(self.icon, 14., c_amber()))
            .child(self.label.clone())
    }
}

impl DraggedNote {
    pub fn preview(&self, cx: &mut App) -> Entity<DragPreview> {
        let (label, icon) = (self.label.clone(), if self.is_dir { "folder-closed" } else { "notebook" });
        cx.new(|_| DragPreview { label, icon })
    }
}

pub struct Workspace {
    pub focus: FocusHandle,
    pub mono: SharedString,
    /// fonte com glifos de ícones (Nerd Font), se instalada
    pub icon_font: Option<SharedString>,
    /// texto do prompt do Brasa (`❯`); `None` com `BRASA_PROMPT=native`
    pub prompt_text: Option<String>,
    pub font_size: f32,
    pub cell_w: f32,
    pub cell_h: f32,

    pub pty: Arc<PtyManager>,
    pty_tx: async_channel::Sender<(SessionId, PtyMsg)>,
    pub tabs: Vec<Tab>,
    pub active: usize,

    pub show_sidebar: bool,
    pub show_notes: bool,

    pub shell_cwd: PathBuf,
    pub git: Option<String>,
    pub follow: bool,
    pub tree: FileTree,
    pub selected_path: Option<PathBuf>,
    pub preview: PreviewView,

    pub notes: NoteStore,
    pub note_expanded: HashSet<String>,
    /// pasta selecionada na árvore de notas ("" = raiz)
    pub note_dir: String,
    /// o que a barra lateral mostra: terminais, notas ou arquivos
    pub side: Side,
    /// pastas da aba Sessões, a selecionada (mostra renomear/remover) e o próximo id
    pub session_folders: Vec<SessionFolder>,
    pub session_folder_sel: Option<u64>,
    pub next_folder_id: u64,
    pub side_filter: String,
    pub filter_focused: bool,
    // estado de rolagem das listas (para desenhar as barras)
    pub side_scroll: gpui::ScrollHandle,
    pub note_scroll: gpui::ScrollHandle,
    /// o título da nota (campo acima do conteúdo) está recebendo teclas
    pub title_focused: bool,
    pub title_text: String,
    pub note_current: Option<String>,
    pub note_blocks: Vec<Block>,
    pub note_mode: NoteMode,
    pub editor: Option<NoteEditor>,
    pub focus_editor: bool,
    pub notes_wide: bool,
    /// larguras atuais (ajustáveis arrastando a borda) e a borda sendo arrastada agora
    pub sidebar_px: f32,
    pub notes_px: f32,
    pub resizing: Option<Resize>,
    pub naming: Option<Naming>,
    /// nota/pasta esperando a confirmação para ir para a lixeira
    pub confirm_trash: Option<String>,
    /// aviso curto na seção de notas (ex.: não deu para mover)
    pub side_notice: Option<String>,
    /// imagem aberta no painel (no lugar do editor)
    pub image_view: Option<PathBuf>,
    /// aviso no painel (arquivo binário, grande demais...)
    pub file_notice: Option<String>,
    pub vw: f32,
    pub vh: f32,
    pub ed_cell_w: f32,
    pub ed_cell_h: f32,

    engine: Engine,
    /// histórico de comandos (do shell e do Brasa), usado nas sugestões e na busca do Ctrl+R
    pub history: SharedHistory,
    pub hist: Option<histsearch::HistSearch>,
    /// notas executáveis: pergunta de variáveis, execução em andamento, aviso do resultado e últimos valores
    pub var_prompt: Option<noterun::VarPrompt>,
    pub note_run: Option<noterun::NoteRun>,
    pub run_notice: Option<String>,
    pub var_memory: std::collections::HashMap<String, String>,
    /// blocos de saída abertos na nota (por posição entre os blocos); os outros ficam recolhidos
    pub open_outputs: HashSet<usize>,
    pub line: String,
    pub suggestions: Vec<Suggestion>,
    pub search: Option<Search>,
    pub block_menu: Option<block_view::BlockMenu>,
    /// janela de configurações aberta
    pub show_settings: bool,
    /// aba aberta nas configurações: 0 = Geral, 1 = Aparência
    pub settings_tab: usize,
    pub settings_field: Option<settings::SettingsField>,
    /// arrastando o marcador da roda de cores
    pub wheel_drag: bool,
    pub font_query: String,
    /// texto editável da pré-visualização da fonte (aba Aparência)
    pub preview_text: String,
    /// posição do cursor (em caracteres) no texto da pré-visualização
    pub preview_caret: usize,
    /// arrastando o marcador do tamanho da fonte
    pub size_drag: bool,
    /// resultado da verificação de nova versão
    pub update: update::UpdateState,
    /// passo atual da atualização automática ("Baixando…", "Conferindo…"), escrito pela thread que instala
    pub update_progress: Arc<std::sync::Mutex<String>>,
    pub keymap: keys::Keymap,
    /// ação cujo atalho está sendo gravado nas configurações (próxima tecla vira o atalho)
    pub rebinding: Option<keys::Action>,
    pub keys_notice: Option<String>,
    /// busca global da barra superior
    pub gsearch: String,
    pub gsearch_focused: bool,
    pub gsearch_sel: usize,
    /// medidor de latência do teclado: instante da tecla enviada ao shell, se já chegou eco e as últimas medidas (ms)
    pub key_sent: Option<std::time::Instant>,
    pub echo_seen: bool,
    pub latency_ms: std::collections::VecDeque<f32>,
    /// deslocamento da pré-visualização (colunas, linhas): a "câmera" segue o cursor
    pub preview_scroll: (usize, usize),
    /// largura e altura da área de texto da pré-visualização (f32 em bits), medidas ao desenhar
    pub preview_size: Arc<[std::sync::atomic::AtomicU32; 2]>,
    pub accent_hex: String,
    pub bg_hex: String,
    /// fontes de largura fixa instaladas (calculado na primeira vez que a aba Aparência abre)
    pub mono_fonts: Option<Vec<String>>,
    /// cartão de completar caminhos aberto pelo Tab
    pub completion: Option<CompletionMenu>,
    /// CPU, memória e processos (indicador do cabeçalho e painel de desempenho)
    pub monitor: sysmon::Monitor,
    pub mon: pane_view::MonitorUi,
    /// painéis na tela (terminais e o monitor) e o retângulo de cada um no último desenho
    pub panes: Node,
    pub pane_rects: Vec<(PaneKind, Rect)>,
    /// desenhando o painel em foco (só ele mostra busca e o cartão de completar)
    pub focused_render: bool,
    /// raio dos cantos por dentro da borda da janela (as partes encostadas nos cantos usam)
    pub corner_r: f32,
    /// gerenciador de arquivos em painel (criado ao abrir pela barra de widgets) e o fuso local
    pub files: Option<files_pane::FilesPane>,
    pub tz_offset: i64,
    /// painel de desempenho clicado por último (recebe o contorno de foco); None = o terminal ativo
    pub focused_pane: Option<PaneKind>,
    /// cursor aceso (pisca quando você para de digitar)
    pub caret_on: bool,
    /// pulsos do relógio do cursor (o desempenho é lido a cada dois)
    pub tick: u64,
    pub last_input: std::time::Instant,
}

/// Prompt do shell: por padrão um `❯` roxo (pasta e branch ficam nos chips, como no Warp).
/// Variável de ambiente do app: `BRASA_<nome>`, ou o nome antigo `OXITERM_<nome>` (compatibilidade).
pub fn app_env(name: &str) -> Result<String, std::env::VarError> {
    std::env::var(format!("BRASA_{name}")).or_else(|_| std::env::var(format!("OXITERM_{name}")))
}

/// Pasta de configuração e notas (`~/.config/brasa`). Na primeira vez, traz a pasta do nome antigo
/// (`~/.config/oxiterm`) para cá, com as notas e as pastas de sessão, sem copiar nem apagar nada.
pub fn config_dir() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_default();
    let dir = base.join("brasa");
    let old = base.join("oxiterm");
    if !dir.exists() && old.is_dir() {
        let _ = std::fs::rename(&old, &dir);
    }
    dir
}

/// `BRASA_PROMPT=texto` troca o texto (vazio = sem prompt); `BRASA_PROMPT=native` mantém o do shell.
fn prompt_config() -> Option<PromptConfig> {
    let color = Some((0xb3, 0x92, 0xf0)); // c_accent()
    match app_env("PROMPT") {
        Ok(v) if v == "native" => None,
        Ok(v) => Some(PromptConfig { text: v, color }),
        Err(_) => Some(PromptConfig { text: "❯".into(), color }),
    }
}

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

pub fn shorten(p: &Path) -> String {
    match p.strip_prefix(home()) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn hsla(c: u32) -> Hsla {
    rgb(c).into()
}

/// Botão discreto (texto pequeno, destaca ao passar o mouse).
pub fn small_btn(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .px_2()
        .py(px(2.))
        .text_xs()
        .rounded_sm()
        .cursor_pointer()
        .hover(|s| s.bg(rgb(c_hover())))
        .child(label.into())
}

/// Botão "cinza" dos modelos do Warp: fundo zinc, hover mais claro.
pub fn soft_btn(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .px_3()
        .py_2()
        .text_sm()
        .rounded_md()
        .cursor_pointer()
        .bg(rgb(c_surface()))
        .hover(|s| s.bg(rgb(c_btn_hover())))
        .child(label.into())
}

/// Botão de ação principal (verde). Não reutiliza `small_btn`: o GPUI só aceita um `hover` por elemento.
pub fn primary_btn(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .px_2()
        .py(px(2.))
        .text_xs()
        .rounded_sm()
        .cursor_pointer()
        .bg(rgb(c_run_bg()))
        .hover(|s| s.bg(rgb(c_run_hover())))
        .child(label.into())
}

pub fn measure(window: &Window, family: &SharedString, size: f32) -> (f32, f32) {
    let font = Font {
        family: family.clone(),
        features: FontFeatures::default(),
        fallbacks: None,
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    };
    let ts = window.text_system();
    let id = ts.resolve_font(&font);
    let adv = ts.advance(id, px(size), 'm').map(|s| f32::from(s.width)).unwrap_or(size * 0.6);
    (adv, (size * 1.35).round())
}

fn button_code(b: MouseButton) -> Option<u8> {
    match b {
        MouseButton::Left => Some(0),
        MouseButton::Middle => Some(1),
        MouseButton::Right => Some(2),
        _ => None,
    }
}

/// Bits de modificador do protocolo de mouse (Shift fica de fora: ele força a seleção local).
fn mouse_mods(m: &gpui::Modifiers) -> u8 {
    (m.alt as u8) * 8 + (m.control as u8) * 16
}

impl Workspace {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        // tamanhos em rem (text_xs, px_2, gap_1...) crescem junto: ~12% maiores que o padrão de 16px
        window.set_rem_size(px(18.));

        let fonts = window.text_system().all_font_names();
        let mono: SharedString = MONO_CANDIDATES
            .iter()
            .find(|c| fonts.iter().any(|f| f == *c))
            .copied()
            .unwrap_or("monospace")
            .to_string()
            .into();
        let (saved_font, saved_size) = settings::load();
        let mono: SharedString = saved_font.filter(|f| fonts.iter().any(|x| x == f)).map(Into::into).unwrap_or(mono);
        let start_size = saved_size.unwrap_or(DEFAULT_FONT).clamp(9.0, 32.0);
        let (cell_w, cell_h) = measure(window, &mono, start_size);
        let ed_cell = measure(window, &mono, ED_FONT);

        // fila limitada: se a tela não acompanha, o leitor do PTY espera e o programa (ex.: `cat`) desacelera,
        // em vez de a saída inteira acumular na memória
        let (tx, rx) = async_channel::bounded(PTY_QUEUE);

        // relógio do cursor: pisca a cada CARET_BLINK (fica aceso enquanto você digita) e, de carona,
        // atualiza o "Executando… (Ns)"
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(CARET_BLINK).await;
            let alive = this.update(cx, |ws, cx| {
                ws.tick += 1;
                let idle = ws.last_input.elapsed();
                // pisca enquanto há digitação recente; depois de 10 s parado fica aceso (como no GNOME)
                let blinking = idle >= CARET_BLINK && idle < CARET_BLINK_FOR;
                let caret = if blinking { !ws.caret_on } else { true };
                let mut changed = caret != ws.caret_on;
                ws.caret_on = caret;
                // desempenho a cada dois pulsos (~1 s); processos só com o painel na tela
                if ws.tick % 2 == 0 {
                    let monitor = ws.panes.leaves().iter().any(|k| k.is_widget());
                    let shown = ws.monitor.cpu.round();
                    // processos no intervalo escolhido (padrão 2 s, como o btop) e congelados enquanto o
                    // mouse está na tabela ou o menu de um processo está aberto, para dar tempo de clicar
                    let every = (ws.mon.interval_secs * 2).max(2) as u64;
                    let procs = ws.panes.contains(PaneKind::Procs) && ws.tick % every == 0 && !ws.mon.hovered && ws.mon.menu.is_none();
                    ws.monitor.sample(procs);
                    changed |= monitor || ws.monitor.cpu.round() != shown;
                }
                // tamanhos de pastas que terminaram de ser somados no gerenciador de arquivos
                if let Some(f) = ws.files.as_mut() {
                    changed |= f.apply_sizes();
                }
                // atualização em andamento: redesenha para mostrar o passo atual
                changed |= ws.update == update::UpdateState::Installing;
                // comando rodando: atualiza o contador "Executando… (Ns)" e faz a brasa do logo tremeluzir
                changed |= ws.tabs.iter().any(|t| !t.cmd_starts.is_empty());
                // só redesenha quando algo na tela mudou
                if changed {
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
        })
        .detach();

        // ponte thread-do-PTY -> thread-da-UI; agrupa rajadas numa só atualização
        cx.spawn(async move |this, cx| {
            while let Ok(first) = rx.recv().await {
                let mut batch = vec![first];
                while batch.len() < PTY_QUEUE {
                    match rx.try_recv() {
                        Ok(more) => batch.push(more),
                        Err(_) => break,
                    }
                }
                // sobrou saída na fila: depois desta rodada, cede a vez para a tela desenhar
                let more_pending = !rx.is_empty();
                let batch = coalesce_output(batch);
                let alive = this.update(cx, |ws, cx| {
                    for (id, msg) in batch {
                        ws.on_pty(id, msg);
                    }
                    ws.refresh_shell_state();
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
                if more_pending {
                    cx.background_executor().timer(std::time::Duration::from_millis(1)).await;
                }
            }
        })
        .detach();

        let notes_dir = config_dir().join("notes");
        let notes = NoteStore::new(notes_dir).expect("não foi possível criar a pasta de notas");
        if notes.list().map(|l| l.is_empty()).unwrap_or(false) {
            let _ = notes.write("bem-vindo", WELCOME_NOTE);
        }

        let history: SharedHistory = Arc::new(std::sync::RwLock::new(History::load(&Self::history_log_path())));
        let mut ws = Self {
            focus,
            mono,
            icon_font: icons::find_icon_font(&fonts),
            prompt_text: prompt_config().map(|p| p.text),
            font_size: start_size,
            cell_w,
            cell_h,
            pty: Arc::new(PtyManager::new()),
            pty_tx: tx,
            tabs: Vec::new(),
            active: 0,
            show_sidebar: true,
            show_notes: false,
            shell_cwd: home(),
            git: None,
            follow: true,
            tree: FileTree::new(home(), false),
            selected_path: None,
            preview: PreviewView::None,
            notes,
            note_expanded: HashSet::new(),
            note_dir: String::new(),
            side: Side::Terminals,
            // pastas das Sessões salvas da última vez (vazias: os terminais não sobrevivem ao app)
            session_folders: folders::load().into_iter().enumerate().map(|(i, (name, open))| SessionFolder { id: i as u64 + 1, name, open }).collect(),
            session_folder_sel: None,
            next_folder_id: 0, // ajustado logo abaixo, depois das pastas carregadas
            side_filter: String::new(),
            filter_focused: false,
            side_scroll: gpui::ScrollHandle::new(),
            note_scroll: gpui::ScrollHandle::new(),
            title_focused: false,
            title_text: String::new(),
            note_current: None,
            note_blocks: Vec::new(),
            note_mode: NoteMode::View,
            editor: None,
            focus_editor: false,
            notes_wide: false,
            sidebar_px: SIDEBAR_W,
            notes_px: NOTES_W,
            resizing: None,
            naming: None,
            confirm_trash: None,
            side_notice: None,
            image_view: None,
            file_notice: None,
            vw: 1500.0,
            vh: 900.0,
            ed_cell_w: ed_cell.0,
            ed_cell_h: (ED_FONT * 1.5).round(),
            engine: Engine::with_history(history.clone()),
            history,
            hist: None,
            var_prompt: None,
            note_run: None,
            run_notice: None,
            var_memory: noterun::load_var_memory(),
            open_outputs: HashSet::new(),
            line: String::new(),
            suggestions: Vec::new(),
            search: None,
            block_menu: None,
            show_settings: false,
            settings_tab: 0,
            settings_field: None,
            wheel_drag: false,
            font_query: String::new(),
            preview_text: settings::DEFAULT_PREVIEW.to_string(),
            preview_caret: 0,
            size_drag: false,
            update: update::UpdateState::Unknown,
            update_progress: Arc::new(std::sync::Mutex::new(String::new())),
            keymap: keys::Keymap::load(),
            rebinding: None,
            keys_notice: None,
            gsearch: String::new(),
            gsearch_focused: false,
            gsearch_sel: 0,
            key_sent: None,
            echo_seen: false,
            latency_ms: std::collections::VecDeque::new(),
            preview_scroll: (0, 0),
            preview_size: Arc::new([std::sync::atomic::AtomicU32::new(0), std::sync::atomic::AtomicU32::new(0)]),
            accent_hex: theme::hex_string(theme::custom_accent()),
            bg_hex: theme::hex_string(theme::custom_bg()),
            mono_fonts: None,
            completion: None,
            monitor: sysmon::Monitor::new(),
            mon: pane_view::MonitorUi::default(),
            panes: Node::Leaf(PaneKind::Cpu), // trocado pelo primeiro terminal logo abaixo
            pane_rects: Vec::new(),
            focused_render: true,
            corner_r: WINDOW_RADIUS - 1.0,
            focused_pane: None,
            files: None,
            tz_offset: files_pane::local_offset(),
            caret_on: true,
            tick: 0,
            last_input: std::time::Instant::now(),
        };
        ws.next_folder_id = ws.session_folders.len() as u64 + 1;
        ws.new_tab(None);

        // ganchos de automação (testes visuais): comandos iniciais e nota aberta no começo
        if let Ok(name) = app_env("OPEN_NOTE") {
            ws.open_note(&name);
            ws.show_notes = true;
            ws.set_note_mode(NoteMode::Edit);
        }
        if let Ok(path) = app_env("OPEN_FILE") {
            ws.open_file(PathBuf::from(path));
        }
        if let Ok(cmds) = app_env("STARTUP") {
            let cmds: Vec<String> = cmds.lines().map(str::to_string).filter(|c| !c.trim().is_empty()).collect();
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(std::time::Duration::from_millis(1500)).await;
                for c in cmds {
                    let _ = this.update(cx, |ws, cx| {
                        ws.run_in_terminal(&c);
                        cx.notify();
                    });
                    cx.background_executor().timer(std::time::Duration::from_millis(800)).await;
                }
            })
            .detach();
        }
        if settings::updates_enabled() {
            ws.check_updates(cx);
        }
        ws
    }

    // ----- abas / sessões -----

    /// Novo terminal no lugar do painel em foco (o anterior continua na lista de Sessões).
    pub fn new_tab(&mut self, cwd: Option<PathBuf>) {
        let prev = self.tabs.get(self.active).map(|t| t.session);
        if let Some(sid) = self.spawn_tab(cwd) {
            match prev {
                Some(p) if self.panes.replace(PaneKind::Term(p), PaneKind::Term(sid)) => {}
                _ => self.panes = Node::Leaf(PaneKind::Term(sid)),
            }
            self.focus_session(sid);
        }
    }

    /// Divide o painel em foco e abre um terminal novo ao lado (`row`) ou embaixo.
    pub fn split_new(&mut self, row: bool) {
        let old = self.session();
        let cwd = self.pty.cwd(old).or_else(|| Some(self.shell_cwd.clone()));
        if let Some(sid) = self.spawn_tab(cwd) {
            self.panes.split(PaneKind::Term(old), PaneKind::Term(sid), row);
            self.focus_session(sid);
        }
    }

    /// Abre/fecha um painel de desempenho (CPU, memória, processos). O primeiro abre numa coluna
    /// inteira à direita; os seguintes se empilham embaixo do último dessa coluna.
    pub fn toggle_widget(&mut self, kind: PaneKind) {
        if !self.panes.remove(kind) {
            let last_widget = self.panes.leaves().into_iter().filter(|k| k.is_widget()).last();
            match last_widget {
                Some(w) => {
                    self.panes.split(w, kind, false);
                }
                None => {
                    let old = std::mem::replace(&mut self.panes, Node::Leaf(kind));
                    self.panes = Node::Split { row: true, ratio: 0.62, a: Box::new(old), b: Box::new(Node::Leaf(kind)) };
                }
            }
            self.monitor.sample(self.panes.contains(PaneKind::Procs));
            if kind == PaneKind::Files {
                // abre na pasta do shell em foco; recarrega se já existia
                match self.files.as_mut() {
                    Some(f) => f.reload(),
                    None => self.files = Some(files_pane::FilesPane::new(self.shell_cwd.clone())),
                }
            }
            self.focused_pane = Some(kind);
        } else if self.focused_pane == Some(kind) {
            self.focused_pane = None;
        }
        self.ensure_layout();
    }

    /// Tira um painel da tela. O terminal continua aberto na lista de Sessões.
    pub fn close_pane(&mut self, kind: PaneKind) {
        self.panes.remove(kind);
        self.ensure_layout();
    }

    fn focus_session(&mut self, sid: SessionId) {
        if let Some(i) = self.tabs.iter().position(|t| t.session == sid) {
            self.active = i;
            self.line.clear();
            self.suggestions.clear();
            self.completion = None;
        }
    }

    /// Mantém o layout coerente: só sessões vivas, pelo menos um terminal e o foco num painel visível.
    pub fn ensure_layout(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        for leaf in self.panes.leaves() {
            if let PaneKind::Term(s) = leaf {
                if !self.tabs.iter().any(|t| t.session == s) && !self.panes.remove(leaf) {
                    self.panes = Node::Leaf(PaneKind::Term(self.tabs[self.active.min(self.tabs.len() - 1)].session));
                }
            }
        }
        let has_term = self.panes.leaves().iter().any(|l| matches!(l, PaneKind::Term(_)));
        if !has_term {
            let sid = self.tabs[self.active.min(self.tabs.len() - 1)].session;
            let widgets = std::mem::replace(&mut self.panes, Node::Leaf(PaneKind::Term(sid)));
            self.panes = Node::Split { row: true, ratio: 0.62, a: Box::new(Node::Leaf(PaneKind::Term(sid))), b: Box::new(widgets) };
        }
        self.active = self.active.min(self.tabs.len() - 1);
        if self.focused_pane.is_some_and(|k| !self.panes.contains(k)) {
            self.focused_pane = None;
        }
        if !self.panes.contains(PaneKind::Term(self.session())) {
            if let Some(PaneKind::Term(s)) = self.panes.leaves().into_iter().find(|l| matches!(l, PaneKind::Term(_))) {
                self.focus_session(s);
            }
        }
    }

    /// Índice da sessão cujo painel está sob o ponto (coordenadas da janela).
    pub fn tab_at(&self, pos: gpui::Point<gpui::Pixels>) -> Option<usize> {
        let (x, y) = (f32::from(pos.x), f32::from(pos.y));
        self.pane_rects.iter().find_map(|(k, r)| match k {
            PaneKind::Term(s) if r.contains(x, y) => self.tabs.iter().position(|t| t.session == *s),
            _ => None,
        })
    }

    /// Cria a sessão sem mexer no layout nem no foco.
    fn spawn_tab(&mut self, cwd: Option<PathBuf>) -> Option<SessionId> {
        let (cols, rows) = self.tabs.get(self.active).map_or((100, 30), |t| (t.term.cols, t.term.rows));
        let cfg = SpawnConfig { cols: cols as u16, rows: rows as u16, cwd, prompt: prompt_config(), ..Default::default() };
        if let Ok(session) = self.pty.spawn(cfg, Arc::new(ChannelEvents(self.pty_tx.clone()))) {
            self.tabs.push(Tab {
                session,
                folder: None,
                name: None,
                rect: Rect::default(),
                term: TermView::new(cols, rows),
                title: String::new(),
                last_cmd: String::new(),
                layout: None,
                scroll_y: 0.0,
                follow: true,
                hidden: HashSet::new(),
                collapsed: HashSet::new(),
                bookmarks: HashSet::new(),
                selected_block: None,
                hovered_block: None,
                cmd_starts: std::collections::VecDeque::new(),
                prompts_seen: 0,
                durations: std::collections::HashMap::new(),
                scroll_acc: 0.0,
                dragging: false,
                mouse_btn: None,
                last_cell: None,
            });
            return Some(session);
        }
        None
    }

    pub fn close_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(idx);
        let _ = self.pty.kill(tab.session);
        if self.tabs.is_empty() {
            self.panes = Node::Leaf(PaneKind::Cpu); // trocado pelo novo terminal logo abaixo
            self.new_tab(None);
        } else if self.active >= self.tabs.len() || idx < self.active {
            self.active = self.active.saturating_sub(1).min(self.tabs.len() - 1);
        }
        self.ensure_layout();
        self.line.clear();
        self.suggestions.clear();
        self.refresh_shell_state();
    }

    /// Foca a sessão; se ela não está na tela, aparece no lugar do painel em foco.
    pub fn switch_tab(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            let sid = self.tabs[idx].session;
            if !self.panes.contains(PaneKind::Term(sid)) {
                self.panes.replace(PaneKind::Term(self.session()), PaneKind::Term(sid));
            }
            self.active = idx;
            self.line.clear();
            self.suggestions.clear();
            self.refresh_shell_state();
        }
    }

    pub fn tab(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    pub fn session(&self) -> SessionId {
        self.tabs[self.active].session
    }

    fn on_pty(&mut self, id: SessionId, msg: PtyMsg) {
        let Some(idx) = self.tabs.iter().position(|t| t.session == id) else { return };
        match msg {
            PtyMsg::Output(bytes) => {
                if self.key_sent.is_some() && idx == self.active {
                    self.echo_seen = true;
                }
                let tab = &mut self.tabs[idx];
                for reply in tab.term.feed(&bytes) {
                    let _ = self.pty.write(id, reply.as_bytes());
                }
                if let Some(t) = tab.term.take_title() {
                    tab.title = t;
                }
                // duração: cada prompt novo encerra o comando anterior (medido do Enter até aqui)
                let mut finished: Vec<(String, Option<i32>, String)> = Vec::new();
                if let Some(p) = &self.prompt_text {
                    let model = tab.term.blocks(p);
                    let n = model.blocks.len();
                    for k in tab.prompts_seen.max(1)..n {
                        if let Some(t0) = tab.cmd_starts.pop_front() {
                            tab.durations.insert(model.blocks[k - 1].id, t0.elapsed().as_secs_f32());
                        }
                        // comando que terminou, com o código de saída e a saída (para as notas executáveis)
                        let b = &model.blocks[k - 1];
                        finished.push((b.command.clone(), b.status, tab.term.lines_text(b.out_start, b.out_end).trim_end().to_string()));
                    }
                    tab.prompts_seen = tab.prompts_seen.max(n);
                }
                for (command, status, output) in finished {
                    self.note_command_finished(id, &command, status, &output);
                }
            }
            PtyMsg::Exit => self.close_tab(idx),
        }
    }

    /// Lê a pasta real do shell ativo (e o branch git); a árvore de arquivos acompanha se `follow`.
    fn refresh_shell_state(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        if let Some(cwd) = self.pty.cwd(tab.session) {
            if cwd != self.shell_cwd {
                self.git = ox_files::git_branch(&cwd);
                if self.follow {
                    self.tree.set_root(&cwd);
                }
                self.shell_cwd = cwd;
            }
        }
    }

    pub fn sidebar_w(&self) -> f32 {
        if self.show_sidebar {
            self.sidebar_px
        } else {
            0.0
        }
    }

    /// Ajusta a grade do terminal ao espaço disponível na janela.
    /// Calcula o retângulo de cada painel e ajusta a grade de cada terminal visível ao seu espaço.
    fn sync_size(&mut self, window: &Window) {
        let vp = window.viewport_size();
        self.vw = f32::from(vp.width);
        let vh = f32::from(vp.height);
        self.vh = vh;
        self.ensure_layout();
        // área principal: à direita da barra lateral e das notas, abaixo do cabeçalho (1 px de borda)
        let x = 1.0 + self.sidebar_w() + self.notes_w();
        // a barra de widgets ocupa a ponta direita
        let main = Rect { x, y: 1.0 + TITLE_H, w: (self.vw - x - 1.0 - pane_view::WIDGET_BAR_W).max(80.0), h: (vh - TITLE_H - 2.0).max(80.0) };
        let multi = self.panes.leaves().len() > 1;
        let area = if multi { pane_view::inset(main, pane_view::PANE_GAP) } else { main };
        self.pane_rects = self.panes.rects(area, if multi { pane_view::PANE_GAP } else { 0.0 });
        for (kind, r) in self.pane_rects.clone() {
            let PaneKind::Term(sid) = kind else { continue };
            let Some(idx) = self.tabs.iter().position(|t| t.session == sid) else { continue };
            let content = if multi { pane_view::content_rect(r) } else { r };
            self.tabs[idx].rect = content;
            let (cols, rows) = if self.tabs[idx].layout.is_some() {
                // modo em blocos: lista rolável + barra de entrada fixa embaixo
                let list_h = (content.h - INPUT_H).max(60.0);
                let w = content.w - BLOCK_PAD_X * 2.0;
                ((w / self.cell_w).floor().max(10.0) as usize, (list_h / self.cell_h).floor().max(3.0) as usize)
            } else {
                let w = content.w - MARGIN * 2.0 - TERM_PAD * 2.0;
                let h = content.h - MARGIN - STATUS_H - TERM_PAD * 2.0;
                ((w / self.cell_w).floor().max(10.0) as usize, (h / self.cell_h).floor().max(3.0) as usize)
            };
            let t = &mut self.tabs[idx];
            if (cols, rows) != (t.term.cols, t.term.rows) {
                t.term.resize(cols, rows);
                let _ = self.pty.resize(sid, cols as u16, rows as u16);
            }
        }
    }

    /// Executa a ação de um atalho.
    fn run_action(&mut self, action: keys::Action, window: &mut Window, cx: &mut Context<Self>) {
        use keys::Action::*;
        match action {
            Search => {
                self.focus_editor = false;
                self.search = Some(self::Search { query: String::new(), matches: Vec::new(), current: 0 });
            }
            Copy => self.copy_selection(cx),
            Paste => self.paste_clipboard(cx),
            NewTab => self.new_tab(Some(self.shell_cwd.clone())),
            CloseTab => self.close_tab(self.active),
            PrevTab => self.switch_tab((self.active + self.tabs.len() - 1) % self.tabs.len()),
            NextTab => self.switch_tab((self.active + 1) % self.tabs.len()),
            ToggleSidebar => self.show_sidebar = !self.show_sidebar,
            ToggleNotes => self.show_notes = !self.show_notes,
            SplitRight => self.split_new(true),
            SplitDown => self.split_new(false),
            ClosePane => self.close_pane(PaneKind::Term(self.session())),
            Performance => self.toggle_widget(PaneKind::Procs),
            ZoomIn => self.set_font_size(self.font_size + 1.0, window),
            ZoomOut => self.set_font_size(self.font_size - 1.0, window),
            ZoomReset => self.set_font_size(DEFAULT_FONT, window),
            Settings => self.show_settings = !self.show_settings,
            HistorySearch => {
                if self.hist.is_some() {
                    self.hist = None;
                } else {
                    self.open_hist_search();
                }
            }
            GlobalSearch => {
                self.gsearch_focused = true;
                self.filter_focused = false;
                self.focus_editor = false;
            }
        }
        cx.notify();
    }

    pub fn set_font_size(&mut self, size: f32, window: &Window) {
        self.font_size = size.clamp(9.0, 32.0);
        (self.cell_w, self.cell_h) = measure(window, &self.mono, self.font_size);
        settings::save(&self.mono, self.font_size);
    }

    pub fn set_mono(&mut self, family: &str, window: &Window) {
        self.mono = family.to_string().into();
        (self.cell_w, self.cell_h) = measure(window, &self.mono, self.font_size);
        self.ed_cell_w = measure(window, &self.mono, ED_FONT).0;
        settings::save(&self.mono, self.font_size);
    }

    // ----- entrada do terminal -----

    /// "Executar no Terminal": injeta o comando na sessão ativa.
    pub fn run_in_terminal(&mut self, command: &str) {
        self.tab().cmd_starts.push_back(std::time::Instant::now());
        self.tab().follow = true;
        self.tab().last_cmd = command.lines().next().unwrap_or("").trim().to_string();
        let _ = self.pty.run_command(self.session(), command);
        self.line.clear();
        self.suggestions.clear();
    }

    pub fn copy_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.tab().term.selection_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub fn paste_clipboard(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
            let bracketed = self.tab().term.mode().contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE);
            let payload = if bracketed {
                format!("\x1b[200~{text}\x1b[201~")
            } else {
                text.replace("\r\n", "\r").replace('\n', "\r")
            };
            let _ = self.pty.write(self.session(), payload.as_bytes());
            self.line.push_str(&text);
            self.refresh_suggestions();
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.caret_on = true;
        self.last_input = std::time::Instant::now();
        let k = &ev.keystroke;
        let m = &k.modifiers;
        let key = k.key.to_lowercase();

        // com a página de configurações aberta, as teclas são dela, a menos que se esteja digitando na barra lateral
        let typing_in_sidebar = self.naming.is_some() || self.filter_focused || self.title_focused;
        if self.show_settings && !typing_in_sidebar && self.on_settings_key(k, cx) {
            return;
        }

        if self.var_prompt.is_some() && self.on_var_key(k, cx) {
            return;
        }

        if self.hist.is_some() && self.on_hist_key(k, cx) {
            return;
        }

        if self.gsearch_focused && self.on_gsearch_key(k, cx) {
            return;
        }

        if self.naming.is_some() {
            self.on_naming_key(k, cx);
            return;
        }

        if self.confirm_trash.is_some() && key == "escape" {
            self.confirm_trash = None;
            cx.notify();
            return;
        }

        if self.filter_focused && self.on_filter_key(k, cx) {
            return;
        }

        if self.mon.filter_focused && self.on_monitor_key(k, cx) {
            return;
        }

        // gerenciador de arquivos em foco: setas, Enter, Backspace
        // (o editor do painel de notas tem prioridade quando está com o foco)
        let editing = self.focus_editor && self.show_notes && self.note_mode != NoteMode::View;
        if self.focused_pane == Some(PaneKind::Files) && !editing && !(m.control && m.shift) && self.on_files_key(k, cx) {
            return;
        }

        if self.title_focused && self.on_title_key(k, cx) {
            return;
        }

        if self.focus_editor && self.show_notes && self.note_mode != NoteMode::View && self.on_editor_key(k, cx) {
            return;
        }

        // Ctrl+R abre a busca no histórico, menos num programa de tela cheia (vim, htop), onde a tecla é dele
        let action = self.keymap.lookup(k).filter(|a| *a != keys::Action::HistorySearch || !self.tabs[self.active].term.is_alt_screen());
        if self.search.is_some() && action != Some(keys::Action::Search) {
            self.on_search_key(k, cx);
            return;
        }

        if let Some(action) = action {
            self.run_action(action, window, cx);
            return;
        }
        // Ctrl+Shift+tecla sem ação não vai para o terminal
        if m.control && m.shift {
            return;
        }

        // cartão de completar: setas escolhem, Enter/Tab completam, Esc fecha
        if self.completion.is_some() && self.on_completion_key(k, cx) {
            return;
        }
        if key == "tab" && !m.shift && !m.control && !m.alt && self.try_complete(cx) {
            return;
        }

        // aceita a sugestão com → (aproximação do zsh-autosuggestions)
        if (key == "right" && !m.control && !m.alt) || (key == "tab" && m.shift) {
            if let Some(s) = self.suggestions.first() {
                let ghost = s.ghost(&self.line).to_string();
                if !ghost.is_empty() {
                    let _ = self.pty.write(self.session(), ghost.as_bytes());
                    self.line.push_str(&ghost);
                    self.refresh_suggestions();
                    cx.notify();
                    return;
                }
            }
        }

        let mode = self.tab().term.mode();
        let Some(bytes) = key_bytes(k, mode) else { return };
        if key == "enter" {
            self.tab().cmd_starts.push_back(std::time::Instant::now());
            if !self.line.trim().is_empty() {
                let cmd = self.line.trim().to_string();
                self.record_command(&self.line.clone());
                self.tab().last_cmd = cmd;
            }
        }
        self.track_line(k);
        let tab = self.tab();
        tab.term.clear_selection();
        tab.term.scroll_to_bottom();
        tab.follow = true;
        // latência: só uma medida por vez; uma tecla sem eco há mais de 1 s é esquecida
        if self.key_sent.map_or(true, |t| t.elapsed() > std::time::Duration::from_secs(1)) {
            self.key_sent = Some(std::time::Instant::now());
            self.echo_seen = false;
        }
        let _ = self.pty.write(self.session(), &bytes);
        self.refresh_suggestions();
        cx.notify();
    }

    /// Chamado a cada desenho: se o eco da última tecla já chegou, registra o tempo desde a tecla.
    fn note_latency(&mut self) {
        if let (Some(t), true) = (self.key_sent, self.echo_seen) {
            let ms = t.elapsed().as_secs_f32() * 1000.0;
            if ms < 500.0 {
                if self.latency_ms.len() >= 200 {
                    self.latency_ms.pop_front();
                }
                self.latency_ms.push_back(ms);
            }
            self.key_sent = None;
            self.echo_seen = false;
        }
    }

    /// Tab no modo em blocos: abre o cartão com as pastas/arquivos que combinam com a palavra sob o
    /// cursor. Uma opção só completa direto; nenhuma (ou o nome do comando) deixa o Tab com o shell.
    fn try_complete(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(prompt) = self.prompt_text.clone() else { return false };
        let tab = &self.tabs[self.active];
        if tab.layout.is_none() || tab.term.is_alt_screen() {
            return false;
        }
        let model = tab.term.blocks(&prompt);
        if !model.idle {
            return false;
        }
        let before = tab.term.input_view(&model, &prompt).before;
        let Some(c) = complete::complete(&before, &self.shell_cwd, &home()) else { return false };
        match c.items.len() {
            0 => false,
            1 => {
                self.accept_completion(&c.base, &c.items[0], &before);
                cx.notify();
                true
            }
            _ => {
                self.completion = Some(CompletionMenu { base: c.base, items: c.items, sel: 0, before, wheel: 0.0 });
                cx.notify();
                true
            }
        }
    }

    pub fn accept_completion(&mut self, base: &str, item: &complete::Item, before: &str) {
        let bytes = complete::accept_bytes(base, item);
        let _ = self.pty.write(self.session(), &bytes);
        // a linha rastreada passa a ser o texto completado (para as sugestões continuarem certas)
        let kept: String = before.chars().take(before.chars().count() - base.chars().count()).collect();
        self.line = format!("{kept}{}{}", complete::shell_escape(&item.name), if item.is_dir { "/" } else { " " });
        self.completion = None;
        self.refresh_suggestions();
    }

    /// Teclas com o cartão de completar aberto. Devolve `false` para as que seguem para o terminal
    /// (letras, apagar...): o cartão fecha e a tecla vai para o shell normalmente.
    fn on_completion_key(&mut self, k: &gpui::Keystroke, cx: &mut Context<Self>) -> bool {
        let key = k.key.to_lowercase();
        let Some(menu) = self.completion.as_mut() else { return false };
        let n = menu.items.len();
        match key.as_str() {
            "down" => menu.sel = (menu.sel + 1) % n,
            "tab" if !k.modifiers.shift => menu.sel = (menu.sel + 1) % n,
            "up" => menu.sel = (menu.sel + n - 1) % n,
            "tab" => menu.sel = (menu.sel + n - 1) % n,
            "escape" => self.completion = None,
            "enter" | "right" => {
                let menu = self.completion.take().expect("cartão aberto");
                self.accept_completion(&menu.base, &menu.items[menu.sel], &menu.before);
            }
            _ => {
                self.completion = None;
                return false;
            }
        }
        cx.notify();
        true
    }

    /// Teclas enquanto a barra de busca está aberta.
    fn on_search_key(&mut self, k: &gpui::Keystroke, cx: &mut Context<Self>) {
        let m = &k.modifiers;
        let key = k.key.to_lowercase();
        match key.as_str() {
            "escape" => self.search = None,
            "f" if m.control && m.shift => self.search = None,
            "enter" => {
                let n = self.search.as_ref().map_or(0, |s| s.matches.len());
                if n > 0 {
                    let s = self.search.as_mut().unwrap();
                    // Enter sobe (ocorrência mais antiga); Shift+Enter desce
                    s.current = if m.shift { (s.current + 1) % n } else { (s.current + n - 1) % n };
                    let line = s.matches[s.current].line;
                    self.scroll_to_match(line);
                }
            }
            "backspace" => {
                if let Some(s) = self.search.as_mut() {
                    s.query.pop();
                }
                self.run_search();
            }
            "space" => {
                if let Some(s) = self.search.as_mut() {
                    s.query.push(' ');
                }
                self.run_search();
            }
            _ if !m.control && !m.alt => {
                if let (Some(c), Some(s)) = (&k.key_char, self.search.as_mut()) {
                    s.query.push_str(c);
                    self.run_search();
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Recalcula as ocorrências e salta para a mais recente.
    fn run_search(&mut self) {
        let Some(query) = self.search.as_ref().map(|s| s.query.clone()) else { return };
        let matches = self.tabs[self.active].term.search(&query);
        let current = matches.len().saturating_sub(1);
        if let Some(m) = matches.get(current) {
            let line = m.line;
            self.scroll_to_match(line);
        }
        if let Some(s) = self.search.as_mut() {
            s.matches = matches;
            s.current = current;
        }
    }

    /// Mantém uma cópia aproximada da linha digitada (sem integração com o shell, só heurística).
    fn track_line(&mut self, k: &gpui::Keystroke) {
        let m = &k.modifiers;
        match k.key.as_str() {
            "backspace" => {
                self.line.pop();
            }
            "enter" | "escape" | "tab" | "up" | "down" | "left" | "right" | "home" | "end" | "delete" => self.line.clear(),
            "space" => self.line.push(' '),
            _ if m.control => {
                if k.key == "w" {
                    let trimmed = self.line.trim_end().len();
                    self.line.truncate(self.line[..trimmed].rfind(' ').map_or(0, |i| i + 1));
                } else {
                    self.line.clear();
                }
            }
            _ if !m.alt => {
                if let Some(c) = &k.key_char {
                    self.line.push_str(c);
                }
            }
            _ => {}
        }
    }

    fn refresh_suggestions(&mut self) {
        let ctx = SuggestContext { input: &self.line, cwd: &self.shell_cwd };
        self.suggestions = self.engine.suggest(&ctx, 4);
    }

    // ----- mouse no terminal -----

    /// A roda rola o painel que está sob o mouse, sem mudar o foco do teclado.
    fn on_scroll(&mut self, ev: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let focused = self.active;
        if let Some(i) = self.tab_at(ev.position) {
            self.active = i;
        }
        self.on_scroll_here(ev, cx);
        self.active = focused;
    }

    fn on_scroll_here(&mut self, ev: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let cell_h = self.cell_h;
        // lista de blocos: a roda move a rolagem em pixels (positivo = para cima)
        if !self.tab().term.mouse_mode() {
            if let Some(lay) = self.tab().layout.as_ref() {
                let max = blocks::max_scroll(lay.total, lay.view_h);
                let dy = f32::from(ev.delta.pixel_delta(px(cell_h)).y);
                let tab = self.tab();
                tab.scroll_y = (tab.scroll_y - dy).clamp(0.0, max);
                tab.follow = tab.scroll_y >= max - 1.0;
                cx.notify();
                return;
            }
        }
        let dy = f32::from(ev.delta.pixel_delta(px(cell_h)).y);
        let tab = self.tab();
        tab.scroll_acc += dy / cell_h;
        let lines = tab.scroll_acc.trunc() as i32;
        if lines == 0 {
            return;
        }
        tab.scroll_acc -= lines as f32;
        let mode = tab.term.mode();

        use alacritty_terminal::term::TermMode as M;
        if tab.term.mouse_mode() && !ev.modifiers.shift {
            // roda do mouse vira botão 64 (acima) / 65 (abaixo)
            let code = if lines > 0 { 64 } else { 65 } + mouse_mods(&ev.modifiers);
            for _ in 0..lines.abs() {
                self.report_mouse(code, ev.position, false);
            }
        } else if mode.contains(M::ALT_SCREEN) && mode.contains(M::ALTERNATE_SCROLL) {
            // less/man/vim sem mouse: a roda vira setas
            let app = mode.contains(M::APP_CURSOR);
            let arrow: &[u8] = match (lines > 0, app) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            for _ in 0..lines.abs() {
                let _ = self.pty.write(self.session(), arrow);
            }
        } else {
            self.tab().term.scroll(lines);
        }
        cx.notify();
    }

    /// Posição do mouse (coordenadas da janela) -> (coluna, linha, metade direita da célula).
    fn cell_at(&self, pos: gpui::Point<gpui::Pixels>) -> (usize, usize, bool) {
        let r = self.tabs[self.active].rect;
        let (x0, y0) = (r.x + MARGIN + TERM_PAD, r.y + MARGIN + TERM_PAD);
        let fx = ((f32::from(pos.x) - x0) / self.cell_w).max(0.0);
        let fy = ((f32::from(pos.y) - y0) / self.cell_h).max(0.0);
        (fx.floor() as usize, fy.floor() as usize, fx.fract() >= 0.5)
    }

    /// Envia um evento de mouse ao app que está no terminal.
    fn report_mouse(&mut self, code: u8, pos: gpui::Point<gpui::Pixels>, release: bool) {
        let (c, r, _) = self.cell_at(pos);
        let tab = self.tab();
        let (c, r) = (c.min(tab.term.cols - 1), r.min(tab.term.rows - 1));
        if let Some(bytes) = mouse_report(tab.term.mode(), code, c, r, release) {
            let _ = self.pty.write(self.session(), &bytes);
        }
    }

    fn on_term_down(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        // clicar num painel põe o foco (teclado) nele
        self.focused_pane = None;
        if let Some(i) = self.tab_at(ev.position) {
            if i != self.active {
                let sid = self.tabs[i].session;
                self.focus_session(sid);
                self.refresh_shell_state();
            }
        }
        // app TUI com mouse ativo recebe o clique; Shift força a seleção do terminal
        if self.tab().term.mouse_mode() && !ev.modifiers.shift {
            if let Some(b) = button_code(ev.button) {
                let code = b + mouse_mods(&ev.modifiers);
                let tab = self.tab();
                tab.mouse_btn = Some(code);
                tab.last_cell = None;
                self.report_mouse(code, ev.position, false);
            }
            return;
        }
        if self.tab().layout.is_some() {
            match ev.button {
                MouseButton::Left => match self.block_point(ev.position) {
                    Some((line, col, right)) => {
                        let tab = self.tab();
                        tab.term.start_selection_at(line, col, right, ev.click_count);
                        tab.dragging = true;
                    }
                    None => self.tab().term.clear_selection(),
                },
                MouseButton::Right => self.paste_clipboard(cx),
                _ => {}
            }
            cx.notify();
            return;
        }
        match ev.button {
            MouseButton::Left => {
                let (c, r, right) = self.cell_at(ev.position);
                let tab = self.tab();
                tab.term.start_selection(c, r, right, ev.click_count);
                tab.dragging = true;
            }
            MouseButton::Right => self.paste_clipboard(cx),
            _ => {}
        }
        cx.notify();
    }

    fn on_term_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        // só o painel em foco reage ao movimento (a seleção continua mesmo saindo dele)
        if !self.tabs[self.active].dragging && self.tab_at(ev.position).is_some_and(|i| i != self.active) {
            return;
        }
        if self.tab().layout.is_some() && !self.tab().term.mouse_mode() {
            if ev.pressed_button == Some(MouseButton::Left) && self.tab().dragging {
                if let Some((line, col, right)) = self.block_point(ev.position) {
                    self.tab().term.update_selection_at(line, col, right);
                    cx.notify();
                }
            }
            return;
        }
        let mode = self.tab().term.mode();
        if self.tab().term.mouse_mode() && !ev.modifiers.shift {
            use alacritty_terminal::term::TermMode as M;
            let cell = {
                let (c, r, _) = self.cell_at(ev.position);
                (c, r)
            };
            if self.tab().last_cell == Some(cell) {
                return;
            }
            let mods = mouse_mods(&ev.modifiers);
            let code = match ev.pressed_button.and_then(button_code) {
                Some(b) if mode.intersects(M::MOUSE_DRAG | M::MOUSE_MOTION) => Some(b + 32 + mods),
                None if mode.contains(M::MOUSE_MOTION) => Some(35 + mods), // movimento sem botão
                _ => None,
            };
            if let Some(code) = code {
                self.tab().last_cell = Some(cell);
                self.report_mouse(code, ev.position, false);
            }
            return;
        }
        if ev.pressed_button == Some(MouseButton::Left) && self.tab().dragging {
            let (c, r, right) = self.cell_at(ev.position);
            self.tab().term.update_selection(c, r, right);
            cx.notify();
        }
    }

    fn on_term_up(&mut self, ev: &MouseUpEvent, cx: &mut Context<Self>) {
        if let Some(code) = self.tab().mouse_btn.take() {
            self.report_mouse(code, ev.position, true);
            return;
        }
        self.tab().dragging = false;
        cx.notify();
    }

    // ----- blocos -----

    /// (linha da grade, coluna, metade direita) sob o mouse na lista de blocos.
    fn block_point(&self, pos: gpui::Point<gpui::Pixels>) -> Option<(i32, usize, bool)> {
        let tab = &self.tabs[self.active];
        let lay = tab.layout.as_ref()?;
        let m = self.block_metrics();
        let y = f32::from(pos.y) - tab.rect.y + tab.scroll_y;
        let left = tab.rect.x + BLOCK_PAD_X;
        let fx = ((f32::from(pos.x) - left) / self.cell_w).max(0.0);
        match blocks::hit(&lay.boxes, y, &m) {
            blocks::Hit::Row(i, k) => Some((lay.out_starts[i] + k as i32, fx.floor() as usize, fx.fract() >= 0.5)),
            _ => None,
        }
    }

    /// Rola até uma ocorrência da busca (linha da grade).
    fn scroll_to_match(&mut self, line: i32) {
        let m = self.block_metrics();
        let tab = self.tab();
        if let Some(lay) = tab.layout.as_ref() {
            for (i, start) in lay.out_starts.iter().enumerate() {
                let rows = lay.boxes[i].rows as i32;
                if line >= *start && line < start + rows {
                    tab.scroll_y = blocks::scroll_to_row(&lay.boxes, i, (line - start) as usize, lay.view_h, lay.total, &m);
                    tab.follow = false;
                    return;
                }
            }
            return; // fora das saídas visíveis (p.ex. no texto do comando): mantém a rolagem
        }
        tab.term.scroll_to_line(line);
    }

    pub fn find_block(&self, id: i64) -> Option<terminal::BlockInfo> {
        let prompt = self.prompt_text.clone()?;
        self.tabs[self.active].term.blocks(&prompt).blocks.into_iter().find(|b| b.id == id)
    }

    /// Texto da saída de um bloco (sem linhas em branco no fim).
    pub fn block_output(&self, b: &terminal::BlockInfo) -> String {
        let term = &self.tabs[self.active].term;
        let end = term.last_content_line(b.out_start, b.out_end);
        term.lines_text(b.out_start, end + 1)
    }

    pub fn block_copy(&mut self, id: i64, cx: &mut Context<Self>) {
        if let Some(b) = self.find_block(id) {
            cx.write_to_clipboard(ClipboardItem::new_string(self.block_output(&b)));
        }
    }

    /// "Criar nota a partir do bloco": comando + saída num arquivo novo, aberto no editor.
    pub fn block_to_note(&mut self, id: i64) {
        let Some(b) = self.find_block(id) else { return };
        let output = self.block_output(&b).replace("```", "'''");
        let slug: String = b.command.chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
        let slug: String = slug.trim_matches('-').chars().take(40).collect();
        let base = if slug.is_empty() { "comando".to_string() } else { slug };
        let (mut name, mut n) = (base.clone(), 1);
        while self.notes.read(&name).is_ok() {
            n += 1;
            name = format!("{base}-{n}");
        }
        let body = format!("```bash\n{}\n```\n\nSaída em `{}`:\n\n```text\n{}\n```\n", b.command, b.info.path, output);
        if self.notes.write(&name, &body).is_ok() {
            self.open_note(&format!("{name}.md"));
            self.show_notes = true;
            self.set_note_mode(NoteMode::Edit);
        }
    }

    // ----- arquivos -----

    /// Pasta alvo do "cd aqui": a pasta selecionada, ou a raiz da árvore.
    pub fn target_dir(&self) -> PathBuf {
        match &self.selected_path {
            Some(p) if p.is_dir() => p.clone(),
            _ => self.tree.root().to_path_buf(),
        }
    }

    pub fn select_file(&mut self, path: PathBuf) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.preview = match ox_files::preview(&path) {
            Ok(Preview::Text { content, truncated, .. }) => PreviewView::Text { name, path: path.clone(), content, truncated },
            Ok(Preview::Image { .. }) => PreviewView::Image { name, path: path.clone() },
            Ok(Preview::Binary { size }) => PreviewView::Info(format!("{name}: binário, {size} bytes")),
            Ok(Preview::TooLarge { size }) => PreviewView::Info(format!("{name}: grande demais ({size} bytes)")),
            Err(e) => PreviewView::Info(format!("{name}: {e}")),
        };
        self.selected_path = Some(path);
    }

    // ----- notas -----

    pub fn open_note(&mut self, rel: &str) {
        self.save_note();
        self.open_outputs.clear();
        if self.note_run.is_none() {
            self.run_notice = None;
        }
        self.image_view = None;
        self.file_notice = None;
        if let Ok(src) = self.notes.read(rel) {
            self.note_blocks = ox_markdown::parse(&src);
            self.editor = Some(NoteEditor::new(rel, &src));
            self.note_current = Some(rel.to_string());
            if let Some((dir, _)) = rel.rsplit_once('/') {
                self.note_dir = dir.to_string();
            }
        }
    }

    /// Teclas no campo "Filtrar drive...". Devolve `false` para deixar passar os atalhos globais (Ctrl+...).
    fn on_filter_key(&mut self, k: &gpui::Keystroke, cx: &mut Context<Self>) -> bool {
        if k.modifiers.control || k.modifiers.alt {
            return false;
        }
        match k.key.to_lowercase().as_str() {
            "escape" => {
                self.filter_focused = false;
                self.side_filter.clear();
            }
            "enter" => self.filter_focused = false,
            "backspace" => {
                self.side_filter.pop();
            }
            "space" => self.side_filter.push(' '),
            _ => {
                if let Some(c) = &k.key_char {
                    self.side_filter.push_str(c);
                }
            }
        }
        cx.notify();
        true
    }

    pub fn start_naming(&mut self, folder: bool) {
        self.naming = Some(Naming { folder, text: String::new(), rename: None, sessions: false, terminal: None });
        self.confirm_trash = None;
    }

    fn on_naming_key(&mut self, k: &gpui::Keystroke, cx: &mut Context<Self>) {
        let key = k.key.to_lowercase();
        match key.as_str() {
            "escape" => self.naming = None,
            "enter" => self.confirm_naming(),
            "backspace" => {
                if let Some(n) = self.naming.as_mut() {
                    n.text.pop();
                }
            }
            "space" => {
                if let Some(n) = self.naming.as_mut() {
                    n.text.push(' ');
                }
            }
            _ if !k.modifiers.control && !k.modifiers.alt => {
                if let (Some(c), Some(n)) = (&k.key_char, self.naming.as_mut()) {
                    if !c.contains('/') {
                        n.text.push_str(c);
                    }
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Começa a renomear uma pasta de notas no próprio lugar (Enter confirma, Esc cancela).
    pub fn start_rename_folder(&mut self, rel: &str) {
        let label = rel.rsplit('/').next().unwrap_or(rel).to_string();
        self.naming = Some(Naming { folder: true, text: label, rename: Some(rel.to_string()), sessions: false, terminal: None });
        self.confirm_trash = None;
        self.side_notice = None;
    }

    /// Renomeia um terminal no próprio lugar da lista (Enter confirma, Esc cancela).
    pub fn start_rename_terminal(&mut self, idx: usize) {
        let Some(t) = self.tabs.get(idx) else { return };
        let current = t.name.clone().unwrap_or_default();
        self.naming = Some(Naming { folder: false, text: current, rename: None, sessions: true, terminal: Some(t.session) });
        self.confirm_trash = None;
    }

    /// Nome mostrado de um terminal: o dado pelo usuário, senão o último comando, o título ou "shell N".
    pub fn terminal_label(&self, i: usize) -> String {
        let t = &self.tabs[i];
        if let Some(n) = &t.name {
            n.clone()
        } else if !t.last_cmd.is_empty() {
            t.last_cmd.clone()
        } else if !t.title.is_empty() {
            t.title.clone()
        } else {
            format!("shell {}", i + 1)
        }
    }

    /// Nova pasta na aba Sessões (o nome é digitado no campo que aparece na lista).
    pub fn start_session_folder(&mut self) {
        self.naming = Some(Naming { folder: true, text: String::new(), rename: None, sessions: true, terminal: None });
        self.confirm_trash = None;
    }

    pub fn start_rename_session_folder(&mut self, id: u64) {
        let Some(f) = self.session_folders.iter().find(|f| f.id == id) else { return };
        self.naming = Some(Naming { folder: true, text: f.name.clone(), rename: Some(id.to_string()), sessions: true, terminal: None });
        self.confirm_trash = None;
    }

    /// Põe a sessão arrastada na pasta (`None` = de volta à lista solta).
    pub fn move_session(&mut self, item: &DraggedSession, folder: Option<u64>) {
        if let Some(t) = self.tabs.iter_mut().find(|t| t.session == item.session) {
            t.folder = folder;
        }
        if let Some(f) = folder.and_then(|id| self.session_folders.iter_mut().find(|f| f.id == id)) {
            f.open = true;
        }
        self.save_session_folders();
    }

    /// Remove a pasta; os terminais dela continuam abertos, de volta à lista.
    /// Grava as pastas das Sessões (chamado a cada mudança: criar, renomear, abrir/fechar, remover).
    pub fn save_session_folders(&self) {
        folders::save(self.session_folders.iter().map(|f| (f.name.as_str(), f.open)));
    }

    pub fn remove_session_folder(&mut self, id: u64) {
        self.session_folders.retain(|f| f.id != id);
        for t in self.tabs.iter_mut().filter(|t| t.folder == Some(id)) {
            t.folder = None;
        }
        if self.session_folder_sel == Some(id) {
            self.session_folder_sel = None;
        }
        self.confirm_trash = None;
        self.save_session_folders();
    }

    /// Troca o prefixo `from` por `to` nos caminhos guardados (nota aberta, pastas abertas, pasta atual)
    /// depois de mover ou renomear algo.
    fn remap_note_paths(&mut self, from: &str, to: &str) {
        let remap = |p: &str| -> Option<String> {
            if p == from {
                Some(to.to_string())
            } else {
                p.strip_prefix(&format!("{from}/")).map(|rest| format!("{to}/{rest}"))
            }
        };
        if let Some(new) = self.note_current.as_deref().and_then(remap) {
            self.note_current = Some(new.clone());
            if let Some(ed) = self.editor.as_mut().filter(|e| e.file.is_none()) {
                ed.name = new;
            }
        }
        if let Some(new) = remap(&self.note_dir) {
            self.note_dir = new;
        }
        self.note_expanded = self.note_expanded.iter().map(|p| remap(p).unwrap_or_else(|| p.clone())).collect();
    }

    /// Solta uma nota/pasta arrastada dentro de `dest_dir` (`""` = todas as notas).
    pub fn move_note(&mut self, item: &DraggedNote, dest_dir: &str) {
        self.save_note();
        self.confirm_trash = None;
        match self.notes.move_into(&item.rel, item.is_dir, dest_dir) {
            Ok(new_rel) if new_rel != item.rel => {
                self.remap_note_paths(&item.rel, &new_rel);
                if !dest_dir.is_empty() {
                    self.note_expanded.insert(dest_dir.to_string());
                }
                self.side_notice = None;
            }
            Ok(_) => {}
            Err(e) => self.side_notice = Some(format!("Não deu para mover: {e}")),
        }
    }

    fn confirm_naming(&mut self) {
        let Some(n) = self.naming.take() else { return };
        if let Some(sid) = n.terminal {
            // nome vazio volta a mostrar o último comando
            if let Some(t) = self.tabs.iter_mut().find(|t| t.session == sid) {
                t.name = Some(n.text.trim().to_string()).filter(|s| !s.is_empty());
            }
            return;
        }
        let name = n.text.trim().trim_start_matches('.').to_string();
        if name.is_empty() {
            return;
        }
        if n.sessions {
            match n.rename.and_then(|r| r.parse::<u64>().ok()) {
                Some(id) => {
                    if let Some(f) = self.session_folders.iter_mut().find(|f| f.id == id) {
                        f.name = name;
                    }
                }
                None => {
                    let id = self.next_folder_id;
                    self.next_folder_id += 1;
                    self.session_folders.push(SessionFolder { id, name, open: true });
                    self.session_folder_sel = Some(id);
                }
            }
            self.save_session_folders();
            return;
        }
        if let Some(old) = n.rename {
            self.save_note();
            match self.notes.rename_folder(&old, &name) {
                Ok(new_rel) => self.remap_note_paths(&old, &new_rel),
                Err(e) => self.side_notice = Some(format!("Não deu para renomear: {e}")),
            }
            return;
        }
        let rel = if self.note_dir.is_empty() { name.clone() } else { format!("{}/{name}", self.note_dir) };
        if n.folder {
            if self.notes.create_folder(&rel).is_ok() {
                self.note_expanded.insert(rel.clone());
                self.note_dir = rel;
            }
        } else if self.notes.write(&rel, "").is_ok() {
            if !self.note_dir.is_empty() {
                self.note_expanded.insert(self.note_dir.clone());
            }
            let full = if rel.ends_with(".md") { rel } else { format!("{rel}.md") };
            self.open_note(&full);
            self.show_notes = true;
            self.set_note_mode(NoteMode::Edit);
            if let Some(ed) = self.editor.as_mut() {
                ed.buf.move_doc_end(false);
            }
        }
    }

    pub fn trash_note(&mut self, rel: &str, is_dir: bool) {
        self.save_note();
        self.confirm_trash = None;
        if self.notes.trash(rel, is_dir).is_ok() {
            let affected = |cur: &str| cur == rel || (is_dir && cur.starts_with(&format!("{rel}/")));
            if self.note_current.as_deref().is_some_and(affected) {
                self.note_current = None;
                self.editor = None;
                self.note_blocks.clear();
                self.focus_editor = false;
            }
            if is_dir {
                self.note_expanded.remove(rel);
                if self.note_dir == rel || self.note_dir.starts_with(&format!("{rel}/")) {
                    self.note_dir.clear();
                }
            }
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_size(window);
        self.note_latency();
        let maximized = window.is_maximized();
        // cantos arredondados sempre (também maximizada); as partes encostadas nos cantos se arredondam
        // junto, porque o recorte do GPUI é retangular e o fundo delas cobriria a curva
        let radius = WINDOW_RADIUS;
        self.corner_r = radius - 1.0;

        // brasa respirando: pede o próximo quadro só enquanto há comando rodando (parado não gasta nada)
        if self.tabs.iter().any(|t| !t.cmd_starts.is_empty()) {
            window.request_animation_frame();
        }
        let titlebar = self.render_titlebar(window, cx);
        let sidebar = self.show_sidebar.then(|| self.render_sidebar(cx));
        let terminal = self.render_panes(cx);
        let widgets = self.render_widget_bar(cx);
        let notes = self.show_notes.then(|| self.render_notes(window, cx));
        let settings_page = self.show_settings.then(|| self.render_settings(window, cx));

        // a barra lateral vai até o topo; o cabeçalho pertence só ao painel principal
        let r = px(self.corner_r);
        let main_pane = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(c_bg()))
            .rounded_r(r)
            .when(!self.show_sidebar, |d| d.rounded_l(r))
            .child(titlebar)
            .child(self.ember_line())
            .child(match settings_page {
                Some(page) => page,
                None => div().flex_1().min_h_0().flex().flex_row().when_some(notes, |d, n| d.child(n)).child(terminal).child(widgets).into_any_element(),
            });

        div()
            .size_full()
            .relative()
            .font_family(UI_FONT)
            .text_color(rgb(c_text()))
            .text_size(px(15.))
            .track_focus(&self.focus)
            .key_context("Brasa")
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| this.on_key(ev, window, cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, _| {
                    window.focus(&this.focus);
                    this.focus_editor = false;
                    this.filter_focused = false;
                    this.title_focused = false;
                    this.block_menu = None;
                    this.mon.filter_focused = false;
                    this.settings_field = None;
                    this.close_gsearch();
                }),
            )
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_row()
                    .rounded(px(radius))
                    .border_1()
                    .border_color(rgb(c_border()))
                    .overflow_hidden()
                    .when_some(sidebar, |d, s| d.child(s))
                    .child(main_pane),
            )
            // arrastar bordas: larguras da barra lateral, das notas e divisões entre painéis
            .children(self.width_handles(cx))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if this.resizing.is_some() {
                    if ev.pressed_button == Some(MouseButton::Left) {
                        this.on_resize_move(ev.position);
                    } else {
                        this.resizing = None;
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    if this.resizing.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    if this.resizing.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .children(if maximized { Vec::new() } else { self.resize_handles(cx) })
            .when_some(self.render_block_menu(cx), |d, m| d.child(m))
            .when_some(self.render_hist_search(cx), |d, m| d.child(m))
            .when_some(self.render_var_prompt(cx), |d, m| d.child(m))
    }
}

fn main() {
    Application::new().with_assets(assets::Assets).run(|cx: &mut App| {
        // fontes do Warp: Hack (terminal/código) e Roboto (interface e texto das notas)
        let _ = cx.text_system().add_fonts(assets::fonts());
        theme::load_accent();
        let bounds = Bounds::centered(None, size(px(1500.), px(900.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions { title: Some("Brasa".into()), appears_transparent: true, ..Default::default() }),
                // decoração própria: botões de janela e bordas redimensionáveis são desenhados por chrome.rs
                window_decorations: Some(WindowDecorations::Client),
                // fundo da janela desfocado onde o compositor suporta (KDE/Wayland); senão só transparente
                window_background: WindowBackgroundAppearance::Blurred,
                window_min_size: Some(size(px(760.), px(480.))),
                // nome da janela para o sistema (extensões de desfoque do GNOME escolhem por ele)
                app_id: Some("brasa".into()),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Workspace::new(window, cx)),
        )
        .unwrap();
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(id: SessionId, s: &str) -> (SessionId, PtyMsg) {
        (id, PtyMsg::Output(s.as_bytes().to_vec()))
    }

    #[test]
    fn coalesce_merges_consecutive_output_of_the_same_terminal() {
        let batch = vec![out(1, "ab"), out(1, "cd"), out(2, "x"), out(1, "e"), (1, PtyMsg::Exit), out(1, "f")];
        let merged = coalesce_output(batch);
        let shown: Vec<String> = merged
            .iter()
            .map(|(id, m)| match m {
                PtyMsg::Output(b) => format!("{id}:{}", String::from_utf8_lossy(b)),
                PtyMsg::Exit => format!("{id}:exit"),
            })
            .collect();
        // a ordem entre terminais e o fim da sessão é preservada; só leituras vizinhas do mesmo terminal se juntam
        assert_eq!(shown, ["1:abcd", "2:x", "1:e", "1:exit", "1:f"]);
    }
}
