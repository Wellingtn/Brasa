# Roadmap do Brasa

Ordenado por custo-benefício: o que é mais útil e mais fácil de fazer vem primeiro.
Esforço: P = dias, M = 1–2 semanas, G = várias semanas.

## Visão geral das versões

| Versão | O que traz | Situação |
|---|---|---|
| 1.0.0 | Terminal em blocos, notas em Markdown que executam, gerenciador de arquivos, monitor de CPU/memória/processos | Publicada |
| 1.1.0 | Configurações, temas por cor e fundo, fontes, busca global (`Ctrl+K`), atalhos configuráveis, aviso de versão | Publicada |
| 1.2.x | Atualização dentro do app, histórico inteligente e `Ctrl+R`, notas com variáveis e "Rodar todos", `.rpm` e Flatpak, método de latência | Publicada (1.2.3) |
| 1.3.0 | SSH integrado (`Ctrl+Shift+H`) e imagens no terminal (protocolo Kitty) | Publicada (1.3.0) |
| 1.4.0 | Banco de dados: PostgreSQL (com TLS) e SQLite, edição na grade, Query com scripts e triggers, diagrama | Publicada (1.4.0) |
| 1.5.1 | Correção: CPU parada alta (22%) depois de uma saída com mais de 10 mil linhas | Publicada (1.5.1) |
| 1.5.0 | Banco de dados, segunda parte: editor de Query completo, autocompletar com Tab, resultado sem limite de linhas, exportar, índices/triggers/funções, cancelar e histórico, grade só leitura (dados por SQL), cursor nos formulários, Sixel, correção do travamento do terminal com psql | Publicada (1.5.0) |
| 2.0.0 | macOS e Windows | A fazer |

## Pendências e débitos (o que ficou sem fechar)

| Item | Detalhe | Quando olhar |
|---|---|---|
| Flatpak nunca aberto | Foi construído pelo workflow e nunca instalado. Risco principal: o terminal achar o shell do computador por `flatpak-spawn --host` (só a montagem do comando tem teste) | Antes de divulgar o Flatpak |
| `.rpm` sem o app aberto | Instala num Fedora 45 limpo (contêiner) e as bibliotecas estão todas; falta abrir o app numa máquina com tela. Pacote sem assinatura | Antes de divulgar o `.rpm` |
| Atualização no AppImage | Provada só no `.deb` (1.2.2 para 1.2.3); no AppImage o código troca o próprio arquivo e ainda não foi exercitado. Dá para testar agora: abrir o AppImage 1.2.2 e atualizar para 1.3.0 | Agora |
| Sixel | Segundo protocolo de imagem (`img2sixel`, `chafa -f sixel`); o desenho das imagens do Kitty já serve de base | Se alguém pedir |
| Imagens na tela cheia e animadas | As imagens do Kitty não aparecem em programas de tela cheia (vim, less) e não animam; sem recorte (`x`, `y`, `w`, `h`) | Se alguém pedir |
| Editor de Query básico | Rodam scripts, funções com `$$`, triggers, transações e vários comandos (testado), mas o editor não tem cursor móvel nem seleção (só digita no fim e cola com `Ctrl+V`), nem autocompletar, histórico, várias abas, botão de cancelar (só o limite de 10 s), nem exportar resultado | 1.5.0 |
| Navegação do banco incompleta | A lista mostra tabelas e visões, mas não triggers, funções, sequências nem índices em detalhe; no PostgreSQL os valores chegam como texto (sem tipos próprios); resultado cortado em 1000 linhas | 1.5.0 |
| MySQL | Só PostgreSQL e SQLite | Se alguém pedir |
| Blocos na máquina remota | A sessão SSH é um comando em execução; o servidor não ganha blocos nem o prompt do Brasa | Se valer a pena |
| Comparação de latência | Método e script prontos; falta medir em tela real com GPU contra kitty, alacritty, foot, wezterm | Quando houver forma de medir em tela real (ver `docs/latencia.md`) |
| Releases 1.2.0 e 1.2.1 apagadas | Tinham a atualização dentro do app quebrada (filtro de endereço sensível a maiúsculas). Quem as instalou troca à mão uma vez | Nada a fazer; só avisar se alguém perguntar |
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

## 1.3.0 — Terminal completo (M a G) (feito)

Quatro marcos. A vazão com saída enorme ficou de fora por decisão anterior (já está em cerca de 60 MB/s, 3º de 9 no comparativo); só volta se você pedir.

