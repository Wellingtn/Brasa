//! Posição das tabelas e caminho das setas do diagrama do banco. Só geometria, sem janela, para poder testar.
//!
//! As tabelas ficam em colunas: quem não aponta para ninguém fica à esquerda, e cada tabela fica uma coluna à
//! direita da mais à direita das que ela referencia. A seta sai do lado esquerdo da tabela que tem a chave
//! estrangeira e chega no lado direito da tabela referenciada, em segmentos horizontais e verticais.

use std::collections::HashMap;

use ox_db::Table;

pub const CARD_W: f32 = 224.0;
pub const GAP_X: f32 = 88.0;
pub const GAP_Y: f32 = 36.0;
pub const MARGIN: f32 = 28.0;
pub const HEAD_H: f32 = 34.0;
pub const ROW_H: f32 = 22.0;
/// colunas mostradas por tabela; o resto vira "+N colunas"
pub const MAX_ROWS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Placed {
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn mid_y(&self) -> f32 {
        self.y + self.h / 2.0
    }
}

/// Uma seta: do `from` (tem a chave estrangeira) para o `to` (referenciada).
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    /// segmentos (x1, y1, x2, y2), todos horizontais ou verticais, na ordem do caminho
    pub segs: Vec<(f32, f32, f32, f32)>,
    /// onde a ponta da seta fica (no lado direito da tabela referenciada)
    pub tip: (f32, f32),
}

#[derive(Debug, Default, PartialEq)]
pub struct Layout {
    /// uma posição por tabela da lista (`None` para visões, que não entram no diagrama)
    pub placed: Vec<Option<Placed>>,
    pub edges: Vec<Edge>,
    pub width: f32,
    pub height: f32,
}

fn card_height(columns: usize) -> f32 {
    let shown = columns.min(MAX_ROWS) + usize::from(columns > MAX_ROWS);
    HEAD_H + shown as f32 * ROW_H + 10.0
}

/// A tabela referenciada, se existir na lista.
fn find(tables: &[Table], schema: &Option<String>, name: &str, from: &Table) -> Option<usize> {
    tables.iter().position(|t| !t.is_view && t.name == name && (schema.is_none() || t.schema == *schema || (t.schema.is_none() && from.schema.is_none())))
}

