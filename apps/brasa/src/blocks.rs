//! Geometria da lista de blocos (comando + saída): posições, rolagem e acerto do mouse.
//! Tudo em pixels, sem GPUI, para poder ser testado.

use std::ops::Range;

#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    pub cell_h: f32,
    pub header_h: f32,
    pub cmd_h: f32,
    /// espaço abaixo da saída, dentro do cartão
    pub pad_b: f32,
    /// espaço entre cartões
    pub gap: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockBox {
    pub top: f32,
    pub h: f32,
    /// topo da área de saída (relativo à lista)
    pub rows_top: f32,
    /// linhas de saída visíveis (0 se recolhido)
    pub rows: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    None,
    Header(usize),
    Command(usize),
    /// (bloco, índice da linha dentro da saída)
    Row(usize, usize),
}

/// `rows[i]` = linhas de saída do bloco i (já 0 quando recolhido). Devolve as caixas e a altura total.
pub fn layout(rows: &[usize], m: &Metrics) -> (Vec<BlockBox>, f32) {
    let mut boxes = Vec::with_capacity(rows.len());
    let mut y = 0.0;
    for &n in rows {
        let rows_top = y + m.header_h + m.cmd_h;
        // sem saída: só o cabeçalho; com saída: respiro em cima (cmd_h), linhas e respiro embaixo (pad_b)
        let body = if n > 0 { m.cmd_h + n as f32 * m.cell_h + m.pad_b } else { 0.0 };
        let h = m.header_h + body;
        boxes.push(BlockBox { top: y, h, rows_top, rows: n });
        y += h + m.gap;
    }
    let total = if boxes.is_empty() { 0.0 } else { y - m.gap };
    (boxes, total)
}

pub fn max_scroll(total: f32, view_h: f32) -> f32 {
    (total - view_h).max(0.0)
}

/// Blocos que cruzam a janela [scroll, scroll + view_h].
pub fn visible_blocks(boxes: &[BlockBox], scroll: f32, view_h: f32) -> Range<usize> {
    let first = boxes.partition_point(|b| b.top + b.h < scroll);
    let last = boxes.partition_point(|b| b.top <= scroll + view_h);
    first..last.max(first)
}

/// Linhas de saída de um bloco que cruzam a janela.
pub fn visible_rows(b: &BlockBox, scroll: f32, view_h: f32, m: &Metrics) -> Range<usize> {
    if b.rows == 0 {
        return 0..0;
    }
    let from = ((scroll - b.rows_top) / m.cell_h).floor().max(0.0) as usize;
    let to = (((scroll + view_h - b.rows_top) / m.cell_h).ceil().max(0.0) as usize).min(b.rows);
    from.min(b.rows)..to.max(from.min(b.rows))
}

/// O que está sob a coordenada `y` (relativa ao início da lista, já somada à rolagem).
pub fn hit(boxes: &[BlockBox], y: f32, m: &Metrics) -> Hit {
    let i = boxes.partition_point(|b| b.top + b.h <= y);
    let Some(b) = boxes.get(i).filter(|b| y >= b.top) else { return Hit::None };
    if y < b.top + m.header_h {
        Hit::Header(i)
    } else if y < b.rows_top {
        Hit::Command(i)
    } else {
        let k = ((y - b.rows_top) / m.cell_h) as usize;
        if k < b.rows { Hit::Row(i, k) } else { Hit::None }
    }
}

/// Rolagem que deixa a linha `k` do bloco `i` no meio da janela (para a busca).
pub fn scroll_to_row(boxes: &[BlockBox], i: usize, k: usize, view_h: f32, total: f32, m: &Metrics) -> f32 {
    let y = boxes[i].rows_top + k as f32 * m.cell_h;
    (y - view_h / 2.0).clamp(0.0, max_scroll(total, view_h))
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: Metrics = Metrics { cell_h: 20.0, header_h: 30.0, cmd_h: 26.0, pad_b: 8.0, gap: 10.0 };

    #[test]
    fn layout_stacks_blocks_with_gaps() {
        let (b, total) = layout(&[2, 0, 3], &M);
        assert_eq!(b[0], BlockBox { top: 0.0, h: 30.0 + 26.0 + 40.0 + 8.0, rows_top: 56.0, rows: 2 });
        assert_eq!(b[1].top, b[0].h + 10.0);
        assert_eq!(b[1].h, 30.0); // sem saída: só o cabeçalho
        assert_eq!(total, b[2].top + b[2].h);
        assert_eq!(layout(&[], &M).1, 0.0);
    }

    #[test]
    fn hit_testing_header_command_rows_and_gaps() {
        let (b, _) = layout(&[2, 1], &M);
        assert_eq!(hit(&b, 5.0, &M), Hit::Header(0));
        assert_eq!(hit(&b, 40.0, &M), Hit::Command(0));
        assert_eq!(hit(&b, 56.0, &M), Hit::Row(0, 0));
        assert_eq!(hit(&b, 56.0 + 25.0, &M), Hit::Row(0, 1));
        assert_eq!(hit(&b, 56.0 + 41.0, &M), Hit::None); // padding inferior
        assert_eq!(hit(&b, b[0].h + 5.0, &M), Hit::None); // espaço entre blocos
        assert_eq!(hit(&b, b[1].top + 1.0, &M), Hit::Header(1));
        assert_eq!(hit(&b, -3.0, &M), Hit::None);
        assert_eq!(hit(&b, 9999.0, &M), Hit::None);
    }

    #[test]
    fn visibility_ranges_follow_the_scroll() {
        let (b, total) = layout(&[100, 100, 100], &M); // 3 blocos de ~2.1k px
        let v = visible_blocks(&b, 0.0, 500.0);
        assert_eq!(v, 0..1);
        let mid = b[1].top + 100.0;
        assert_eq!(visible_blocks(&b, mid, 300.0), 1..2);
        assert_eq!(visible_blocks(&b, b[0].h - 10.0, 100.0), 0..2); // cruza a fronteira
        // linhas visíveis: só as que cabem na janela
        let r = visible_rows(&b[1], b[1].rows_top + 200.0, 100.0, &M);
        assert_eq!(r, 10..15);
        assert_eq!(visible_rows(&b[1], 0.0, 10.0, &M), 0..0); // janela acima do bloco
        assert_eq!(max_scroll(total, 500.0), total - 500.0);
        assert_eq!(max_scroll(100.0, 500.0), 0.0);
    }

    #[test]
    fn scroll_to_row_centers_and_clamps() {
        let (b, total) = layout(&[100], &M);
        let s = scroll_to_row(&b, 0, 50, 400.0, total, &M);
        assert_eq!(s, b[0].rows_top + 50.0 * 20.0 - 200.0);
        assert_eq!(scroll_to_row(&b, 0, 0, 400.0, total, &M), 0.0);
        assert_eq!(scroll_to_row(&b, 0, 99, 400.0, total, &M), max_scroll(total, 400.0));
    }
}
