# Roadmap do Brasa

Ordenado por custo-benefício: o que é mais útil e mais fácil de fazer vem primeiro.
Esforço: P = dias, M = 1–2 semanas, G = várias semanas.

## 1.1.0 — Acabamento e personalização (feito)
- Estilização "brasa" no app: fio de brasa sob o cabeçalho que respira enquanto um comando roda e o logo que acompanha a cor.
- Configurações (engrenagem na barra superior): aba Geral e aba Aparência, com cor do tema por roda de matizes ou hexadecimal, fundo, fonte com busca e tamanho em régua, tudo salvo em `~/.config/brasa`.
- Comandos compridos nas notas quebram a linha dentro do cartão.
- Busca global na barra superior (`Ctrl+K`): acha notas pelo nome e pelo texto (mostra a linha encontrada) e sessões pelo nome e pelo que apareceu na saída; o filtro por nome da barra lateral continua.
- Aviso de nova versão: verifica a release mais recente ao abrir (pode ser desligado) e mostra uma bolinha na engrenagem.
- Atalhos configuráveis: clique no atalho nas configurações e aperte o novo; ficam em `~/.config/brasa/keys.txt`.
- Medidor de latência do teclado nas configurações (aba Geral, Diagnóstico).

## 1.2.0 — Dia a dia mais rápido (P a M)
- Comparar a latência de teclado do Brasa com a de outros terminais, com um método que dê para repetir (o medidor do Brasa já existe).
- Autocompletar e histórico inteligente (aproveita o `ox-suggest`).
- Notas executáveis mais ricas: variáveis, rodar blocos em sequência, guardar a saída na nota.
- Mais formas de instalar: Flatpak e `.rpm` (o AUR vem depois, se houver interesse).

## 1.3.0 — Terminal completo (M)
- Protocolo de imagens Kitty (e Sixel, se sobrar tempo).
- SSH integrado: abrir uma sessão remota como uma sessão normal do Brasa.
- Mais vazão com saída enorme (tirar análise e desenho da thread da interface).

## 1.4.0 — MCP (M a G)
- Conectar servidores MCP e mostrar as ferramentas deles.
- Um assistente poder usar blocos e notas do Brasa.

## 1.5.0 — Editor de banco de dados (G)
- Primeiro SQLite (sem servidor, mais fácil), depois PostgreSQL e MySQL.
- Ver tabelas, rodar consultas, editar linhas.

## 2.0.0 — Outros sistemas (G)
- macOS e Windows. É a maior mudança: GPUI e o PTY precisam de ajustes em cada sistema.

## Por que essa ordem
- 1.1.0 e 1.2.0 não mexem na base e todo usuário percebe.
- 1.3.0 abre o Brasa para fluxos de trabalho mais pesados, mas toca no núcleo do terminal.
- MCP e banco de dados são os recursos que mais diferenciam o Brasa, mas são grandes e se apoiam na configuração (1.1.0) e na busca.
- Mac e Windows ficam por último porque dobram o trabalho de teste.
