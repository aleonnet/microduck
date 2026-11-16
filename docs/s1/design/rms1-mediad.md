# `rms1-mediad`: o serviço de mídia do S1

Status: em uso no robô e no simulador desde 2026-11-15, com a estação Hailo de verdade. Data: 2026-11-14; atualizado em 2026-11-15. A ativação no robô está em [`../robot/rms1-mediad-no-robo.md`](../robot/rms1-mediad-no-robo.md).

O dono único da câmera, do microfone e do alto-falante do S1. No pato a câmera é do `mediad`, e o microfone e o som são do `robotd`. No S1 os três ficam num processo só, fora do `rms1d`, por três razões:

- a saída de som e a captura do microfone são de um cliente só;
- a trava contra a própria voz precisa saber, no mesmo lugar, o que toca e o que chega;
- uma falha de mídia não pode derrubar quem move o robô.

Nada aqui depende do `mediad` do pato, que só existe com GStreamer. O que é igual ao pato é a interface, e o que compila para o S1 é reusado: o vigia de som (`pet-detect`), a supressão das caixas (`duck-detect`) e o desenho do tocador (`robotd/src/sound.rs`).

## O caminho

```text
console, cérebro, rms1d (o som)                 estação Hailo (Pi 4), achada pelo anúncio
        │  JSON-RPC, um socket unix                    ▲ quadro reduzido      │ entregas
        ▼                                              │ TCP                  ▼
                              rms1-mediad
        ┌──────────────┬──────────────┬───────────────┐
        ▼              ▼              ▼               ▼
     câmera        microfone     alto-falante     percepção
  teste | sim | S1   arquivo | S1   aplay | nula   (segue o anúncio)
```

As fontes ficam atrás da mesma interface: a de teste, a do simulador e a do robô. Entre uma e outra só muda `[midia]` em `rms1d.toml`.

## As peças

| Arquivo | O que é |
|---|---|
| `rms1-mediad/src/quadros.rs` | O encontro de quadros: quem quer um quadro pede, e recebe o próximo que for capturado |
| `rms1-mediad/src/camera.rs` | As fontes da câmera, atrás da mesma interface: a de teste, a do simulador e a do robô |
| `rms1-mediad/src/camera_s1.rs` | A câmera do robô, aberta pelos 27 passos do `dji_camera`, com cada resposta do driver conferida com a gravada |
| `rms1-mediad/src/mixer.rs` | O mixer do chip de áudio do robô: o estado da partida da DJI, gravado e conferido, e o volume na conta da DJI |
| `rms1-mediad/src/microfone.rs` | O ouvinte: o vigia de som do pato, com a trava. As fontes: arquivo, o `arecord` do robô e o simulador |
| `rms1-mediad/src/trava.rs` | A trava contra a própria voz |
| `rms1-mediad/src/alto_falante.rs` | O tocador: um som por vez, e o `wheee` em três partes. A saída é uma interface (`aplay`, o simulador ou nula) |
| `rms1-mediad/src/reducao.rs` | O quadro que vai à estação: NV12 reduzido a RGB 640 por 360, sem moldura; e a NV12 640 por 360, para o codificador |
| `rms1-mediad/src/venc.rs` | O cliente do `rms1-venc`: uma conexão só, partilhada pela percepção e pela imagem da tela |
| `rms1-mediad/venc/rms1-venc.c` | O programa do lado Android do robô que comanda o codificador de hardware do S1 (Hantro H1): H.264 e JPEG |
| `rms1-mediad/src/estacao.rs` | O anúncio da estação e o cabeçalho das mensagens |
| `rms1-mediad/src/percepcao.rs` | A percepção pela estação: segue o anúncio, manda o quadro, lê as entregas, publica |
| `rms1-mediad/src/servidor.rs` | O socket local |
| `rms1d-params` | `[midia]` em `rms1d.toml`: o esquema, os padrões e a validação |

## O que vem do desenho do pato, e quem garante

- **Quadro pedido, e não publicado.** Copiar um quadro custa o quadro inteiro (1.382.400 bytes no S1). Quadro que ninguém pediu custa uma leitura atômica. `sem_pedido_nao_copia`.
- **O próximo, e não o último.** O pedido é respondido por uma captura feita depois dele; no prazo vencido, a resposta é erro, nunca um quadro de antes. `sem_quadro_no_prazo_e_erro`.
- **Uma captura responde a todos os que esperam.** `dois_pedidos_uma_copia`.
- **O robô não ouve a si mesmo.** Enquanto o alto-falante toca, e por `[midia] trava_ms` depois (800 ms, o do pato, até a medição no S1), o que o microfone ouve é descartado. `o_proprio_som_e_descartado`, `depois_da_trava_volta_a_ouvir`.
- **Um som por vez.** Um som novo corta o anterior; durante o `wheee`, o avulso é descartado; sem renovação por 300 ms, o `wheee` pousa. `um_som_por_vez`, `wheee_pousa_ou_corta`.
- **Ligar não toca nada.** Som só sai por `robot.sound`.
- **`<socket>.lock` garante um só dono**, antes de qualquer dispositivo ser aberto.

## O socket

