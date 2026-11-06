//! Barra de rolagem fina (só indicador) para as listas. O GPUI rola por conta própria; aqui só se
//! calcula e desenha o "polegar" a partir do estado do `ScrollHandle`.

use gpui::{div, prelude::*, px, rgb, Div, ScrollHandle, Stateful};

use crate::theme::*;

const MIN_THUMB: f32 = 28.0;

/// (topo, altura) do polegar dentro de uma área de altura `view_h`.
/// `max_scroll` = quanto o conteúdo ultrapassa a área; `offset` = quanto já rolou (>= 0).
/// `None` quando não há o que rolar.
pub fn thumb_geometry(view_h: f32, max_scroll: f32, offset: f32) -> Option<(f32, f32)> {
    if view_h <= 0.0 || max_scroll <= 1.0 {
        return None;
    }
    let total = view_h + max_scroll;
    let len = (view_h * view_h / total).clamp(MIN_THUMB.min(view_h), view_h);
    let t = (offset / max_scroll).clamp(0.0, 1.0);
    Some(((view_h - len) * t, len))
}

pub fn thumb(top: f32, len: f32) -> Div {
    div().absolute().right(px(2.)).top(px(top)).w(px(4.)).h(px(len)).rounded_full().bg(rgb(c_btn_hover()))
}

/// Área rolável com polegar. `content` é a coluna de itens (já com `id`); aqui ganha rolagem e o rastreio.
pub fn scroll_list(content: Stateful<Div>, h: &ScrollHandle) -> Div {
    let geo = thumb_geometry(f32::from(h.bounds().size.height), f32::from(h.max_offset().height), -f32::from(h.offset().y));
    div()
        .relative()
        .flex_1()
        .min_h_0()
        .child(content.size_full().overflow_y_scroll().track_scroll(h))
        .when_some(geo, |d, (top, len)| d.child(thumb(top, len)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_thumb_when_content_fits() {
        assert_eq!(thumb_geometry(400.0, 0.0, 0.0), None);
        assert_eq!(thumb_geometry(400.0, 0.5, 0.0), None);
        assert_eq!(thumb_geometry(0.0, 100.0, 0.0), None);
    }

    #[test]
    fn thumb_size_is_proportional_and_has_a_minimum() {
        // conteúdo 2x a área -> polegar de metade da altura
        assert_eq!(thumb_geometry(400.0, 400.0, 0.0), Some((0.0, 200.0)));
        // conteúdo enorme -> não some: altura mínima
        let (_, len) = thumb_geometry(400.0, 1_000_000.0, 0.0).unwrap();
        assert_eq!(len, 28.0);
    }

    #[test]
    fn thumb_follows_the_scroll_offset_and_stays_inside() {
        let (top0, len) = thumb_geometry(400.0, 400.0, 0.0).unwrap();
        let (top_mid, _) = thumb_geometry(400.0, 400.0, 200.0).unwrap();
        let (top_end, _) = thumb_geometry(400.0, 400.0, 400.0).unwrap();
        assert_eq!((top0, top_mid, top_end), (0.0, 100.0, 200.0));
        assert_eq!(top_end + len, 400.0); // encosta no fim, sem passar
        // rolagem além do limite (elástico) é contida
        assert_eq!(thumb_geometry(400.0, 400.0, 9999.0).unwrap().0, 200.0);
        assert_eq!(thumb_geometry(400.0, 400.0, -50.0).unwrap().0, 0.0);
    }
}
