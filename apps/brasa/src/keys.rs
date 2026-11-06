//! Atalhos de teclado do Brasa. Cada ação tem um ou mais atalhos de fábrica; quem quiser troca nas
//! configurações (aba Geral) ou editando `~/.config/brasa/keys.txt`, uma linha por ação:
//!
//! ```text
//! nova-sessao = ctrl+shift+t
//! zoom-mais = ctrl+=, ctrl+plus
//! ```
//!
//! Só as ações trocadas ficam no arquivo; apagar a linha volta ao atalho de fábrica.

use gpui::Keystroke;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Search,
    Copy,
    Paste,
    NewTab,
    CloseTab,
    PrevTab,
    NextTab,
    ToggleSidebar,
    ToggleNotes,
    SplitRight,
    SplitDown,
    ClosePane,
    Performance,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Settings,
    GlobalSearch,
    HistorySearch,
}

/// (ação, nome no arquivo, descrição, atalhos de fábrica)
pub const ACTIONS: &[(Action, &str, &str, &[&str])] = &[
    (Action::NewTab, "nova-sessao", "Novo terminal", &["ctrl+shift+t"]),
    (Action::CloseTab, "fechar-sessao", "Fechar terminal", &["ctrl+shift+w"]),
    (Action::PrevTab, "sessao-anterior", "Sessão anterior", &["ctrl+shift+left"]),
    (Action::NextTab, "proxima-sessao", "Próxima sessão", &["ctrl+shift+right"]),
    (Action::Copy, "copiar", "Copiar", &["ctrl+shift+c"]),
    (Action::Paste, "colar", "Colar", &["ctrl+shift+v"]),
    (Action::Search, "buscar", "Buscar na saída", &["ctrl+shift+f"]),
    (Action::ToggleSidebar, "barra-lateral", "Mostrar ou esconder a barra lateral", &["ctrl+shift+e"]),
    (Action::ToggleNotes, "notas", "Mostrar ou esconder as notas", &["ctrl+shift+n"]),
    (Action::SplitRight, "dividir-lado", "Dividir lado a lado", &["ctrl+shift+d"]),
    (Action::SplitDown, "dividir-baixo", "Dividir embaixo", &["ctrl+shift+b"]),
    (Action::ClosePane, "fechar-painel", "Tirar o painel da tela", &["ctrl+shift+x"]),
    (Action::Performance, "desempenho", "Desempenho (CPU, memória, processos)", &["ctrl+shift+m"]),
    (Action::ZoomIn, "zoom-mais", "Aumentar a fonte", &["ctrl+=", "ctrl+plus"]),
    (Action::ZoomOut, "zoom-menos", "Diminuir a fonte", &["ctrl+-"]),
    (Action::ZoomReset, "zoom-padrao", "Tamanho padrão da fonte", &["ctrl+0"]),
    (Action::HistorySearch, "historico", "Buscar no histórico de comandos", &["ctrl+r"]),
    (Action::GlobalSearch, "busca-global", "Buscar em notas e sessões", &["ctrl+k"]),
    (Action::Settings, "configuracoes", "Abrir as configurações", &["ctrl+,"]),
];

fn defaults(a: Action) -> Vec<String> {
    ACTIONS.iter().find(|x| x.0 == a).map(|x| x.3.iter().map(|s| s.to_string()).collect()).unwrap_or_default()
}

/// Atalho normalizado de uma tecla apertada ("ctrl+shift+t"); vazio se for só um modificador.
pub fn combo_of(k: &Keystroke) -> String {
    let key = match k.key.to_lowercase().as_str() {
        "+" => "plus".to_string(),
        " " => "space".to_string(),
        "control" | "ctrl" | "shift" | "alt" | "super" | "meta" | "platform" | "function" | "" => return String::new(),
        other => other.to_string(),
    };
    let m = &k.modifiers;
    let mut out = String::new();
    for (on, name) in [(m.control, "ctrl"), (m.alt, "alt"), (m.shift, "shift"), (m.platform, "super")] {
        if on {
            out.push_str(name);
            out.push('+');
        }
    }
    out + &key
}

