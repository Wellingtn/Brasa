//! Tema "brasa": fundos em carvão (preto quente com um fio de vermelho), cinzas de cinza e dois
//! acentos tirados da brasa: coral = algo rodando/ação de executar e a seção Sessões; âmbar = foco,
//! seleção, dicas de teclado e a seção Notas. Verde-água e vermelho seguem só como estado (ok / erro).

/// painel principal (terminal e notas): carvão
pub const BG: u32 = 0x161213;
/// barra lateral
pub const PANEL: u32 = 0x211a1a;
/// barra lateral translúcida (RRGGBBAA, 90% opaca = 10% de transparência): a área de trabalho aparece de leve por trás. O desfoque
/// vem do sistema: o app pede ao compositor (funciona no KDE); no GNOME, pela extensão Blur my Shell
/// aplicada ao app "brasa"
pub const PANEL_GLASS: u32 = 0x211a1ae6;
/// linha selecionada, chips
pub const SURFACE: u32 = 0x3a2b27;
/// linhas finas de separação
pub const BORDER: u32 = 0x2c2322;
/// hover de linhas de lista
pub const HOVER: u32 = 0x2a201f;
/// hover de botões
pub const BTN_HOVER: u32 = 0x3d2e2a;
pub const TEXT: u32 = 0xefe6e2;
pub const MUTED: u32 = 0xa3938e;
pub const DISABLED: u32 = 0x6f625e;

/// brasa: o acento principal
pub const CORAL: u32 = 0xe8805f;
/// pêssego: o miolo claro da brasa (gráfico de CPU, logo)
pub const PEACH: u32 = 0xf5b48e;
/// verde-água: só estado "ok" (carga baixa, processo rodando)
pub const TEAL: u32 = 0x7cc7b4;
/// fundos do seletor da barra lateral, um por seção (a cor da seção bem escurecida)
pub const CORAL_DIM: u32 = 0x4a2a21;
pub const AMBER_DIM: u32 = 0x3d2c16;
/// âmbar: a parte amarela da brasa (foco, seleção, dicas, Notas)
pub const AMBER: u32 = 0xe9b25f;
pub const ACCENT: u32 = AMBER;
pub const RUN_BG: u32 = 0xc0593f;
pub const RUN_HOVER: u32 = 0xd2694d;
pub const CODE_BG: u32 = 0x1f1817;

/// contorno de cartões e menus flutuantes
pub const CARD_BORDER: u32 = 0x40312d;
/// linhas divisórias entre painéis
pub const DIVIDER: u32 = 0x3a2d2a;
/// faixa da linha de comando de cada bloco (como a linha do prompt destacada)
pub const CMD_BG: u32 = 0x251c1b;
pub const OUT_TEXT: u32 = 0xe8deda;
/// bloco com erro: faixa vermelha e fundo levemente avermelhado
pub const ERROR_BG: u32 = 0x2a1517;
pub const SELECTED_BLOCK_BG: u32 = 0x2a1d19;

pub const TERM_BG: u32 = BG;
pub const TERM_FG: u32 = 0xefe6e2;
pub const SELECTION: u32 = 0x5a3427;
/// linhas que separam blocos na grade crua
pub const SEP: u32 = BORDER;
pub const SEP_ACTIVE: u32 = 0x7a4433;

// paleta ANSI (base do Warp, AGPL-3.0, veja NOTICE.md)
pub const GREEN: u32 = 0xb4fa72;
pub const YELLOW: u32 = 0xfefdc2;
pub const PURPLE: u32 = 0xff8ffd;
pub const RED: u32 = 0xff8272;

pub const WIN_CLOSE_HOVER: u32 = 0xe81123;
