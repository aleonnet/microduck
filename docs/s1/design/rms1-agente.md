# `rms1-agente`: quem cumpre no corpo o pedido da estação

Status: escrito e provado no Mac, contra o `rms1d` de verdade com o corpo falso e a estação de verdade; sobe com o simulador e com a ativação do robô desde 2026-11-20, e a prova nos dois é do dono. Data: 2026-11-20.

**Em resumo:**
- A estação Hailo entende o que foi pedido e valida a ferramenta e os parâmetros. O agente é quem faz o corpo cumprir: recebe o pedido validado, comanda o `rms1d`, e diz como terminou.
- É a primeira forma: quatro habilidades (parar, mover, olhar, procurar), uma por vez, tiradas de uma tabela em arquivo.
- O agente não arma o robô, e a parada nunca é barrada.
- Entre o agente e quem pilota manda a tabela de fontes do `rms1d` ([`rms1d.md`](rms1d.md)): o agente é a linha `agente`, abaixo do piloto. Quando o piloto pega os comandos, o corpo é dele na hora, e o agente encerra o pedido dizendo quem assumiu.
- Quem o sobe: `scripts/rms1`, pelo serviço local do laboratório, no simulador; e a ativação (`scripts/s1/rms1d-bench.sh up`), no robô.

## O caminho

```
estação Hailo ── pedido validado (tipo 22) ──▶ rms1-mediad ── s1.station.request ──▶ rms1-agente ── robot.move, robot.stop, s1.gimbal ──▶ rms1d ──▶ corpo
estação Hailo ◀── desfecho (tipo 23) ───────── rms1-mediad ◀── s1.station.outcome ── rms1-agente ◀── robot.state, s1.state ────────────── rms1d
                                               rms1-mediad ── media.detections ───▶ rms1-agente
```

O agente fala com dois sockets locais, e com nada pela rede:

- **O do `rms1-mediad`** (`[midia] socket`): assina `s1.media.subscribe {"requests": true, "detections": true}`. Recebe `s1.station.request {"request", "tool", "params"}`, cada pedido que a estação manda cumprir, venha ele da voz do robô ou do painel dela; e `media.detections`, as caixas do detector. Manda `s1.station.outcome {"request", "tool", "outcome", "reason"}`, que o `rms1-mediad` leva de volta à estação.
- **O do `rms1d`:** ao ligar, três linhas, nesta ordem. `hello {"api_version", "client": "agente"}`, que põe o que vem depois na linha `agente` da tabela de fontes; `robot.subscribe {"hz": 20}`, de onde lê se o robô está armado (`policy` diferente de `held`) e onde o corpo diz que está (`odom.position`); e `s1.subscribe {"hz": 20}`, de onde lê, no bloco `control`, quem tem as rodas e a torre e qual linha da tabela ele mesmo é (`client`). Manda `robot.move` e `s1.gimbal` como notificação, e `robot.stop` como requisição: o `rms1d` não aplica a parada mandada como notificação (`rms1d/src/respostas.rs`, `aplicar_intencao`).

Cada ligação religa sozinha. O `rms1-mediad` e a estação: [`rms1-mediad.md`](rms1-mediad.md).

## As regras

- **Uma habilidade por vez.** Um pedido novo encerra a anterior como interrompida, e começa.
- **Parar nunca é barrado.** Sai mesmo sem o `rms1d` responder ou com o robô desarmado, e encerra a habilidade em curso.
- **`robot.stop` é só da ferramenta de parar.** No `rms1d` a parada apaga os pedidos de todas as fontes, inclusive o de quem pilota. Para arrumar a casa no fim de uma habilidade (cumprida, vencida, largada, trocada, corpo desarmado ou mudo), o agente solta só o canal dele: quem andava manda `robot.move` a zero, e quem girava a torre manda `s1.gimbal` com `stop`. Até 2026-11-20 todo fim de habilidade mandava `robot.stop`.
- **Outra fonte assumiu o canal: a habilidade termina, sem mexer no corpo.** Com o canal da habilidade em curso (as rodas, para mover; a torre, para olhar e procurar) na mão de outra fonte por mais de `perdeu_apos_ms` (300, três renovações), ela termina `interrompida`, com o motivo dizendo quem assumiu ("o piloto assumiu as rodas"), e não retoma sozinha. Ao desistir o agente zera a linha dele (`robot.move` a zero, ou `s1.gimbal` com `stop`), como em todo fim de habilidade: no `rms1d` o pedido de uma fonte superada fica guardado, e o último que o agente renovou passaria a valer quando a vez da outra vencesse, com o pedido já dado por interrompido (o robô andava sozinho por até 0,4 s, meio segundo depois de uma parada do piloto; o leitor frio do diff achou, e `tests/fontes.rs` reproduz). O zero na linha do agente não mexe no que quem assumiu pede. Por isso `perdeu_apos_ms`, mais uma renovação, tem de caber em `[seguranca] prazo_ms`: o agente não sobe se não couber. Um instante com outra fonte não conta: o prazo recomeça quando o canal volta ao agente, ou a ninguém. O dono é comparado com o `client` que o `rms1d` diz, e não com o nome `agente` suposto: num `rms1d` cuja tabela não tem essa linha o agente é a fonte sem nome, e não se dá por tomado por ele mesmo.
- **O agente não arma.** Com o robô desarmado, o pedido é recusado; desarmado no meio, a habilidade é largada.
- **Toda habilidade termina, e diz como:** `cumprida`, `vencida` (o teto da tabela), `interrompida` ou `recusada`, com o motivo em texto.
- **O pedido contínuo é renovado** a cada `renovar_ms` (100). O `rms1d` zera o corpo sem pedido novo por `[seguranca] prazo_ms` (500): se o agente cair, o robô para sozinho. Ao subir, o agente confere que cabem duas renovações no prazo e que a velocidade da tabela cabe no corpo, e não sobe se não couberem.
- **Sem o estado do corpo por `corpo_vale_ms` (1000),** o `rms1d` saiu: o pedido é recusado, e a habilidade em curso é largada.
- **A estação pode mandar largar um pedido** (ela o interrompeu, ou o teto dela venceu): se é o que está em curso, o agente solta o canal dele, e ele termina interrompido.
- **Sem agente ligado,** o `rms1-mediad` recusa o pedido na hora, e a estação não fica à espera.