/// "ctrl+shift+t" -> "Ctrl+Shift+T".
pub fn pretty(combo: &str) -> String {
    combo
        .split('+')
        .map(|p| match p {
            "plus" => "+".to_string(),
            "ctrl" => "Ctrl".to_string(),
            "alt" => "Alt".to_string(),
            "shift" => "Shift".to_string(),
            "super" => "Super".to_string(),
            "left" => "←".to_string(),
            "right" => "→".to_string(),
            "up" => "↑".to_string(),
            "down" => "↓".to_string(),
            other => {
                let mut c = other.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Atalhos aceitos: precisam de Ctrl, Alt ou Super, para não roubar o que se digita no terminal.
pub fn is_valid(combo: &str) -> bool {
    combo.split('+').count() >= 2 && (combo.starts_with("ctrl+") || combo.contains("alt+") || combo.contains("super+"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keymap {
    binds: Vec<(Action, Vec<String>)>,
}

impl Keymap {
    pub fn factory() -> Self {
        Keymap { binds: ACTIONS.iter().map(|x| (x.0, defaults(x.0))).collect() }
    }

    fn path() -> std::path::PathBuf {
        crate::config_dir().join("keys.txt")
    }

    pub fn load() -> Self {
        Self::parse(&std::fs::read_to_string(Self::path()).unwrap_or_default())
    }

    /// Lê o arquivo: linhas inválidas, ações desconhecidas e atalhos inválidos são ignorados.
    pub fn parse(text: &str) -> Self {
        let mut map = Self::factory();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((name, combos)) = line.split_once('=') else { continue };
            let Some(action) = ACTIONS.iter().find(|x| x.1 == name.trim()).map(|x| x.0) else { continue };
            let combos: Vec<String> = combos.split(',').map(|c| c.trim().to_lowercase()).filter(|c| is_valid(c)).collect();
            if !combos.is_empty() {
                map.binds.iter_mut().find(|b| b.0 == action).unwrap().1 = combos;
            }
        }
        map
    }

    pub fn save(&self) {
        let mut text = String::from("# Atalhos trocados; apague a linha para voltar ao de fábrica.\n");
        for (action, combos) in &self.binds {
            if *combos != defaults(*action) {
                let name = ACTIONS.iter().find(|x| x.0 == *action).unwrap().1;
                text.push_str(&format!("{name} = {}\n", combos.join(", ")));
            }
        }
        let _ = std::fs::write(Self::path(), text);
    }

    pub fn lookup(&self, k: &Keystroke) -> Option<Action> {
        let combo = combo_of(k);
        if combo.is_empty() {
            return None;
        }
        self.binds.iter().find(|(_, c)| c.iter().any(|x| *x == combo)).map(|b| b.0)
    }

    pub fn combos(&self, a: Action) -> &[String] {
        self.binds.iter().find(|b| b.0 == a).map(|b| b.1.as_slice()).unwrap_or(&[])
    }

    /// Troca o atalho da ação. Se o atalho já é de outra ação, não troca e devolve essa ação.
    pub fn set(&mut self, a: Action, combo: &str) -> Result<(), Action> {
        if let Some((other, _)) = self.binds.iter().find(|(o, c)| *o != a && c.iter().any(|x| x == combo)) {
            return Err(*other);
        }
        self.binds.iter_mut().find(|b| b.0 == a).unwrap().1 = vec![combo.to_string()];
        Ok(())
    }

    pub fn reset(&mut self, a: Action) {
        self.binds.iter_mut().find(|b| b.0 == a).unwrap().1 = defaults(a);
    }

    pub fn is_default(&self, a: Action) -> bool {
        self.combos(a) == defaults(a).as_slice()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> Keystroke {
        Keystroke::parse(s).unwrap()
    }

    #[test]
    fn keystrokes_are_normalized() {
        assert_eq!(combo_of(&key("ctrl-shift-t")), "ctrl+shift+t");
        assert_eq!(combo_of(&key("shift-ctrl-t")), "ctrl+shift+t");
        assert_eq!(combo_of(&key("ctrl-=")), "ctrl+=");
        assert_eq!(combo_of(&key("ctrl-shift-left")), "ctrl+shift+left");
        assert_eq!(combo_of(&key("alt-x")), "alt+x");
        assert_eq!(combo_of(&key("t")), "t");
        assert_eq!(pretty("ctrl+shift+t"), "Ctrl+Shift+T");
        assert_eq!(pretty("ctrl+shift+left"), "Ctrl+Shift+←");
        assert_eq!(pretty("ctrl+plus"), "Ctrl++");
    }

    #[test]
    fn factory_shortcuts_are_found_and_unique() {
        let map = Keymap::factory();
        assert_eq!(map.lookup(&key("ctrl-shift-t")), Some(Action::NewTab));
        assert_eq!(map.lookup(&key("ctrl-shift-m")), Some(Action::Performance));
        assert_eq!(map.lookup(&key("ctrl-=")), Some(Action::ZoomIn));
        assert_eq!(map.lookup(&key("ctrl-0")), Some(Action::ZoomReset));
        assert_eq!(map.lookup(&key("ctrl-c")), None);
        assert_eq!(map.lookup(&key("t")), None);
        let mut all: Vec<&str> = ACTIONS.iter().flat_map(|x| x.3.iter().copied()).collect();
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n, "atalho de fábrica repetido");
        assert!(ACTIONS.iter().all(|x| x.3.iter().all(|c| is_valid(c))));
    }

    #[test]
    fn user_file_overrides_only_what_it_names() {
        let map = Keymap::parse("# comentário\nnova-sessao = ctrl+alt+t\nzoom-mais = ctrl+plus\ncopiar = c\nnao-existe = ctrl+q\nlixo\n");
        assert_eq!(map.lookup(&key("ctrl-alt-t")), Some(Action::NewTab));
        assert_eq!(map.lookup(&key("ctrl-shift-t")), None);
        assert_eq!(map.combos(Action::ZoomIn), ["ctrl+plus"]);
        // "c" sozinho não vale: continua o de fábrica
        assert_eq!(map.combos(Action::Copy), ["ctrl+shift+c"]);
        assert_eq!(map.combos(Action::Paste), ["ctrl+shift+v"]);
    }

    #[test]
    fn rebinding_refuses_conflicts() {
        let mut map = Keymap::factory();
        assert_eq!(map.set(Action::NewTab, "ctrl+shift+w"), Err(Action::CloseTab));
        assert_eq!(map.set(Action::NewTab, "ctrl+alt+t"), Ok(()));
        assert!(!map.is_default(Action::NewTab));
        map.reset(Action::NewTab);
        assert!(map.is_default(Action::NewTab));
        assert_eq!(map, Keymap::factory());
    }
}