Um socket unix, `[midia] socket`, em JSON-RPC, uma linha por mensagem.

| Chamada | O que faz |
|---|---|
| `hello` | A versão do contrato e a do serviço |
| `media.frame` | O próximo quadro: uma linha com o cabeçalho do contrato do pato (`MediaFrameHeader`) e, em seguida, exatamente `bytes` bytes crus. A conexão fecha depois dele |
| `robot.sound` | O do contrato do pato (`tag` e `hold`), como requisição ou notificação |
| `s1.media.subscribe` | `{"heard": true, "detections": true}`. A conexão passa a receber `s1.audio.heard {"kind"}`, já com a trava aplicada, e `media.detections`. Cada pedido diz as duas assinaturas: a que não vem é desligada |
| `s1.media.status` | O que o serviço está fazendo: quadros copiados, se há som tocando (`playing`), se o microfone está surdo pela trava (`deaf`: o som e a janela depois dele), e a percepção (`perception`), que diz também em que formato o último quadro foi (`format`: `rgb`, `h264` ou `jpeg`) e, se foi cru com a estação pedindo outro, por quê (`encoder_problem`) |
| `s1.media.preview` | `{"width": 640, "format": "jpeg", "quality": 70}`. O quadro reduzido, para quem só quer olhar (a mídia do Pilotar, no simulador): o cabeçalho de `media.frame` e os bytes. Sem `format`, vem em RGB (`format: "RGB"`); com `"jpeg"`, em JPEG (`format: "JPEG"`, qualidade de 30 a 90), pelo codificador do `mediad`; com o codificador de hardware ligado e a largura de 640, o JPEG sai do chip (a qualidade vira o nível dele, de 3 a 9), e por software se o chip não responder do pato. A redução e o JPEG são feitos aqui. A conexão fica aberta e serve o pedido seguinte |

**O som passa pelo `rms1d`.** Quem pede som fala com o `rms1d`, como no pato fala com o `robotd`: ele repassa `robot.sound` a este serviço como notificação, sem esperar, e responde aceito. Sem `[midia]` em `rms1d.toml`, ou com este serviço fora do ar, recusa com o motivo. `rms1d/tests/midia.rs`.

## A percepção pela estação Hailo

O S1 captura, a estação pensa, e o S1 publica o que ela achou. O formato do anúncio e das mensagens mora em `tools/hailo/estacao_anuncio.py`, no laboratório (`~/Development/RoboMaster-S1`); o lado do S1 é conferido contra um pacote e uma mensagem gravados por lá (`rms1-mediad/tests/estacao.rs`).

- **O S1 não lê o contrato das funções: lê o anúncio.** A estação difunde, a cada 0,5 s, na porta UDP `[midia] percepcao.porta_anuncio` (45679), que entradas quer, em que ritmo, e o que cada função ligada entrega. Vale anúncio de rede privada ou da própria máquina. Sem anúncio por 3 s, a estação saiu, e nada é mandado.
- **Com duas estações na rede, fica a que já está em uso** enquanto ela se anuncia. Uma estação é o endereço e a porta dos quadros.
- **O quadro vai reduzido e sem moldura:** RGB 640 por 360, 691.200 bytes (`percepcao.rs`, `QUADRO_LARGURA` e `QUADRO_ALTURA`). A moldura de cada modelo é posta na estação. `scripts/test-rms1-mediad-lab.sh` prova que a redução daqui, com a moldura de lá, dá os mesmos bytes do programa da medição de 2026-11-13, no tamanho daquela medição (320 por 180).
- **O formato é o que a estação prefere.** A entrada do anúncio diz `prefere` (`cru`, `h264` ou `jpeg`) e `qualidade` (a taxa do H.264 em Mb/s, com 0 valendo o máximo do chip; ou o nível do JPEG, de 0 a 10). Com `[midia] venc` no arquivo e o `rms1-venc` respondendo, o quadro é reduzido a NV12 640 por 360 (`reducao.rs`, `nv12_reduzido`), codificado pelo chip e mandado no tipo de mensagem do formato (9, H.264; 10, JPEG), um quadro por mensagem. O fluxo de H.264 começa por um quadro inteiro a cada conexão, a cada troca de taxa ou de ritmo, e quando a estação recusa algo, no máximo uma vez por segundo. **Se o codificador falha, aquele quadro vai cru,** e `s1.media.status` diz o motivo; uma estação que não diz `prefere` recebe o cru. Medido no laboratório em 2026-11-15 e 16, com 90 quadros e o `yolov11n` (`out/2026-11-14-s1-midia/adendo/`, `comparacao-transporte-taxas-altas.txt` e `notas-por-variante-taxas-altas.txt`): o chip leva de 1,5 a 3,1 ms por quadro, na mediana; o H.264 no máximo tem 78.515 bytes por quadro (9,4 Mb/s a 15 por segundo) e deixa o objeto acima da nota 0,35 em 61 dos 90 quadros, como o cru, que tem 691.200 bytes por quadro (82,9 Mb/s).
- **O ritmo é o da estação,** até o teto de `[midia] percepcao.ritmo_max` (22 por segundo). Os 2 por segundo do detector do pato são calor na placa dele; aqui o Hailo está no Pi.
- **Cada entrega vem com o canal da função.** As caixas do detector de patos (leitor `nanodet_v8`) passam pela supressão do pato (`duck_detect::decode`), voltam aos pixels do quadro da câmera e saem em `media.detections`, no formato do pato, também sem caixa nenhuma. Só a função que o anúncio marca com `publica` é publicada; as outras são contadas em `s1.media.status`.
- **A medida.** Uma vez por segundo o S1 manda à estação a ida e volta que mediu e os quadros por segundo: é o que o painel da estação mostra.
- **Conexão caída reconecta, com espera crescente.** `s1.media.status` diz por que nada está sendo mandado, quando não está.