| Marco | O que entra | Esforço | Situação |
|---|---|---|---|
| **M1. SSH integrado** | Lista de hosts do `~/.ssh/config`, atalho (`Ctrl+Shift+H`) e botão na barra lateral; a sessão abre já conectada e com o nome do host | M | Feito (testado contra um servidor SSH num contêiner) |
| **M2. Imagens no terminal (Kitty)** | Protocolo gráfico do Kitty (`kitty +kitten icat`, `chafa`, visualizadores): decodificar e desenhar imagens dentro do bloco | G | Feito (testado com `chafa -f kitty` e envios à mão; sem `icat` instalado) |
| **M3. Sixel (opcional)** | Segundo protocolo de imagem, reaproveitando o desenho do M2 | M | Não feito (ver Pendências) |
| **M4. Lançamento 1.3.0** | Versão, README, captura de tela sem dados pessoais, tag, release conferida, teste da atualização no AppImage | P | Feito, menos o teste da atualização no AppImage (ver Pendências) |

### M1. SSH integrado
- Ler `~/.ssh/config` (Host, HostName, User, Port) e listar os hosts; atalho `Ctrl+Shift+H` e botão SSH na barra lateral; também aceita `usuario@maquina` digitado (validado, sem opções disfarçadas).
- A sessão remota é um comando `ssh` em execução como qualquer outro: o Brasa não instala nada no servidor, então a máquina remota não ganha blocos próprios nem o prompt do Brasa. Fica para depois, se valer a pena.
- Pronto quando: abre uma sessão num host de teste (um servidor SSH local no contêiner), roda comandos e fecha sem deixar processo órfão; nome do host aparece na sessão.

### M2. Imagens no terminal (Kitty)
- Interpretar as sequências do protocolo gráfico do Kitty (transmitir, mostrar, apagar) e desenhar a imagem no bloco.
- Como ficou: o crate `ox-graphics` separa a sequência do fluxo, decodifica PNG/RGB/RGBA (com zlib) e guarda as imagens; a grade ganha células reservadas que apontam para a imagem, então ela rola com o texto. O tamanho da célula em pixels vai no tamanho do terminal, para o `chafa` acertar a proporção.\n- Limites: só transmissão direta (`t=d`; ler arquivo ou memória compartilhada fica de fora de propósito); imagem até 16 milhões de pixels; 256 MB de imagens por sessão; no máximo 60 linhas de altura. Não desenha na tela cheia (vim, less); animação e recorte (`x`, `y`, `w`, `h`) não existem.
- Pronto quando: `chafa -f kitty` mostra a imagem na proporção certa e rola com o texto; imagem enorme, pedido de arquivo e lixo na sequência não travam o app (verificado).

## 1.4.0 — Editor de banco de dados (G)

Dois bancos: SQLite (um arquivo, sem servidor) e PostgreSQL (o que o `psql` conecta). No lugar da tabela desenhada com `|` e `-` do terminal, uma grade moderna, a mesma para os dois. MySQL fica para depois.

| Marco | O que entra | Esforço | Situação |
|---|---|---|---|
| **M1. Motor SQLite** | Crate `ox-db`: esquema, consulta com limite e tempo máximo, edição pela chave | M | Feito (15 testes) |
| **M2. Página Banco e conector PostgreSQL** | Página que ocupa o lugar do terminal (como as Configurações), aberta pelo botão Banco da barra da direita: tabela de conexões salvas (conectar, editar, excluir, nova), lista de tabelas, abas Dados, Estrutura, Diagrama e Resultado, consulta fixa embaixo com `Ctrl+Enter`; PostgreSQL e SQLite pelo mesmo motor | G | Feito (30 testes no motor, 8 contra um PostgreSQL real; página testada com os dois bancos) |
| **M3. Edição na grade** | Editar célula (dois cliques), inserir (`+ Linha`) e apagar (`×`) com mudanças pendentes, aplicar ou descartar numa transação; sem "modo edição", como no DBeaver; conexão pode ser marcada como somente leitura | M | Feito (testado nos dois bancos e conferido no `psql`) |
| **M4. Conexões salvas e TLS** | Salvar conexões (sem a senha) e conexão remota com TLS | M | Feito junto com o M2 e os ajustes (TLS testado contra um servidor real) |
| **M5. Lançamento 1.4.0** | Versão, README, tag, release conferida | P | Feito (4 arquivos e somas conferidos) |

