//! Vários blocos na tela ao mesmo tempo, como no Wave: uma árvore de divisões (lado a lado ou
//! empilhado, meio a meio) cujas folhas são terminais ou os painéis de desempenho.

use ox_pty::SessionId;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaneKind {
    Term(SessionId),
    /// gráfico de uso da CPU
    Cpu,
    /// gráfico de memória
    Mem,
    /// tabela de processos
    Procs,
    /// gerenciador de arquivos (com visualizador de imagens)
    Files,
    /// Só existe para o botão do Banco na barra da direita; a página do banco é `Workspace::show_db`
    /// e ocupa o lugar do terminal, como as Configurações.
    Db,
}

impl PaneKind {
    /// Painéis de desempenho (abertos pela barra de widgets).
    pub fn is_widget(self) -> bool {
        !matches!(self, PaneKind::Term(_))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Leaf(PaneKind),
    /// `row`: true = lado a lado (a | b), false = empilhado (a em cima de b);
    /// `ratio`: fração do espaço que fica com `a` (arrastando a divisória muda)
    Split { row: bool, ratio: f32, a: Box<Node>, b: Box<Node> },
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

impl Node {
    /// Folhas na ordem de leitura (esquerda→direita, cima→baixo).
    pub fn leaves(&self) -> Vec<PaneKind> {
        match self {
            Node::Leaf(k) => vec![*k],
            Node::Split { a, b, .. } => {
                let mut v = a.leaves();
                v.extend(b.leaves());
                v
            }
        }
    }

    pub fn contains(&self, k: PaneKind) -> bool {
        self.leaves().contains(&k)
    }

    /// Retângulo de cada folha dentro de `r`, com `gap` px entre vizinhos.
    pub fn rects(&self, r: Rect, gap: f32) -> Vec<(PaneKind, Rect)> {
        match self {
            Node::Leaf(k) => vec![(*k, r)],
            Node::Split { row, ratio, a, b } => {
                let (ra, rb) = halves(r, *row, *ratio, gap);
                let mut v = a.rects(ra, gap);
                v.extend(b.rects(rb, gap));
                v
            }
        }
    }

    /// Divide a folha `target`, pondo `new` à direita (`row`) ou embaixo. Devolve se achou a folha.
    pub fn split(&mut self, target: PaneKind, new: PaneKind, row: bool) -> bool {
        match self {
            Node::Leaf(k) if *k == target => {
                *self = Node::Split { row, ratio: 0.5, a: Box::new(Node::Leaf(target)), b: Box::new(Node::Leaf(new)) };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.split(target, new, row) || b.split(target, new, row),
        }
    }

    /// Tira a folha `target`; a irmã ocupa o espaço. A última folha não sai (devolve false).
    pub fn remove(&mut self, target: PaneKind) -> bool {
        let Node::Split { a, b, .. } = self else { return false };
        if **a == Node::Leaf(target) {
            *self = (**b).clone();
            return true;
        }
        if **b == Node::Leaf(target) {
            *self = (**a).clone();
            return true;
        }
        a.remove(target) || b.remove(target)
    }

    /// Divisórias arrastáveis: (número da divisão, lado a lado?, faixa da divisória, área da divisão).
    /// O número segue a ordem da árvore e é o que `set_ratio` recebe.
    pub fn dividers(&self, r: Rect, gap: f32) -> Vec<(usize, bool, Rect, Rect)> {
        let mut out = Vec::new();
        self.collect_dividers(r, gap, &mut 0, &mut out);
        out
    }

    fn collect_dividers(&self, r: Rect, gap: f32, next: &mut usize, out: &mut Vec<(usize, bool, Rect, Rect)>) {
        if let Node::Split { row, ratio, a, b } = self {
            let id = *next;
            *next += 1;
            let (ra, rb) = halves(r, *row, *ratio, gap);
            let bar = if *row { Rect { x: ra.x + ra.w, w: gap, ..r } } else { Rect { y: ra.y + ra.h, h: gap, ..r } };
            out.push((id, *row, bar, r));
            a.collect_dividers(ra, gap, next, out);
            b.collect_dividers(rb, gap, next, out);
        }
    }

    /// Muda a proporção da divisão `id` (limitada para nenhum lado sumir).
    pub fn set_ratio(&mut self, id: usize, value: f32) -> bool {
        self.set_ratio_at(id, value.clamp(0.12, 0.88), &mut 0)
    }

    fn set_ratio_at(&mut self, id: usize, value: f32, next: &mut usize) -> bool {
        let Node::Split { ratio, a, b, .. } = self else { return false };
        if *next == id {
            *ratio = value;
            return true;
        }
        *next += 1;
        a.set_ratio_at(id, value, next) || b.set_ratio_at(id, value, next)
    }

    /// Troca o conteúdo de uma folha (ex.: abrir outra sessão no mesmo lugar).
    pub fn replace(&mut self, target: PaneKind, new: PaneKind) -> bool {
        match self {
            Node::Leaf(k) if *k == target => {
                *k = new;
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.replace(target, new) || b.replace(target, new),
        }
    }
}

/// Divide `r` em duas partes com `gap` no meio; `a` fica com a fração `ratio`.
fn halves(r: Rect, row: bool, ratio: f32, gap: f32) -> (Rect, Rect) {
    if row {
        let w = ((r.w - gap) * ratio).floor();
        (Rect { w, ..r }, Rect { x: r.x + w + gap, w: r.w - w - gap, ..r })
    } else {
        let h = ((r.h - gap) * ratio).floor();
        (Rect { h, ..r }, Rect { y: r.y + h + gap, h: r.h - h - gap, ..r })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PaneKind::*;

    #[test]
    fn split_rects_remove_and_replace() {
        let mut n = Node::Leaf(Term(1));
        assert!(n.split(Term(1), Term(2), true));
        assert!(n.split(Term(2), Procs, false));
        assert_eq!(n.leaves(), [Term(1), Term(2), Procs]);
        let r = n.rects(Rect { x: 0.0, y: 0.0, w: 206.0, h: 106.0 }, 6.0);
        assert_eq!(r[0], (Term(1), Rect { x: 0.0, y: 0.0, w: 100.0, h: 106.0 }));
        assert_eq!(r[1], (Term(2), Rect { x: 106.0, y: 0.0, w: 100.0, h: 50.0 }));
        assert_eq!(r[2], (Procs, Rect { x: 106.0, y: 56.0, w: 100.0, h: 50.0 }));
        assert!(r[2].1.contains(150.0, 80.0) && !r[2].1.contains(150.0, 10.0));
        // tirar o monitor: o terminal 2 volta a ocupar a coluna inteira
        assert!(n.remove(Procs));
        assert_eq!(n.leaves(), [Term(1), Term(2)]);
        assert!(n.replace(Term(2), Term(7)));
        assert!(n.remove(Term(1)));
        assert_eq!(n, Node::Leaf(Term(7)));
        // divisórias e proporção
        let mut n = Node::Leaf(Term(1));
        n.split(Term(1), Term(2), true);
        let d = n.dividers(Rect { x: 0.0, y: 0.0, w: 206.0, h: 50.0 }, 6.0);
        assert_eq!(d, [(0, true, Rect { x: 100.0, y: 0.0, w: 6.0, h: 50.0 }, Rect { x: 0.0, y: 0.0, w: 206.0, h: 50.0 })]);
        assert!(n.set_ratio(0, 0.25));
        assert_eq!(n.rects(Rect { x: 0.0, y: 0.0, w: 206.0, h: 50.0 }, 6.0)[0].1.w, 50.0);
        assert!(n.set_ratio(0, 0.0)); // limitado: o lado esquerdo não some
        assert!(n.rects(Rect { x: 0.0, y: 0.0, w: 206.0, h: 50.0 }, 6.0)[0].1.w >= 24.0);
        assert!(!n.set_ratio(5, 0.5));
        let mut n = Node::Leaf(Term(7));
        // a última folha fica
        assert!(!n.remove(Term(7)));
        assert!(!n.split(Term(99), Procs, true));
    }
}