Os testes: `rms1-mediad/tests/percepcao.rs`, com uma estação de mentira em 127.0.0.1 e, para o quadro codificado, um `rms1-venc` de mentira. O programa de verdade contra a estação de verdade, com os quadros do chip: `out/2026-11-14-s1-midia/adendo/mediad_estacao.py`, no laboratório.

## O codificador de hardware

O chip do S1 codifica H.264 e JPEG (Hantro H1), e as bibliotecas dele são do Android do robô: o `rms1-mediad`, que roda no chroot, não as carrega. Quem as chama é o `rms1-venc` (`rms1-mediad/venc/rms1-venc.c`), um programa do lado Android, compilado com o NDK por `sh rms1-mediad/venc/compila.sh` (que confere cada símbolo importado contra as bibliotecas do próprio robô). Os dois conversam por um socket de domínio Unix que os dois lados enxergam: `[midia] venc`, no robô `/run/rms1-venc/venc.sock`, que de fora do chroot é `/data/debian/run/rms1-venc/venc.sock`.

- **O protocolo** está no cabeçalho do `rms1-venc.c`: o pedido leva o comando (1, um quadro em H.264; 2, em JPEG, com o nível; 3, um fluxo novo de H.264, com a taxa e o ritmo), e o quadro NV12 de 640 por 360; a resposta, o quadro codificado ou o motivo da recusa.
- **Um cliente por vez, e um chip só:** a percepção e a imagem da tela usam a mesma conexão, um pedido de cada vez (`venc.rs`). Um pedido que não volta em 500 ms derruba a conexão, e o seguinte a refaz.
- **Como as estruturas do chip foram achadas,** e a medida de cada formato: `out/2026-11-14-s1-midia/adendo/leitura-codificador.md`, no laboratório.
- **O que falta provar no robô** (2026-11-16): o `rms1-venc` em serviço, com os dois codificadores vivos no mesmo processo. Os dois foram provados no robô um por vez, por arquivo.

## A porta do vídeo

Com `[midia] porta_video` diferente de zero (47010 no arquivo do robô), o serviço atende também uma porta TCP, em todas as interfaces, por onde só passam `hello`, `s1.media.preview` e `s1.media.status`: nada que mude o robô (`servidor.rs`, `servir_tcp`; teste `a_porta_do_video_entrega_jpeg_pela_rede_e_so_deixa_olhar`). É por ela que a tela busca a imagem, direto pela rede: pela ponte do `adb` um quadro RGB de 691 KB levava mais de 4 s (medido em 2026-11-15, `out/2026-11-14-s1-midia/medida-tela-no-robo.txt`, no laboratório). Não tem senha: quem está na rede do robô vê a câmera dele. O custo do JPEG no processador do robô e no laço não está medido (etapa `video` de `tools/simulador/prova_midia_s1.sh`, no laboratório).

## O som do robô simulado

Com `[midia.alto_falante] saida = "simulador"` e `[midia.microfone] fonte = "simulador"`, o som passa pelo serviço local do simulador (`tools/simulador/s1_servico.py`, no laboratório, porta `--som`), em duas conexões que o `rms1-mediad` abre:

- **O alto-falante** (`alto_falante.rs`, `Simulador`): o primeiro byte é `F`; depois, recados de um byte de tipo, quatro de tamanho e o conteúdo: `1` um `.wav` inteiro, `2` o começo de um fluxo (a taxa), `3` um pedaço do fluxo (PCM S16LE mono), `4` o corte. A página toca o que chega. O som dura aqui o que o cabeçalho do `.wav` diz, e é isso que mantém a trava contra a própria voz valendo no simulador.
- **O microfone** (`microfone.rs`, `rodar_simulador`): o primeiro byte é `O`; depois o serviço manda PCM cru, S16LE, mono, a 16 kHz: o que a página toca e, se o dono ligar, o microfone do Mac. Página calada não é defeito: o nível vai a zero.

Testes: `cargo test -p rms1-mediad --test som_simulado`.

## O que ficou para depois desta frente

- O que o robô faz com as caixas: ninguém age sobre elas ainda (é da frente do cérebro).
- A janela da trava contra a própria voz segue nos 0,8 s do pato. No robô, o nível do microfone voltou ao piso 0,00 s depois do fim do som, e nenhum evento passou (`out/2026-11-14-s1-midia/prova-trava.txt`, no laboratório, 2026-11-15); a janela não foi encurtada.