/// `moved`: posição (x, y) que a pessoa escolheu arrastando, por índice de tabela; vale no lugar da calculada, e as
/// setas são traçadas de novo a partir dela.
pub fn layout(tables: &[Table], moved: &HashMap<usize, (f32, f32)>) -> Layout {
    let n = tables.len();
    // relações (filho, pai), sem visões, sem referência a si mesma e sem tabela desconhecida
    let mut rel: Vec<(usize, usize)> = Vec::new();
    for (c, t) in tables.iter().enumerate().filter(|(_, t)| !t.is_view) {
        for fk in &t.foreign_keys {
            if let Some(p) = find(tables, &fk.ref_schema, &fk.ref_table, t) {
                if p != c && !rel.contains(&(c, p)) {
                    rel.push((c, p));
                }
            }
        }
    }
    // coluna de cada tabela: uma à direita dos pais (com teto, para um ciclo não girar para sempre)
    let mut layer = vec![0usize; n];
    for _ in 0..n.max(1) {
        let mut changed = false;
        for &(c, p) in &rel {
            let want = (layer[p] + 1).min(n);
            if layer[c] < want {
                layer[c] = want;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut placed: Vec<Option<Placed>> = vec![None; n];
    let layers = tables.iter().enumerate().filter(|(_, t)| !t.is_view).map(|(i, _)| layer[i]).max().map_or(0, |m| m + 1);
    let (mut width, mut height) = (0f32, 0f32);
    for l in 0..layers {
        let mut ids: Vec<usize> = (0..n).filter(|&i| !tables[i].is_view && layer[i] == l).collect();
        ids.sort_by_key(|&i| tables[i].label().to_lowercase());
        let mut y = MARGIN;
        for i in ids {
            let h = card_height(tables[i].columns.len());
            let p = Placed { x: MARGIN + l as f32 * (CARD_W + GAP_X), y, w: CARD_W, h };
            width = width.max(p.right());
            y += h + GAP_Y;
            height = height.max(p.y + h);
            placed[i] = Some(p);
        }
    }

    for (i, p) in placed.iter_mut().enumerate() {
        if let (Some(p), Some(&(x, y))) = (p.as_mut(), moved.get(&i)) {
            p.x = x.max(0.0);
            p.y = y.max(0.0);
        }
    }
    width = placed.iter().flatten().map(|p| p.right()).fold(width, f32::max);
    height = placed.iter().flatten().map(|p| p.y + p.h).fold(height, f32::max);

    // setas; as que saem ou chegam no mesmo lado se espalham um pouco para não ficarem em cima umas das outras
    let (mut out_count, mut in_count) = (vec![0usize; n], vec![0usize; n]);
    let mut out_total = vec![0usize; n];
    let mut in_total = vec![0usize; n];
    for &(c, p) in &rel {
        out_total[c] += 1;
        in_total[p] += 1;
    }
    let spread = |k: usize, total: usize| (k as f32 - (total as f32 - 1.0) / 2.0) * 8.0;
    let mut edges = Vec::new();
    for &(c, p) in &rel {
        let (Some(cp), Some(pp)) = (placed[c], placed[p]) else { continue };
        let y1 = cp.mid_y() + spread(out_count[c], out_total[c]);
        let y2 = pp.mid_y() + spread(in_count[p], in_total[p]);
        out_count[c] += 1;
        in_count[p] += 1;
        let x2 = pp.right();
        let segs = if cp.x > x2 {
            let mid = x2 + (cp.x - x2) / 2.0;
            vec![(cp.x, y1, mid, y1), (mid, y1, mid, y2), (mid, y2, x2, y2)]
        } else {
            // mesma coluna ou ciclo: dá a volta pela direita
            let out = cp.right().max(x2) + 24.0;
            width = width.max(out + MARGIN / 2.0);
            vec![(cp.right(), y1, out, y1), (out, y1, out, y2), (out, y2, x2, y2)]
        };
        edges.push(Edge { from: c, to: p, segs, tip: (x2, y2) });
    }
    Layout { placed, edges, width: width + MARGIN, height: height + MARGIN }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ox_db::{Column, ForeignKey};

    fn table(name: &str, cols: usize, refs: &[(&str, &str)]) -> Table {
        Table {
            schema: Some("public".into()),
            name: name.into(),
            is_view: false,
            columns: (0..cols).map(|i| Column { name: format!("c{i}"), decl_type: "integer".into(), not_null: false, pk: usize::from(i == 0), default: None }).collect(),
            indexes: Vec::new(),
            without_rowid: false,
            foreign_keys: refs.iter().map(|(col, to)| ForeignKey { columns: vec![col.to_string()], ref_schema: Some("public".into()), ref_table: to.to_string(), ref_columns: vec!["c0".into()] }).collect(),
        }
    }

    fn shop() -> Vec<Table> {
        vec![
            table("categorias", 2, &[]),
            table("clientes", 5, &[]),
            table("pedidos", 4, &[("c1", "clientes")]),
            table("produtos", 5, &[("c2", "categorias")]),
            table("itens", 4, &[("c0", "pedidos"), ("c1", "produtos")]),
        ]
    }

    #[test]
    fn tables_sit_one_column_right_of_what_they_reference() {
        let t = shop();
        let l = layout(&t, &HashMap::new());
        let x = |n: &str| l.placed[t.iter().position(|x| x.name == n).unwrap()].unwrap().x;
        assert_eq!(x("clientes"), MARGIN);
        assert_eq!(x("categorias"), MARGIN);
        assert_eq!(x("pedidos"), MARGIN + CARD_W + GAP_X);
        assert_eq!(x("produtos"), MARGIN + CARD_W + GAP_X);
        assert_eq!(x("itens"), MARGIN + 2.0 * (CARD_W + GAP_X)); // depois de pedidos e de produtos
    }

    #[test]
    fn cards_in_a_column_never_overlap() {
        let t = shop();
        let l = layout(&t, &HashMap::new());
        let ps: Vec<Placed> = l.placed.iter().flatten().copied().collect();
        for (i, a) in ps.iter().enumerate() {
            for b in ps.iter().skip(i + 1) {
                let overlap = a.x < b.right() && b.x < a.right() && a.y < b.y + b.h && b.y < a.y + a.h;
                assert!(!overlap, "{a:?} e {b:?}");
            }
        }
        assert!(l.width >= ps.iter().map(|p| p.right()).fold(0.0, f32::max) && l.height >= ps.iter().map(|p| p.y + p.h).fold(0.0, f32::max));
    }

    #[test]
    fn arrows_run_from_the_fk_table_to_the_referenced_one() {
        let t = shop();
        let l = layout(&t, &HashMap::new());
        assert_eq!(l.edges.len(), 4);
        let idx = |n: &str| t.iter().position(|x| x.name == n).unwrap();
        let e = l.edges.iter().find(|e| e.from == idx("pedidos") && e.to == idx("clientes")).unwrap();
        let to = l.placed[idx("clientes")].unwrap();
        let from = l.placed[idx("pedidos")].unwrap();
        // chega no lado direito de clientes e sai do lado esquerdo de pedidos
        assert_eq!(e.tip.0, to.right());
        assert_eq!(e.segs.first().unwrap().0, from.x);
        assert_eq!(e.segs.last().unwrap().2, to.right());
        // só segmentos horizontais ou verticais, e cada um começa onde o anterior terminou
        for s in &e.segs {
            assert!(s.0 == s.2 || s.1 == s.3, "{s:?}");
        }
        for w in e.segs.windows(2) {
            assert_eq!((w[0].2, w[0].3), (w[1].0, w[1].1));
        }
    }

    #[test]
    fn views_self_references_and_unknown_tables_are_left_out() {
        let mut t = shop();
        t.push(Table { is_view: true, ..table("v_clientes", 2, &[("c0", "clientes")]) });
        t.push(table("arvore", 3, &[("c1", "arvore"), ("c2", "nao_existe")]));
        let l = layout(&t, &HashMap::new());
        assert_eq!(l.edges.len(), 4, "a visão, a auto-referência e a tabela inexistente não geram seta");
        assert!(l.placed[5].is_none(), "visão fora do diagrama");
        assert!(l.placed[6].is_some());
    }

    #[test]
    fn a_cycle_terminates_and_still_draws_both_arrows() {
        let t = vec![table("a", 2, &[("c1", "b")]), table("b", 2, &[("c1", "a")])];
        let l = layout(&t, &HashMap::new());
        assert_eq!(l.edges.len(), 2);
        assert!(l.placed.iter().all(|p| p.is_some()));
        for e in &l.edges {
            for s in &e.segs {
                assert!(s.0 == s.2 || s.1 == s.3);
            }
        }
    }

    #[test]
    fn many_columns_are_cut_and_arrows_on_one_edge_are_spread() {
        let t = vec![table("grande", 25, &[]), table("filho", 3, &[("c1", "grande"), ("c2", "grande")]), table("filho2", 3, &[("c1", "grande")])];
        let l = layout(&t, &HashMap::new());
        let h = l.placed[0].unwrap().h;
        assert_eq!(h, HEAD_H + (MAX_ROWS + 1) as f32 * ROW_H + 10.0);
        // as duas setas de "filho" para "grande" são uma só relação; as que chegam em "grande" têm alturas diferentes
        let tips: Vec<f32> = l.edges.iter().map(|e| e.tip.1).collect();
        assert_eq!(l.edges.len(), 2);
        assert_ne!(tips[0], tips[1]);
    }

    #[test]
    fn a_dragged_table_keeps_its_place_and_its_arrows_follow() {
        let t = shop();
        let idx = |n: &str| t.iter().position(|x| x.name == n).unwrap();
        let base = layout(&t, &HashMap::new());
        let (cli, ped) = (idx("clientes"), idx("pedidos"));
        let before = base.edges.iter().find(|e| e.from == ped && e.to == cli).unwrap().clone();
        // pedidos vai para mais embaixo e mais à direita
        let moved = HashMap::from([(ped, (700.0, 500.0))]);
        let l = layout(&t, &moved);
        let p = l.placed[ped].unwrap();
        assert_eq!((p.x, p.y), (700.0, 500.0));
        assert_eq!((p.w, p.h), (base.placed[ped].unwrap().w, base.placed[ped].unwrap().h));
        // a seta de pedidos para clientes sai do lado esquerdo da nova posição e ainda chega em clientes
        let after = l.edges.iter().find(|e| e.from == ped && e.to == cli).unwrap();
        assert_ne!(after, &before);
        assert_eq!(after.segs.first().unwrap().0, 700.0);
        assert_eq!(after.tip.0, l.placed[cli].unwrap().right());
        for s in &after.segs {
            assert!(s.0 == s.2 || s.1 == s.3);
        }
        // as outras tabelas não se mexeram, e o desenho cresce para caber
        assert_eq!(l.placed[cli], base.placed[cli]);
        assert!(l.height >= 500.0 + p.h && l.width >= 700.0 + p.w);
        // posição negativa é presa em zero
        let neg = layout(&t, &HashMap::from([(cli, (-50.0, -9.0))]));
        assert_eq!((neg.placed[cli].unwrap().x, neg.placed[cli].unwrap().y), (0.0, 0.0));
        // arrastar para um índice que não existe não faz nada
        assert_eq!(layout(&t, &HashMap::from([(99, (1.0, 1.0))])), base);
    }

    #[test]
    fn an_empty_database_has_an_empty_layout() {
        let l = layout(&[], &HashMap::new());
        assert!(l.placed.is_empty() && l.edges.is_empty());
    }
}