### M2. Painel Banco e conector PostgreSQL
- Conexões salvas em `~/.config/brasa/databases.json` (nome, tipo, servidor, porta, usuário, banco ou arquivo; modo 0600). A senha é pedida ao conectar, nunca é gravada em disco e nem existe como campo no arquivo.
- Diagrama: tabelas em colunas (quem é referenciado fica à esquerda) e setas das chaves estrangeiras, que se destacam na tabela selecionada; as tabelas podem ser arrastadas e as setas acompanham (a posição vale enquanto a página está aberta; não é gravada).
- Aberto só para leitura (`default_transaction_read_only`) e com tempo máximo por consulta (`statement_timeout`).
- Servidor na própria máquina (`localhost`, `127.0.0.1`, `::1`) conecta sem criptografia; qualquer outro **exige TLS**, com o certificado conferido (emissor, nome e validade) contra os certificados do sistema e, se a conexão tiver, contra um arquivo PEM de CA. Sem TLS, a conexão nem é tentada. Mensagens claras para CA desconhecida, nome que não bate e certificado vencido.
- Esquema pelo catálogo do PostgreSQL (`information_schema`); resultados em texto, como o `psql` mostra; consulta do editor com limite de linhas por cursor.
- Pronto quando: testes contra um PostgreSQL de verdade (contêiner) cobrem conexão, senha errada, esquema, página, query, limite, tempo máximo e só leitura; e o painel abre um SQLite e um PostgreSQL e mostra a grade.\n- Como rodar os testes contra o PostgreSQL: `BRASA_TEST_PG=127.0.0.1:5432 BRASA_TEST_PG_PASSWORD=... cargo test -p ox-db` (sem as variáveis, esses testes passam sem fazer nada).\n- Limite assumido: a proteção de só leitura é contra acidente, não contra o próprio usuário (quem digita `SET default_transaction_read_only = off` no editor a desliga); o botão de edição do M3 será o jeito explícito.

## 1.5.0 — Banco de dados, segunda parte (G)

Sai da lista de limites que a própria 1.4.0 deixou (e da sonda de SQL feita no fim dela): o editor de Query ainda é básico, o resultado não sai da tela e a navegação do banco é curta.

