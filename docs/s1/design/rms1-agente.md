# `rms1-agente`: quem cumpre no corpo o pedido da estação

Status: escrito e provado no Mac, contra o `rms1d` de verdade com o corpo falso; ainda não rodou no robô nem no simulador. Data: 2026-11-18.

**Em resumo:**
- A estação Hailo entende o que foi pedido e valida a ferramenta e os parâmetros. O agente é quem faz o corpo cumprir: recebe o pedido validado, comanda o `rms1d`, e diz como terminou.
- É a primeira forma: quatro habilidades (parar, mover, olhar, procurar), uma por vez, tiradas de uma tabela em arquivo.
- O agente não arma o robô, e a parada nunca é barrada.
- Entre o agente e quem pilota pela página vale o que já vale no `rms1d`: o último pedido vence. A tabela de fontes e prioridades é outra peça, e não está aqui.

## O caminho

```
estação Hailo ── pedido validado (tipo 22) ──▶ rms1-mediad ── s1.station.request ──▶ rms1-agente ── robot.move, robot.stop, s1.gimbal ──▶ rms1d ──▶ corpo
estação Hailo ◀── desfecho (tipo 23) ───────── rms1-mediad ◀── s1.station.outcome ── rms1-agente ◀── robot.state ─────────────────────── rms1d
                                               rms1-mediad ── media.detections ───▶ rms1-agente
```

O agente fala com dois sockets locais, e com nada pela rede:

- **O do `rms1-mediad`** (`[midia] socket`): assina `s1.media.subscribe {"requests": true, "detections": true}`. Recebe `s1.station.request {"request", "tool", "params"}`, cada pedido que a estação manda cumprir, venha ele da voz do robô ou do painel dela; e `media.detections`, as caixas do detector. Manda `s1.station.outcome {"request", "tool", "outcome", "reason"}`, que o `rms1-mediad` leva de volta à estação.
- **O do `rms1d`:** assina `robot.subscribe {"hz": 20}` e lê de `robot.state` se o robô está armado (`policy` diferente de `held`) e onde o corpo diz que está (`odom.position`). Manda `robot.move` e `s1.gimbal` como notificação, e `robot.stop` como requisição: o `rms1d` não aplica a parada mandada como notificação (`rms1d/src/respostas.rs`, `aplicar_intencao`).

Cada ligação religa sozinha. O `rms1-mediad` e a estação: [`rms1-mediad.md`](rms1-mediad.md).

## As regras

- **Uma habilidade por vez.** Um pedido novo para o corpo, encerra a anterior como interrompida, e começa.
- **Parar nunca é barrado.** Sai mesmo sem o `rms1d` responder ou com o robô desarmado, e encerra a habilidade em curso.
- **O agente não arma.** Com o robô desarmado, o pedido é recusado; desarmado no meio, a habilidade é largada.
- **Toda habilidade termina, e diz como:** `cumprida`, `vencida` (o teto da tabela), `interrompida` ou `recusada`, com o motivo em texto.
- **O pedido contínuo é renovado** a cada `renovar_ms` (100). O `rms1d` zera o corpo sem pedido novo por `[seguranca] prazo_ms` (500): se o agente cair, o robô para sozinho. Ao subir, o agente confere que cabem duas renovações no prazo e que a velocidade da tabela cabe no corpo, e não sobe se não couberem.
- **Sem o estado do corpo por `corpo_vale_ms` (1000),** o `rms1d` saiu: o pedido é recusado, e a habilidade em curso é largada.
- **A estação pode mandar largar um pedido** (ela o interrompeu, ou o teto dela venceu): se é o que está em curso, o corpo para, e ele termina interrompido.
- **Sem agente ligado,** o `rms1-mediad` recusa o pedido na hora, e a estação não fica à espera.

## As habilidades

A tabela é `rms1-agente/habilidades.toml`, que vem dentro do programa; `--habilidades <arquivo>` usa outra. Os nomes das ferramentas e dos parâmetros são os da estação. **Os números são de partida: nenhum foi medido no robô.**

