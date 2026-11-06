# Latência de teclado

Medida com `scripts/latency.py`. Este arquivo explica o método, mostra os números e diz com clareza o que
eles **não** provam.

## Como se mede

Para cada terminal, o script abre uma tela virtual nova (Xephyr, 1280×800), abre o terminal nela e:

1. descobre os pixels que mudam sozinhos com o terminal parado (cursor piscando) e os ignora;
2. manda uma tecla com XTest e anota o instante;
3. lê a tela virtual em laço e anota o instante em que aparecem pixels novos (mais de 12 fora dos ignorados);
4. repete 45 vezes, com 350 ms de pausa, e descarta as 5 primeiras (aquecimento).

A "latência" é o tempo entre mandar a tecla e ver o caractere na tela virtual. A resolução é o tempo de uma
leitura da tela (cerca de 5,6 ms nesta máquina), então diferenças menores que isso não significam nada.

Para repetir: `scripts/latency.py --trials 40 --startup 10 xterm="xterm -e bash --norc" brasa="./target/release/brasa"`
(com `--env VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json` para os terminais que desenham por GPU).
O script só age no display `:77` que ele mesmo abre e só encerra os processos que abriu.

## Resultados (máquina de desenvolvimento, Pop!_OS 24.04, 12 núcleos)

| Terminal | Versão | Mediana (ms) | Média (ms) | p95 (ms) | Mín–máx (ms) | Teclas |
|---|---|---|---|---|---|---|
| xterm | 390 | 14,2 | 14,2 | 18,9 | 9,3–19,7 | 40 |
| Brasa | 1.1.0 + M2/M3 | 59,1 | 66,6 | 97,5 | 45,2–98,7 | 40 |
| cosmic-term | do sistema | sem medida | | | | 0 |

Repetindo o xterm, a mediana ficou em 15,0 e em 14,2 ms: a medida é estável para esse caso.

## O que esses números não provam

**Não comparam um terminal que desenha na placa de vídeo com um que desenha na CPU.** A tela virtual não tem
GPU: o Brasa (e qualquer terminal que use OpenGL ou Vulkan) desenha por software (lavapipe), que é muito mais
lento que a placa. O xterm desenha direto pelo X11, na CPU, e não sofre isso. Por isso o 59 ms do Brasa aqui
mostra o custo de desenhar por software, não a latência dele no seu computador.

O cosmic-term não abriu na tela virtual (não houve nenhuma mudança de tela para medir).

Para o Brasa numa tela e placa reais, existe o medidor dentro do próprio app (Configurações > Geral >
Diagnóstico): numa sessão de uso normal ele mostrou **média de 9,2 ms e p95 de 15,9 ms**, da tecla até o início
do desenho da tela. Esse número também não é comparável com os da tabela, porque não inclui o monitor.

## O que falta para uma comparação justa

Uma comparação entre terminais com GPU exige medir na tela real, com a placa de vídeo: ou uma câmera de alta
velocidade apontada para o monitor, ou uma ferramenta que injete teclas e capture a tela pelo compositor
(no Wayland isso pede permissões que este script, de propósito, não pede). Enquanto isso não existir, esta
comparação só vale entre terminais da mesma família de desenho (todos por CPU, ou todos por GPU em software).
