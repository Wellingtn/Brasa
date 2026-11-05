<div align="center">

<img src="branding/brasa-wordmark.png" alt="Brasa" width="420">

**Terminal desktop em Rust com blocos de comando, notas executáveis, gerenciador de arquivos e monitor de desempenho, feito para Linux.**

![GNOME](https://img.shields.io/badge/GNOME-46-4a86cf?style=flat-square&logo=gnome&logoColor=white)
![Versão](https://img.shields.io/badge/versão-0.1.0-e8805f?style=flat-square)
![Rust](https://img.shields.io/badge/Rust-GPUI-dea584?style=flat-square&logo=rust&logoColor=black)
![Licença](https://img.shields.io/badge/licença-AGPL--3.0-e9b25f?style=flat-square)

</div>

Terminal, notas e sessões num app só. O nome é o prompt aceso, e o tema segue a brasa: fundos em carvão, coral (Sessões e o que está rodando) e âmbar (Notas, foco e seleção). A brasa do logo, no topo da barra lateral, tremeluz enquanto um comando roda.

A marca fica em `branding/` (`brasa-icon.svg`, `brasa-wordmark.svg` e o ícone em PNG de 16 a 512 px).
Antes chamava Oxiterm: na primeira vez, as notas e pastas de `~/.config/oxiterm` são trazidas para
`~/.config/brasa`, e as variáveis `OXITERM_*` continuam valendo como `BRASA_*`.


```
crates/ox-pty        PTY (portable-pty): sessões, run_command, cwd do shell via /proc
crates/ox-markdown   Markdown -> blocos executáveis; NoteStore: notas em pastas, exclusão vai para .lixeira/
crates/ox-files      listagem, árvore de arquivos preguiçosa (FileTree), pré-visualização, branch git
crates/ox-editor     buffer de texto (cursor, seleção, undo, quebra de linha) do editor de notas
crates/ox-suggest    Engine + trait SuggestionProvider (histórico, specs de subcomandos, caminhos)
apps/brasa         UI GPUI: janela, barra lateral, terminal em blocos, editor de notas
vendor/xattr         patch de 1 linha (libc::ENOATTR não existe no Linux em libc recentes)
```

```bash
cargo test            # crates leves, sem compilar o GPUI
cargo run -p brasa  # app
```

## Atalhos

| Atalho | Ação |
| --- | --- |
| Ctrl+Shift+T / W | nova aba / fechar aba |
| Ctrl+Shift+←/→ | trocar de aba |
| Ctrl+Shift+C / V | copiar seleção / colar (clique direito também cola) |
| Ctrl+= / Ctrl+- / Ctrl+0 | zoom da fonte |
| Ctrl+Shift+F | buscar no histórico (Enter sobe, Shift+Enter desce, Esc fecha) |
| Ctrl+Shift+E / N | mostrar ou ocultar a barra lateral / o painel de notas |
| → | aceitar a sugestão de autocomplete |

Apps TUI (vim, htop, tmux) recebem o mouse; segure Shift para selecionar texto do terminal. Duplo clique seleciona uma palavra; triplo, a linha. Notas ficam em `~/.config/brasa/notes`.

## Interface

- **Janela:** barra de título própria com os botões padrão de minimizar, maximizar e fechar à direita (como em apps de Linux e Windows), arrastar para mover, duplo clique para maximizar e bordas para redimensionar.
- **Cabeçalho:** botões de dividir (lado a lado / embaixo), o indicador de CPU (lido de `/proc/stat` a cada segundo; clique abre o gráfico) e os botões de janela.
- **Vários terminais na tela:** dividir abre um terminal novo ao lado ou embaixo do painel em foco (Ctrl+Shift+D / Ctrl+Shift+B, ou os botões no cabeçalho de cada painel). Clicar num painel põe o teclado nele; a roda do mouse rola o painel sob o cursor. O ✕ do painel tira o terminal da tela, mas ele continua aberto em Sessões (Ctrl+Shift+X faz o mesmo no painel em foco).
- **Larguras ajustáveis:** arraste a borda da barra lateral, a borda do painel de notas ou a divisória entre painéis (aparece uma faixa lilás ao passar o mouse).
- **Desempenho (desenho do Wave):** a barra de widgets à direita abre Arquivos, CPU, Memória e Processos, cada um no seu painel (clicar de novo fecha; os de desempenho se empilham numa coluna à direita). CPU e Memória: gráfico de linha dos últimos 2 minutos, com eixo de 0 a 100 % e marcas de tempo. Processos (Ctrl+Shift+M): atualiza a cada 2 s (o botão ⟳ alterna 1, 2 ou 5 s) e fica parada ("pausado") com o mouse sobre a tabela ou o menu de um processo aberto, para dar tempo de clicar; resumo com carga, memória, núcleos e total; lupa para filtrar por nome, PID ou usuário; clique no título da coluna para ordenar; clique direito num processo para copiar o PID ou enviar SIGINT, SIGHUP, SIGTERM ou SIGKILL. O indicador de CPU do cabeçalho abre o gráfico de CPU.
- **Arquivos (desenho do Wave):** gerenciador em painel, aberto na pasta do terminal em foco. Tabela com Nome, Permissões, Modificado (hora local), Tamanho e Tipo; clique no título para ordenar (pastas sempre primeiro). Duplo clique entra na pasta ou abre o arquivo (imagens no visualizador do próprio painel, com + / − / ⟲ e zoom pela roda; texto e código no editor). ".." sobe um nível; ← volta; o olho mostra/esconde ocultos; ⟳ recarrega. Com o painel em foco: ↑/↓, Enter e Backspace.
- **Painel em foco:** com mais de um painel, o selecionado ganha contorno na cor da seção ativa da barra lateral (coral em Sessões, lilás em Notas).
- **Completar com Tab:** no terminal em blocos, Tab depois de um comando (ex.: `cd `, `cat R`) abre um cartão com as pastas e arquivos que combinam; ↑/↓ ou Tab escolhem, Enter completa, Esc fecha. Com uma opção só, completa direto; sem nenhuma, o Tab vai para o shell.
- **Notas:** digitar ```` ```python ```` e Enter cria o fechamento do bloco sozinho.
- **Terminal em blocos:** cada comando vira um cartão rolável. O cabeçalho mostra o status (✔ / ✘ com o código de saída), `pasta $ comando`, branch, hora e as ações "Criar Nota" e "Copiar" (✕ fecha); clique no cabeçalho para recolher. A barra de entrada fica fixa embaixo, de ponta a ponta, com o badge `shift + tab (autocompletar)` e uma linha de atalhos. Programas de tela cheia (vim, htop) e `BRASA_PROMPT=native` usam a grade crua.
- **Barra lateral:** um seletor no topo alterna entre *Sessões* e *Notas* (os arquivos ficam no painel Arquivos, na barra da direita). *Sessões* (coral; botões "Novo Terminal" e "Nova Pasta"; duplo clique num terminal ou o lápis dá um nome a ele (vazio volta a mostrar o último comando); arraste terminais para dentro de pastas ou para "Todas as sessões"; duplo clique ou o lápis renomeia, a lixeira remove a pasta com confirmação e os terminais voltam para a lista; as pastas ficam salvas em `~/.config/brasa/session-folders.txt` e voltam ao reabrir o app, vazias; ✳ = comando rodando); *Notas* (lilás; árvore de pastas com os botões "Nova Nota" e "Nova Pasta"; arraste notas e pastas para dentro de outra pasta, ou para "Todas as notas"; duplo clique ou o lápis renomeia a pasta). Embaixo, filtro e perfil. A lixeira pede confirmação e move para `.lixeira/` (nada é apagado de verdade).
- **Blocos de código nas notas:** cartão com a linguagem num seletor (Code, Shell, Python…), copiar e `>_` para executar no terminal; as cercas ``` ficam sempre escondidas (o cursor pula por cima delas; só a abertura que você está digitando aparece); a lixeira no rodapé ou Backspace num bloco vazio apaga o bloco. O menu `/` busca blocos e notas (inserir uma nota cria um link para ela). O código ganha cores conforme a linguagem do bloco (Shell, Python, Rust, JavaScript/TypeScript, JSON); arquivos abertos no painel usam a extensão. Digitar ```` ``` ```` numa linha (no teclado ABNT2: crase + espaço, três vezes) já cria o bloco; Backspace no começo de um bloco com texto desfaz o bloco e mantém o texto.
- **Formatação ao vivo nas notas:** negrito (`**…**`), itálico (`*…*`), riscado (`~~…~~`), código (`` `…` ``) e links (`[texto](url)`) aparecem com o efeito já no modo edição, com os símbolos esmaecidos; no modo leitura, só o resultado.
- **Janela:** a barra lateral é levemente translúcida; o desfoque por trás depende do compositor (KDE/Wayland). Zoom (Ctrl+= / Ctrl+-) e o painel de notas (Ctrl+Shift+N) ficam só no teclado.
- **Notas:** painel sem divisórias ao lado da sidebar, com cabeçalho de painel (título centralizado), linha de detalhes (caminho · Editando ✎ / Visualizando), **título editável** acima do conteúdo (em negrito, 1,4× maior; editar renomeia o arquivo) e corpo centralizado com largura máxima de 640 px. Digite `/` numa linha vazia para o menu de blocos; blocos `bash` têm ▶ Executar dentro do editor.

| Atalho (editor focado) | Ação |
| --- | --- |
| Ctrl+Z / Ctrl+Y | desfazer / refazer |
| Ctrl+B / Ctrl+I | negrito / itálico |
| Ctrl+A / C / X / V | selecionar tudo / copiar / recortar / colar |
| Ctrl+←/→, Ctrl+Backspace | andar / apagar por palavra |
| Tab / Shift+Tab | recuar / desfazer recuo |
| Enter | continua listas; em item vazio sai da lista |
| Ctrl+S / Esc | salvar agora / voltar à leitura (salva sozinho em 0,8 s) |

## Arquivos no painel

Clicar num arquivo na aba *Arquivos* abre no painel, como uma nota: código com números de linha, **▶ Rodar** (Python, shell, Node, Ruby, Go, Perl, PHP, Lua, Java, R, TypeScript via `npx tsx`), **Salvar** (Ctrl+S; arquivos não salvam sozinhos), desfazer/refazer e renomear pelo título. Tabulações e fim de linha CRLF são preservados. Imagens abrem num visualizador.

## Prompt do shell

Como no Warp, o caminho **substitui o prompt** no próprio lugar: a linha `user@host:~/pasta$` vira chips (venv, pasta, branch git) e o que você digita fica na linha de baixo, com um placeholder cinza quando está vazia. Cada comando antigo mantém o chip da pasta em que rodou.

Isso funciona porque o Brasa inicia bash, zsh e fish com um prompt próprio de duas linhas (seu `.bashrc`/`.zshrc` carrega antes) cuja primeira linha traz marcadores invisíveis que a interface reconhece. Para mudar:

```bash
BRASA_PROMPT=native cargo run -p brasa   # mantém o prompt original do shell (sem chips)
BRASA_PROMPT=">" cargo run -p brasa      # outro símbolo na linha de digitação (vazio = nenhum)
```

## Verificação visual (opcional)

`scripts/visual-check.sh [saida.png]` roda o app dentro de um Xephyr (tela virtual isolada, não mexe no seu desktop) e salva uma captura. `BRASA_STARTUP` (comandos, um por linha), `BRASA_OPEN_NOTE` (nota aberta no início) e `BRASA_OPEN_FILE` (arquivo aberto no painel) permitem montar a cena; `scripts/xdo.py` clica e digita só nesse display. Precisa de `Xephyr` e do `import` do ImageMagick.

## Referências de design e licença

O visual das notas (menu `/`, botão "+" na linha vazia, barra flutuante de formatação) e da árvore de arquivos foi inspirado no comportamento e na aparência do [Warp](https://github.com/warpdotdev/warp). **Nenhum código do Warp foi copiado**: o cliente dele é AGPL-3.0, e copiar trechos obrigaria este projeto (MIT) a adotar a mesma licença. Só comportamento e aparência foram usados como referência.

## Extensões de autocomplete

Implemente `ox_suggest::SuggestionProvider` e registre com `Engine::register`.

## Limitações conhecidas

- O autocomplete deduz a linha pelas teclas digitadas (sem integração completa com o shell). Os blocos dependem do prompt do Brasa: com `BRASA_PROMPT=native` não há blocos.
- Blocos: o horário é o de quando o prompt apareceu (não há duração) e o status de um comando só aparece quando o próximo prompt é desenhado.
- O prompt em chips foi testado com bash; zsh e fish usam scripts equivalentes que só foram validados como texto.
- O editor de notas usa fonte monoespaçada em tudo (títulos só escalam), sem busca/substituição nem IME; caracteres de largura dupla (CJK) desalinham o cursor.
- Janela com decoração própria (Wayland/X11 via GPUI): cantos arredondados dependem de o compositor suportar transparência.
- Não há renomear/mover notas nem arrastar e soltar entre pastas.
- A pasta do shell vem de `/proc`, então só funciona no Linux.
- Mouse reporting e busca estão cobertos por testes unitários, mas não foram exercitados manualmente em vim/htop.
