# Roadmap do Brasa

Ordenado por custo-benefício: o que é mais útil e mais fácil de fazer vem primeiro.
Esforço: P = dias, M = 1–2 semanas, G = várias semanas.

## Visão geral das versões

| Versão | O que traz | Situação |
|---|---|---|
| 1.0.0 | Terminal em blocos, notas em Markdown que executam, gerenciador de arquivos, monitor de CPU/memória/processos | Publicada |
| 1.1.0 | Configurações, temas por cor e fundo, fontes, busca global (`Ctrl+K`), atalhos configuráveis, aviso de versão | Publicada |
| 1.2.x | Atualização dentro do app, histórico inteligente e `Ctrl+R`, notas com variáveis e "Rodar todos", `.rpm` e Flatpak, método de latência | Publicada (atual: 1.2.3) |
| 1.3.0 | Imagens no terminal (Kitty), SSH integrado, mais vazão | A fazer |
| 1.4.0 | MCP | A fazer |
| 1.5.0 | Editor de banco de dados (SQLite primeiro) | A fazer |
| 2.0.0 | macOS e Windows | A fazer |

## Pendências e débitos (o que ficou sem fechar)

| Item | Detalhe | Quando olhar |
|---|---|---|
| Flatpak nunca aberto | Foi construído pelo workflow e nunca instalado. Risco principal: o terminal achar o shell do computador por `flatpak-spawn --host` (só a montagem do comando tem teste) | Antes de divulgar o Flatpak |
| `.rpm` sem o app aberto | Instala num Fedora 45 limpo (contêiner) e as bibliotecas estão todas; falta abrir o app numa máquina com tela. Pacote sem assinatura | Antes de divulgar o `.rpm` |
| Atualização no AppImage | Provada só no `.deb` (1.2.2 para 1.2.3); no AppImage o código troca o próprio arquivo e ainda não foi exercitado | Na próxima versão publicada, a partir de um AppImage |
| Comparação de latência | Método e script prontos; falta medir em tela real com GPU contra kitty, alacritty, foot, wezterm | Quando houver forma de medir em tela real (ver `docs/latencia.md`) |
| Releases 1.2.0 e 1.2.1 apagadas | Tinham a atualização dentro do app quebrada (filtro de endereço sensível a maiúsculas). Quem as instalou troca à mão uma vez | Nada a fazer; só avisar se alguém perguntar |
| Captura do README | Mostra a pasta pessoal e o nome de usuário do autor num repositório público | Trocar por uma captura com pasta de exemplo |
| Pacotes sem assinatura | `.deb`, `.rpm` e AppImage só têm soma SHA-256; sem assinatura GPG | Se um dia houver repositório próprio |
| Estado de `branding/logo/` e `docs/relatorio-comparativo.md` | Ficam fora do repositório de propósito | Decidir se entram |
| Saída imediata do app numa aba do terminal do Claude | Aconteceu uma vez, sem causa achada; o AppImage abre normalmente (o problema era a permissão de execução) | Só se voltar |

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
| **M1. Atualização dentro do app** | Botão "Atualizar", verificação SHA-256, `SHA256SUMS` na release | P | Feito; provado de 1.2.2 para 1.2.3 no `.deb` (AppImage em Pendências) |
| **M2. Autocompletar e histórico inteligente** | Ranking por frequência, recência e pasta; busca no histórico (`Ctrl+R`); mais comandos com sugestão de opções | M | Feito |
| **M3. Notas executáveis mais ricas** | Variáveis, "Rodar todos" em sequência, saída guardada na nota | M | Feito |
| **M4. Latência comparada** | Método repetível, medição em outros terminais, tabela e script no repositório | P a M | Método e script prontos; comparação justa com terminais de GPU em aberto (ver `docs/latencia.md`) |
| **M5. Mais formas de instalar** | Flatpak e `.rpm` publicados pelo workflow | M | Pacotes publicados; `.rpm` instala num Fedora limpo, Flatpak não aberto (ver Pendências) |
| **M6. Lançamento** | Versão 1.2.0, README e captura de tela novos, tag, release conferida, atualização testada | P | Feito (1.2.0 a 1.2.3; 1.2.0 e 1.2.1 foram apagadas por terem a atualização quebrada) |

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

## 1.3.0 — Terminal completo (M a G)

Quatro marcos. A vazão com saída enorme ficou de fora por decisão anterior (já está em cerca de 60 MB/s, 3º de 9 no comparativo); só volta se você pedir.

| Marco | O que entra | Esforço | Situação |
|---|---|---|---|
| **M1. SSH integrado** | Lista de hosts do `~/.ssh/config`, atalho (`Ctrl+Shift+H`) e botão na barra lateral; a sessão abre já conectada e com o nome do host | M | Feito (testado contra um servidor SSH num contêiner) |
| **M2. Imagens no terminal (Kitty)** | Protocolo gráfico do Kitty (`kitty +kitten icat`, `chafa`, visualizadores): decodificar e desenhar imagens dentro do bloco | G | Feito (testado com `chafa -f kitty` e envios à mão; sem `icat` instalado) |
| **M3. Sixel (opcional)** | Segundo protocolo de imagem, reaproveitando o desenho do M2 | M | Só se sobrar tempo |
| **M4. Lançamento 1.3.0** | Versão, README, captura de tela sem dados pessoais, tag, release conferida, teste da atualização no AppImage | P | Por último |

### M1. SSH integrado
- Ler `~/.ssh/config` (Host, HostName, User, Port) e listar os hosts; atalho `Ctrl+Shift+H` e botão SSH na barra lateral; também aceita `usuario@maquina` digitado (validado, sem opções disfarçadas).
- A sessão remota é um comando `ssh` em execução como qualquer outro: o Brasa não instala nada no servidor, então a máquina remota não ganha blocos próprios nem o prompt do Brasa. Fica para depois, se valer a pena.
- Pronto quando: abre uma sessão num host de teste (um servidor SSH local no contêiner), roda comandos e fecha sem deixar processo órfão; nome do host aparece na sessão.

### M2. Imagens no terminal (Kitty)
- Interpretar as sequências do protocolo gráfico do Kitty (transmitir, mostrar, apagar) e desenhar a imagem no bloco.
- Como ficou: o crate `ox-graphics` separa a sequência do fluxo, decodifica PNG/RGB/RGBA (com zlib) e guarda as imagens; a grade ganha células reservadas que apontam para a imagem, então ela rola com o texto. O tamanho da célula em pixels vai no tamanho do terminal, para o `chafa` acertar a proporção.\n- Limites: só transmissão direta (`t=d`; ler arquivo ou memória compartilhada fica de fora de propósito); imagem até 16 milhões de pixels; 256 MB de imagens por sessão; no máximo 60 linhas de altura. Não desenha na tela cheia (vim, less); animação e recorte (`x`, `y`, `w`, `h`) não existem.
- Pronto quando: `chafa -f kitty` mostra a imagem na proporção certa e rola com o texto; imagem enorme, pedido de arquivo e lixo na sequência não travam o app (verificado).

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