## As habilidades

A tabela é `rms1-agente/habilidades.toml`, que vem dentro do programa; `--habilidades <arquivo>` usa outra. Os nomes das ferramentas e dos parâmetros são os da estação. **Os números são de partida: nenhum foi medido no robô.**

| Ferramenta | O que o agente pede | Quando termina |
|---|---|---|
| `stop` | `robot.stop` | na hora, cumprida |
| `move` (`direction`, `meters`) | `robot.move` a 0,3 m/s na direção pedida (x à frente, y à esquerda); ao terminar, `robot.move` a zero | cumprida ao andar a distância, pela posição que o corpo diz; sem posição (ela não muda), pelo tempo da distância; vencida no teto, o tempo da distância vezes 2, mais 1 s; interrompida quando outra fonte fica com as rodas por 0,3 s |
| `look_at` (`target`) | `s1.gimbal` com a velocidade proporcional à distância do alvo ao meio do quadro (60 graus por segundo na borda) | cumprida com o alvo a menos de 0,08 do quadro do meio por 3 detecções seguidas; vencida em 8 s |
| `search` (`target`) | `s1.gimbal` a 40 graus por segundo, 2 s para cada lado de onde a torre começou | cumprida quando o alvo aparece numa detecção; vencida em 20 s |

**O alvo** é texto livre ("me", "the red ball"). Uma caixa é do alvo se o rótulo dela tem uma palavra do pedido ("ball" casa com "sports ball"), ou se uma palavra do pedido quer dizer aquele rótulo pela tabela `[alvos]` ("me" é `person`). A caixa sem rótulo é a do detector de patos, e vale como `duck`.

## O que esta forma não faz, dito

- **Não sabe quais rótulos o detector tem:** o alvo que nenhum detector reconhece não é recusado de saída, e vence no teto.
- **O casamento por palavra erra em rótulos compostos:** "dog" casa com "hot dog", e "bear" com "teddy bear".
- **Olhar não segue o alvo depois de centrar,** e procurar só gira a torre, sem o corpo.
- **Não retoma o pedido que o piloto interrompeu:** quem pede de novo é a estação. E um pedido feito com o piloto ainda dono das rodas (até meio segundo depois de ele soltar) termina interrompido do mesmo jeito.
- **Confere a renovação contra `[seguranca] prazo_ms`, e não contra o `prazo_ms` da linha `agente`** de `[[fontes]]`, que é o que vale para ele se a linha tiver um próprio (`src/main.rs`). Com a tabela padrão, que não tem, dá no mesmo.
- **Não confere, ao subir, que `rms1d.toml` tem a linha `agente`:** sem ela ele é a fonte sem nome, perde para todas, e não distingue outro cliente sem nome de si.
- **Luzes e tiro** não são habilidades: o `rms1d` ainda não os serve.

## As peças

| Arquivo | O que é |
|---|---|
| `rms1-agente/habilidades.toml` | a tabela das habilidades e dos alvos |
| `rms1-agente/src/tabela.rs` | lê e confere a tabela; chave desconhecida é recusada pelo nome |
| `rms1-agente/src/agente.rs` | o núcleo, sem rede: entram o pedido, o estado do corpo e as detecções; saem os pedidos ao `rms1d` e o desfecho |
| `rms1-agente/src/ligacoes.rs` | os dois sockets, o que o agente manda ao ligar (o `hello` primeiro), e o tique de 20 ms |
| `rms1-agente/src/main.rs` | as opções: `--config`, `--robo`, `--midia`, `--habilidades`, e `--conferir`, que só lê a tabela e diz as habilidades |

## Os testes

`cargo test -p rms1-agente`:

- `tests/agente.rs`: o núcleo. A tabela vale e a errada é recusada pelo nome; mover pela posição e pelo tempo, e o que não chega vence; a parada nunca é barrada; o que não pode ser cumprido é recusado sem mexer no corpo; o pedido novo encerra o anterior; o corpo desarmado ou mudo interrompe; olhar e procurar.
- `tests/ligado.rs`: o agente entre um `rms1d` e um `rms1-mediad` de mentira, nos sockets.
- `tests/fontes.rs`: a tabela de fontes. O piloto assume as rodas, e a habilidade termina dizendo quem assumiu, soltando só a linha do agente; com o `rms1d` de verdade e a tabela de produção, depois da parada do piloto o corpo segue parado quando a vez dele vence; um instante com outra fonte não interrompe; cada habilidade olha o canal dela; e, com os sockets, o `hello` vem primeiro e o dono das rodas é lido do `s1.state`.
- `tests/com_o_rms1d.rs`: o agente contra o servidor e o laço de verdade do `rms1d`, com o corpo falso e sem a linha `agente` na tabela. Desarmado, recusa; armado por um cliente, o laço vê o pedido de andar, que segue valendo além do prazo de 500 ms; a parada chega ao laço.

Do lado do `rms1-mediad`: `tests/percepcao.rs` (o pedido vai a quem assina, o desfecho volta, e sem agente a recusa volta na hora) e `tests/servico.rs` (a assinatura `requests` e `s1.station.outcome`).
