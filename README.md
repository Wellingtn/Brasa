<div align="center">

<img src="branding/brasa-wordmark.png" alt="Brasa" width="420">

**Um terminal que também escreve.** Comandos em blocos, notas que rodam, arquivos e desempenho no mesmo lugar.

![Linux](https://img.shields.io/badge/Linux-desktop-4a86cf?style=flat-square&logo=linux&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-GPUI-dea584?style=flat-square&logo=rust&logoColor=black)
![Licença](https://img.shields.io/badge/licença-AGPL--3.0-e9b25f?style=flat-square)

<img src="docs/captura.png" alt="Brasa com uma nota, um terminal e o painel de CPU" width="860">

</div>

## Por que Brasa

- **Cada comando é um bloco.** Você vê o que rodou, quanto tempo levou, se deu certo, e copia só a saída de que precisa.
- **Notas que executam.** Escreva em Markdown e rode um bloco de código direto no terminal.
- **Sessões organizadas.** Dê nome aos terminais, agrupe em pastas e divida a tela lado a lado.
- **Arquivos e desempenho ao lado.** Gerenciador de arquivos com visualizador de imagens, e monitor de CPU, memória e processos.
- **Leve.** Cerca de 120 MB de memória e 1% de CPU parado, desenhado pela placa de vídeo.

## Instalação

Você precisa de [Rust](https://rustup.rs) e de algumas bibliotecas do sistema. No Ubuntu, Pop!_OS e derivados:

```bash
sudo apt install build-essential pkg-config libxkbcommon-dev libwayland-dev libx11-xcb-dev libxcb1-dev libvulkan-dev mesa-vulkan-drivers
```

Depois, clone e rode:

```bash
git clone https://github.com/Wellingtn/brasa.git
cd brasa
cargo run -p brasa --release
```

Para instalar o executável, rode `cargo build -p brasa --release` e copie `target/release/brasa` para uma pasta no seu `PATH`.

Testado no Pop!_OS 24.04 com GNOME 46.

## Atalhos principais

| Ação | Atalho |
|---|---|
| Novo terminal | `Ctrl+Shift+T` |
| Dividir lado a lado / embaixo | `Ctrl+Shift+D` / `Ctrl+Shift+B` |
| Mostrar ou esconder a barra lateral | `Ctrl+Shift+E` |
| Mostrar ou esconder as notas | `Ctrl+Shift+N` |
| Desempenho (CPU, memória, processos) | `Ctrl+Shift+M` |
| Buscar na saída | `Ctrl+Shift+F` |
| Zoom | `Ctrl+=` / `Ctrl+-` |

## Licença

Brasa é software livre, distribuído sob a [GNU AGPL-3.0](LICENSE). Os créditos de ícones, fontes e inspirações estão em [NOTICE.md](NOTICE.md).
