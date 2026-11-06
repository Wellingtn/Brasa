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

## 1.2.0 — Dia a dia mais rápido (P a M) (feito)

Seis marcos, na ordem em que eu faria. Cada um tem o que entra e "pronto quando", para dar para conferir.

| Marco | O que entra | Esforço | Situação |
|---|---|---|---|
| **M1. Atualização dentro do app** | Botão "Atualizar", verificação SHA-256, `SHA256SUMS` na release | P | Feito; provado de 1.2.2 para 1.2.3 no `.deb` (AppImage ainda sem teste) |
| **M2. Autocompletar e histórico inteligente** | Ranking por frequência, recência e pasta; busca no histórico (`Ctrl+R`); mais comandos com sugestão de opções | M | Feito |
| **M3. Notas executáveis mais ricas** | Variáveis, "Rodar todos" em sequência, saída guardada na nota | M | Feito |
| **M4. Latência comparada** | Método repetível, medição em outros terminais, tabela e script no repositório | P a M | Método e script prontos; comparação justa com terminais de GPU em aberto (ver `docs/latencia.md`) |
| **M5. Mais formas de instalar** | Flatpak e `.rpm` publicados pelo workflow | M | Pacotes prontos no workflow; falta testar em Fedora e Flatpak reais |
| **M6. Lançamento** | Versão 1.2.0, README e captura de tela novos, tag, release conferida, atualização testada | P | Feito (1.2.0 a 1.2.3); falta a captura de tela nova no README |

### M1. Atualização dentro do app
- Já está: botão na barra superior e nas configurações, AppImage troca o próprio arquivo, `.deb` instala com `pkexec apt-get`, soma conferida, cópia compilada só avisa.
- Pronto quando: de uma 1.2.0 instalada (AppImage e `.deb`), a release seguinte aparece como "Atualizar", instala, reabre e mostra a versão nova. Só dá para provar depois de publicar duas versões seguidas, por isso fica para o M6.

### M2. Autocompletar e histórico inteligente
- Sugestões ordenadas por quanto e quando o comando foi usado, e priorizando o que foi rodado na pasta atual.
- `Ctrl+R`: cartão de busca no histórico, com setas e Enter, no lugar do `Ctrl+R` do shell em blocos.
- Sugestão de opções e subcomandos para mais programas (git, cargo, docker, apt, systemctl, ssh).
- Pronto quando: o ranking e a busca no histórico têm testes no `ox-suggest`; a sugestão aparece sem atrasar a digitação (medida pelo medidor de latência); `Ctrl+R` acha um comando antigo digitando um pedaço dele.

### M3. Notas executáveis mais ricas
- Variáveis: `{{nome}}` num bloco pede o valor antes de rodar (lembra o último usado).
- "Rodar todos": executa os blocos da nota em sequência e para no primeiro que falhar.
- Saída guardada: a última saída de cada bloco fica na nota, recolhida, com data e código de saída.
- Pronto quando: o analisador de notas (`ox-markdown`) reconhece variáveis e saídas e tem testes; uma nota de exemplo com variáveis e três blocos roda do começo ao fim e o resultado fica salvo no arquivo `.md`.

### M4. Latência comparada
- Um script no repositório que digita teclas e mede o tempo até a tela mudar, igual para todos os terminais (em uma tela virtual, para não depender do monitor).
- Instalar para a medição: kitty, alacritty, foot, wezterm, xterm e o terminal do COSMIC, e anotar versões.
- Pronto quando: `docs/` tem a tabela com o método, as versões e os números; repetir o script dá resultados parecidos (variação anotada).

### M5. Mais formas de instalar
- `.rpm` gerado no workflow e anexado à release.
- Flatpak: manifesto e build no workflow. Atenção: dentro do Flatpak o terminal precisa abrir o shell do computador, não o da caixa; isso muda o jeito de abrir sessões e precisa de teste.
- Pronto quando: os dois instalam e abrem numa máquina limpa (Fedora para o `.rpm`); o botão "Atualizar" só aparece para AppImage e `.deb`, porque os outros se atualizam pela loja ou pelo gerenciador.

### M6. Lançamento
- Subir a versão para 1.2.0, atualizar README e a captura de tela, criar a tag, conferir os arquivos da release e as somas.
- Publicar depois uma versão seguinte pequena (1.2.1) só para provar a atualização dentro do app.
- Pronto quando: a atualização de 1.2.0 para 1.2.1 funciona no AppImage e no `.deb`.

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