| Marco | O que entra | Esforço | Situação |
|---|---|---|---|
| **M1. Editor de Query de verdade** | Cursor que anda (setas, Home, End, palavra), seleção com Shift e mouse, `Ctrl+A/C/X/V`, desfazer e refazer, rolagem, `Tab` recua; reaproveita o `TextBuffer` do `ox-editor` | M | Feito (10 testes da lógica; conferido na tela) |
| **M2. Resultado que sai da tela** | Copiar célula (`Ctrl+C`) ou a grade inteira (TSV), exportar a tabela inteira ou o resultado para CSV e JSON, sem sobrescrever arquivos | M | Feito (7 testes do motor, o CSV relido pelo `sqlite3`; exportação conferida pela interface) |
| **M3. Ver mais do banco** | Aba "Objetos" com índices, triggers, funções e sequências (PostgreSQL e SQLite) e o texto da definição, que se copia com Ctrl+C | M | Feito (motor testado nos dois bancos; a aba usa a mesma grade, ainda sem conferência visual) |
| **M4. Rodar com controle** | Botão Cancelar (a query para e a conexão continua), histórico das queries por conexão (Alt+↑/↓ no editor, 100 por conexão, `db-history.json` 0600). **Ficaram de fora, com o aval do dono:** busca no histórico e várias abas de Query (candidatas à 1.6.0) | M | Feito em parte (cancelar testado nos dois bancos; sem conferência visual) |
| **A1. Cursor em todos os campos** | Campos de texto do formulário de conexão (e filtro) ganham cursor que anda (setas, Home, End, clique), seleção e colar no meio; confere de novo o editor de Query | P | Feito no formulário de conexão (setas, Home, End, Delete, inserir no meio; 1 teste); o editor de Query já tinha cursor desde o M1 |
| **A2. Edição só por SQL** | Tira a edição na grade (duplo clique, ×, + Linha, pendentes); Dados e Resultado passam a só mostrar. Quem altera dados escreve `UPDATE`/`INSERT`/`DELETE` na Query | M | Feito (a grade só mostra; `UPDATE`/`INSERT`/`DELETE` na Query, que continua protegida pelo \"somente leitura\" da conexão). Sem conferência visual |
| **A3. Resultado sem limite** | Some o corte em 1000 linhas: o resultado é lido aos poucos (cursor no servidor), a grade mostra uma janela e rola por milhões de linhas, com contagem e exportação do resultado inteiro | G | Feito: o resultado é lido em trechos de 1000 linhas (a grade rola por qualquer tamanho, 3 milhões conferidos na janela), botão Contar, e Exportar grava o resultado inteiro lido em fluxo (3 milhões de linhas em 49 MB, conferido). Scripts e INSERT continuam cortando em 1000. Falta uma barra de rolagem para saltar longe |
| **A4. Autocompletar (Tab)** | Na Query, `Tab` abre uma lista como o terminal: tabelas e visões (depois de `FROM`, `JOIN`, `INTO`, `UPDATE`), colunas (de `tabela.` ou das tabelas citadas), esquemas, funções e palavras do SQL; setas escolhem, Enter aceita, Esc fecha | G | Feito: Tab completa e abre a lista (tabelas depois de FROM/JOIN/INTO/UPDATE, `apelido.` com as colunas, colunas das tabelas do comando, funções e palavras); setas, Tab/Enter, Esc, clique; 7 testes da lógica e conferido na janela com SQLite |
| **A5. Terminal firme** | Reproduzir os travamentos (psql no Brasa, saídas enormes, programas de tela cheia), achar a causa, corrigir e deixar testes de estresse; meta: `SELECT` de milhões de linhas no psql sem travar a janela | G | Feito em parte: achado e corrigido um deadlock (escrever no PTY travava a interface quando o programa despejava saída sem ler a entrada); testes de 3 milhões de linhas e de psql com 2 milhões. Conferido na janela: `psql` com 2 milhões de linhas termina em ~6,6 s e a janela segue respondendo |
| **M5. Sixel (terminal)** | Segundo protocolo de imagem, reaproveitando o desenho do Kitty | M | Feito: decodificador (paleta, repetição, HSL, tamanho, limites) com 5 testes, separador de DCS com 2 testes, teste no terminal; conferido na janela com `img2sixel` |
| **M6. Lançamento 1.5.0** | Versão, README, tag, release conferida | P | Feito (ver Publicada na tabela de versões) |

Ordem: A5 (o mais grave) → A1 → A2 → A3 → A4; depois M5 e M6. A2 desfaz o que a 1.4.0 entregou como "edição estilo DBeaver", a pedido do dono depois de testar.

### M1. Editor de Query de verdade
- Trocar o texto que só cresce no fim por um `TextBuffer`: cursor em qualquer posição, seleção, copiar, recortar, colar, desfazer.
- Mouse: clique põe o cursor, arrastar seleciona, duplo clique escolhe a palavra.
- Pronto quando: testes do comportamento do cursor e da seleção na camada que liga o editor à tela; e a Query digitada, editada no meio e colada roda como esperado.

### M2. Resultado que sai da tela
- Copiar (`Ctrl+C` na célula ou linha escolhida) e exportar o resultado inteiro, não só a página; CSV com aspas e quebras de linha escapadas, JSON com valores nulos de verdade.
- Pronto quando: testes com valores difíceis (vírgula, aspas, quebra de linha, acentos, nulo, texto "NULL") em CSV e JSON, e um arquivo exportado que o `sqlite3` e o `psql` consigam reler.

### M3. Ver mais do banco
- PostgreSQL pelo catálogo (`pg_indexes`, `pg_trigger`, `pg_proc`, `pg_sequences`) e SQLite pelo `sqlite_master`; mostrar a definição (`CREATE ...`) para ler e copiar.
- Pronto quando: testes contra um PostgreSQL e um SQLite de verdade com índice, trigger, função e sequência criados nos testes.

### M4. Rodar com controle
- Cancelar de verdade (`pg_cancel_backend` pelo token de cancelamento, e interrupção no SQLite), sem fechar a conexão.
- Histórico em `~/.config/brasa/` por conexão, sem guardar senha nem valores digitados em campos de senha; abas de Query independentes.
- Pronto quando: teste que cancela um `pg_sleep` e um laço infinito e a conexão continua usável; histórico que sobrevive a fechar o app.

### M5. Sixel (terminal)
- Decodificar Sixel e desenhar como as imagens do Kitty (mesmos limites de tamanho e memória).
- Pronto quando: `img2sixel` e `chafa -f sixel` mostram a imagem na proporção certa.

## 2.0.0 — Outros sistemas (G)
- macOS e Windows. É a maior mudança: GPUI e o PTY precisam de ajustes em cada sistema.

## Por que essa ordem
- 1.1.0 e 1.2.0 não mexem na base e todo usuário percebe.
- 1.3.0 abre o Brasa para fluxos de trabalho mais pesados, mas toca no núcleo do terminal.
- O editor de banco de dados é o recurso que mais diferencia o Brasa, mas é grande e se apoia na configuração (1.1.0) e na busca.
- Mac e Windows ficam por último porque dobram o trabalho de teste.
