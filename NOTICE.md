# Avisos de terceiros

O Brasa é distribuído sob a **GNU Affero General Public License v3.0** (veja `LICENSE`).

## Warp

Partes do visual são derivadas do [Warp](https://github.com/warpdotdev/warp),
Copyright (C) 2020-2026 Denver Technologies, Inc., licenciado sob AGPL-3.0:

- ícones em `apps/brasa/assets/icons/` e `apps/brasa/assets/file_type/` (copiados de `app/assets/bundled/svg/`);
- cores do tema escuro padrão (fundo, texto, acento, paleta ANSI e as proporções das superfícies, contornos e textos secundários).

Nenhum código do Warp foi copiado: o Warp usa um framework de interface próprio; o Brasa usa GPUI.

## Fontes

- **Hack** (`apps/brasa/assets/fonts/hack/`): licença MIT com a licença Bitstream Vera; veja `LICENSE.md` na pasta.
- **Roboto** (`apps/brasa/assets/fonts/roboto/`): veja `LICENSE.txt` na pasta.

## Outros

- `vendor/xattr`: crate `xattr` 0.2.3 (MIT/Apache-2.0) com um patch de uma linha.


## Wave Terminal

O painel de desempenho (gráficos de CPU/memória e a tabela de processos com sinais) e os painéis
divididos seguem o desenho do "sysinfo", do "processviewer" e do layout em blocos do
[Wave Terminal](https://github.com/wavetermdev/waveterm) (Apache-2.0). A implementação em Rust é
própria; nenhum código do Wave foi copiado.