| Ferramenta | O que o agente pede | Quando termina |
|---|---|---|
| `stop` | `robot.stop` | na hora, cumprida |
| `move` (`direction`, `meters`) | `robot.move` a 0,3 m/s na direção pedida (x à frente, y à esquerda) | cumprida ao andar a distância, pela posição que o corpo diz; sem posição (ela não muda), pelo tempo da distância; vencida no teto, o tempo da distância vezes 2, mais 1 s |
| `look_at` (`target`) | `s1.gimbal` com a velocidade proporcional à distância do alvo ao meio do quadro (60 graus por segundo na borda) | cumprida com o alvo a menos de 0,08 do quadro do meio por 3 detecções seguidas; vencida em 8 s |
| `search` (`target`) | `s1.gimbal` a 40 graus por segundo, 2 s para cada lado de onde a torre começou | cumprida quando o alvo aparece numa detecção; vencida em 20 s |

**O alvo** é texto livre ("me", "the red ball"). Uma caixa é do alvo se o rótulo dela tem uma palavra do pedido ("ball" casa com "sports ball"), ou se uma palavra do pedido quer dizer aquele rótulo pela tabela `[alvos]` ("me" é `person`). A caixa sem rótulo é a do detector de patos, e vale como `duck`.

## O que esta forma não faz, dito

- **Não sabe quais rótulos o detector tem:** o alvo que nenhum detector reconhece não é recusado de saída, e vence no teto.
- **O casamento por palavra erra em rótulos compostos:** "dog" casa com "hot dog", e "bear" com "teddy bear".
- **Olhar não segue o alvo depois de centrar,** e procurar só gira a torre, sem o corpo.
- **Não há prioridade entre o agente e quem pilota:** os dois pedem ao `rms1d`, e o último pedido vence.
- **Luzes e tiro** não são habilidades: o `rms1d` ainda não os serve.
- **Quem sobe o agente no robô e no simulador** ainda não foi escrito: hoje ele se roda à mão (`rms1-agente --config rms1d.toml`).

## As peças

| Arquivo | O que é |
|---|---|
| `rms1-agente/habilidades.toml` | a tabela das habilidades e dos alvos |
| `rms1-agente/src/tabela.rs` | lê e confere a tabela; chave desconhecida é recusada pelo nome |
| `rms1-agente/src/agente.rs` | o núcleo, sem rede: entram o pedido, o estado do corpo e as detecções; saem os pedidos ao `rms1d` e o desfecho |
| `rms1-agente/src/ligacoes.rs` | os dois sockets, e o tique de 20 ms |
| `rms1-agente/src/main.rs` | as opções: `--config`, `--robo`, `--midia`, `--habilidades`, e `--conferir`, que só lê a tabela e diz as habilidades |

## Os testes

`cargo test -p rms1-agente`:

- `tests/agente.rs`: o núcleo. A tabela vale e a errada é recusada pelo nome; mover pela posição e pelo tempo, e o que não chega vence; a parada nunca é barrada; o que não pode ser cumprido é recusado sem mexer no corpo; o pedido novo encerra o anterior; o corpo desarmado ou mudo interrompe; olhar e procurar.
- `tests/ligado.rs`: o agente entre um `rms1d` e um `rms1-mediad` de mentira, nos sockets.
- `tests/com_o_rms1d.rs`: o agente contra o servidor e o laço de verdade do `rms1d`, com o corpo falso. Desarmado, recusa; armado por um cliente, o laço vê o pedido de andar, que segue valendo além do prazo de 500 ms; a parada chega ao laço.

Do lado do `rms1-mediad`: `tests/percepcao.rs` (o pedido vai a quem assina, o desfecho volta, e sem agente a recusa volta na hora) e `tests/servico.rs` (a assinatura `requests` e `s1.station.outcome`).
