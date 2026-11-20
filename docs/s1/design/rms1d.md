# `rms1d`: o serviço do corpo do S1

Status: em uso no simulador; prova no robô pendente. Data: 2026-11-03. A tabela de quem manda no corpo e o contrato 68 do pato: 2026-11-20, provados no Mac com a estação de verdade; a prova no simulador e no robô é do dono.

O que conta como pronto, pelo dono: atuar, ler e receber telemetria para controlar o S1 no mesmo nível do pato, com lista finita de pontos de controle. O teste só vale no caminho da entrega: controlar, interagir, treinar e colocar inteligência no S1.

Este serviço é novo. O `s1d` fica na árvore só como leitura e comparação: saiu da lista de pacotes em 2026-11-20, porque usa uma chamada que o contrato 68 não tem. A origem de bytes, unidades e sentidos é a biblioteca do laboratório, `~/Development/RoboMaster-S1/biblioteca`, e o firmware copiado do robô.

## O caminho

Um caminho só. Entre simulador e robô, só muda `[corpo] tipo` em `rms1d.toml`.

```text
a página (piloto), o rms1-agente, robotctl, rms1d-roteiro, e depois o cérebro
        │  JSON-RPC, um socket unix, o contrato do pato; cada um diz quem é no `hello`
        ▼
      rms1d
        │  bytes do protocolo do corpo
        ▼
  serial /dev/ttyS3  ou  TCP até a ponte  ou  falso nos testes
        │
        ▼
  controlador de movimento  ou  corpo simulado em MuJoCo
```

## As peças

| Crate | O que é | O que não tem |
|---|---|---|
| `rms1-proto` | quadro, verificadores e os comandos desta fase | E/S |
| `rms1d-params` | esquema, padrões com fonte e validação de `rms1d.toml` | |
| `rms1-control` | corpo, cinemática, intenções, segurança e laço | socket e tokio |
| `rms1d` | o processo, o contrato pelo socket e `rms1d-roteiro` | |

## O que vem do desenho do pato, e quem garante

- Só a segurança escreve no corpo. `Seguranca` é dona do único `Corpo`, e o compilador garante.
- Ligar não move nada. Desarmada, ela não envia nada que mova: só a telemetria e, uma vez, a sensibilidade das placas (`0x3f/0x07`, da central para cada uma das seis placas, 2401 a 2406, como a DJI: `armor_mask` e `setArmorParam armorId` no `dji_hdvt_uav`). Armar não leva a torre a lugar nenhum enquanto ninguém pediu.
- Seguindo a torre, parar é um `0x3f/0x22` a zero, uma vez, como a DJI: repetido, ele manteve o controlador alinhando o chassi no ar, e as rodas foram a 850 rpm na mão do dono (2026-11-03).
- Levantado, pelo cálculo próprio ou pelo bit do controlador, com telemetria recente, as rodas param em qualquer modo e ficam paradas até desarmar; o estado diz `picked_up`. Além da velocidade do controlador contra a das rodas, o levantado também marca quando as rodas giram o robô e o giroscópio não vê o giro, acima de 3 rad/s por 1,5 s: girando no lugar, no ar, a velocidade em linha reta quase não aparece (no chão, andando, até 0,93 s seguidos; no ar, 3,84 s). Nada mede o "voltou ao chão": com as rodas paradas na mão, a velocidade do controlador cai e o cálculo solta. Pela regra de 2 s do levantado, a parada chega cerca de 3,5 s depois de as rodas dispararem (medido na gravação de 2026-11-03).
- O laço nunca espera cliente. Intenções são trocas atômicas; o estado sai num canal limitado, e só é montado com assinante.
- A saúde é publicada pelo laço, então um laço travado se denuncia pela idade do último ciclo.
- Sair para o robô. `SIGTERM` e Ctrl-C passam pelo laço, que desarma e descarrega a gravação antes de o processo terminar. `kill -9` não tem esse caminho.
- A gravação não escreve no ciclo. O laço só monta a linha e a entrega a uma fila de 1.024, sem esperar; uma thread só dela escreve nos arquivos. Fila cheia descarta a linha inteira e conta; falha do disco não vira falha de envio ao corpo; os dois arquivos somados param em `corpo.gravar_limite_mb` (64 MiB, cerca de 27 minutos a 2,49 MB por minuto), e o laço segue. Em 2026-11-15 o disco do robô encheu, a escrita era feita no ciclo, e o laço caiu de 100 para 2 a 4 Hz, 55 s depois de subir (`out/2026-11-14-s1-midia/medicao-disco-do-robo.txt`, no laboratório). Prova: `rms1-control/tests/gravacao.rs`, com um disco de mentira lento e um cheio.
- Armar não cumpre pedido antigo: o que foi pedido antes de armar é esquecido, mesmo dentro do prazo.
- `<socket>.lock` garante um só dono, e nunca é apagado.
- `MissedTickBehavior::Skip`, em executor próprio.

## Quem manda no corpo: a tabela de fontes

**Em resumo:** cada cliente diz o nome no `hello`; em cada canal vale o pedido vivo da fonte de maior prioridade; sem pedido novo dentro do prazo, o canal passa à seguinte; a parada vale para qualquer um. A ordem é a do dono (2026-11-19): piloto, agente, cérebro.

Antes havia um quadro de pedidos só, e o último que escrevia vencia: o piloto e o agente se atropelavam. O pato de hoje diz quem pediu (`move.source`), e ele mesmo escreve que lá não há árbitro ("every intent slot is last-writer-wins", `robotd/src/intents.rs`). O desenho daqui é o do `twist_mux` do ROS (fonte com prioridade e prazo; os exemplos do pacote usam 0,5 s), por canal; e, do Roboglot (`docs/adr/0067`), a parada que nunca é arbitrada.

A tabela mora em `rms1d.toml`, uma linha `[[fontes]]` por fonte. A padrão de `rms1d-params` é a mesma que `deploy/s1/rms1d.toml` escreve por extenso (um teste reprova se as duas se afastam); escrita no arquivo, ela substitui a padrão inteira.

| Fonte (`nome`) | `prioridade` | `canais` | Quem é |
|---|---|---|---|
| `piloto` | 30 | todos | a página e o controle; toma o corpo na hora |
| `agente` | 20 | `rodas`, `torre`, `som` | o `rms1-agente`: não arma, não muda modo nem ajuste |
| `cerebro` | 10 | `rodas`, `torre`, `som`, com `so_autonomo` | só vale com a autonomia ligada |
| `sem_nome` | 0 | todos | quem não diz o nome, ou diz um que a tabela não tem (`robotctl`, um roteiro): pode o que podia antes, e perde para todos |

Os números só ordenam. Cada linha pode ter `prazo_ms` próprio (1 a 10000); sem ele vale `[seguranca] prazo_ms`, 500. A tabela é recusada, pelo nome do que está errado, com nome ou prioridade repetidos, canal desconhecido, sem a linha `sem_nome`, ou com mais de 16 linhas.

| Canal | O que cai nele |
|---|---|
| `rodas` | `robot.move` |
| `torre` | `robot.head`, `robot.look`, `s1.gimbal` |
| `som` | `robot.sound` |
| `energia` | `robot.enable`, `robot.init`, `robot.relax` |
| `modo` | `robot.do` |
| `ajustes` | `s1.armor` e `s1.sight` com valores (ler, qualquer fonte lê) |
| `autonomia` | `robot.autonomous` |

As regras, e onde cada uma mora:

- **Só as rodas e a torre são disputadas** (`rms1-control/src/intencoes.rs`). Cada fonte tem o seu quadro de pedidos de rodas e de velocidade da torre, com carimbo. A cada ciclo, em cada um dos dois, vale o pedido vivo (mais novo que o prazo da fonte) da fonte de maior prioridade. Sem pedido vivo de ninguém, o estado mostra o mais recente e diz `deadman`, como antes.
- **A segurança não escolhe: só julga a idade** do pedido que as intenções entregam, com o prazo da fonte dele (`seguranca.rs`). Segue sendo a única dona do corpo.
- **O alvo de ângulo da torre é um só,** com quem o pôs e quando. Uma fonte só o troca se nenhuma de prioridade maior tem a torre (velocidade viva, ou alvo posto por ela dentro do prazo). Velocidade diferente de zero de quem tem a torre apaga o alvo: nenhum alvo velho, de fonte nenhuma, volta quando a torre para. Vencida a posse, a torre fica onde está, e qualquer fonte pode pedir.
- **A parada nunca é barrada.** `robot.stop`, de qualquer fonte, apaga os pedidos de rodas e de velocidade da torre de todas, e deixa o zero de quem parou. Quem tem prioridade volta a mandar no pedido seguinte: a parada que segura o cérebro depois de um "pare" entra com ele.
- **Armar apaga os pedidos de todas as fontes,** como antes apagava o único.
- **A fonte `so_autonomo` só conta com a autonomia ligada** (`robot.autonomous {active}`, canal `autonomia`; sobe de `[autonomia] ligada`, falso no robô). Desligada, o que ela pediu deixa de valer na hora: as rodas, a velocidade da torre, e o alvo de ângulo que ela pôs.
- **O pedido de uma fonte superada fica guardado até o prazo dele** (é o `twist_mux`): quem para de renovar zera a própria linha, ou o último pedido passa a valer quando a vez da fonte maior vence. O `rms1-agente` faz isso ao desistir.
- **Pedido em canal que a fonte não tem é recusado com o motivo** (`rms1d/src/respostas.rs`, `barrado`); como notificação, é ignorado. O que responde documento e não `accepted` (`robot.look`, `s1.armor`, `s1.sight`) responde erro: `PERMISSION_DENIED`, ou `BUSY` quando a torre é de outra fonte.
- **A requisição (com id) de rodas ou de velocidade da torre que perde para uma fonte maior é recusada, dizendo quem tem o canal, e não fica guardada.** A notificação fica, e passa a valer quando a outra soltar: é assim que quem renova o pedido retoma o corpo. Dizer "recusado" e aplicar meio segundo depois seria mentir.
- **O nome `sem_nome` ninguém pode dizer no `hello`** (`INVALID_PARAMS`): quem o dissesse passaria por quem não disse nada. Um `hello` novo troca quem a conexão é.

O que o estado diz:

- `robot.state`: `move.source`, o nome de quem fez o pedido de `move.requested`, vivo ou vencido, como no pato (ausente para a fonte sem nome); e `autonomous`.
- `robot.mode`: `autonomous`.
- `s1.state` e `s1.telemetry`: o bloco `control`, que sai sempre, também sem telemetria.

```json
"control": {"client": "agente", "wheels": {"source": "piloto"}, "gimbal": {"source": null}, "autonomous": false}
```

`source` é o nome da fonte que tem o canal agora (pedido vivo), ou `null` sem nenhuma. `client` é a linha da tabela que a conexão que lê é: o nome dito no `hello`, ou `sem_nome` se a tabela não o tem. É por ele que um cliente sabe se o dono é ele mesmo, sem supor que o nome dele está na tabela (o `rms1-agente` e a página usam).

Provas: `rms1d-params` (a tabela errada, recusada pelo nome), `rms1-control/tests/fontes.rs` (a escolha, os prazos, a torre, a parada, a autonomia, pelos quadros que chegam ao corpo), `rms1d/tests/fontes.rs` (o processo de verdade, com três clientes) e, no laboratório, `out/2026-11-20-quem-manda-no-corpo/prova_quem_manda.py` (a cena inteira, com a estação de verdade). Cada uma com o defeito plantado reprovando (`.roadworthy/refutations.jsonl`).

## O contrato 68 do pato

O ramo do S1 subiu para cima do pato de hoje em 2026-11-20 (336 commits; um conflito, na lista de pacotes). `rms1d/src/respostas.rs` não tem braço genérico: cada chamada nova do contrato para a compilação até alguém decidir o que ela é neste corpo. As 15 novas:

| Chamada | Neste corpo |
|---|---|
| `robot.autonomous` | servida: é o nível da autonomia da tabela de fontes |
| `robot.laserTrack`, `robot.soccer` | recusadas com o motivo: este corpo não tem o jogo do pato |
| `robot.volume`, `robot.setVolume` | erro `METHOD_NOT_FOUND`: respondem documento, e o volume do S1 é do `rms1-mediad` (`s1.media.volume`) |
| `nfc.*`, `photo.*`, `map.*` | de outros serviços (`METHOD_NOT_FOUND`) |

`robot.remoteSessionActive` saiu do contrato, e daqui.

## O som é do `rms1-mediad`

`robot.sound` chega ao `rms1d`, como no pato chega ao `robotd`, e ele o repassa ao `rms1-mediad` como notificação, sem esperar pela resposta: o laço nunca espera cliente. Sem `[midia]` em `rms1d.toml`, ou com o `rms1-mediad` fora do ar, a chamada é recusada com o motivo. A câmera, o microfone e o alto-falante têm um dono só, e a página dele é [`rms1-mediad.md`](rms1-mediad.md).

## O pato é piso, não teto

O contrato comum é uma das saídas. O que o S1 tem a mais, como telemetria completa, armaduras, lançador e modos, entra inteiro nas etapas seguintes, com lista própria, sem ser cortado para caber no pato. Modos e tiro chegam ao console como skills de `robot.do`, porque `mediad` recusa `s1.*` em `session.rs`.

## O lugar da percepção na Coral

Isto é desenho; a implementação é da etapa 5, com a câmera.

- **Por onde o quadro chega:** `mediad` já serve um quadro bruto por socket local (`media.frame`), e tem entrada de câmera simulada (`--sim-camera`). No robô, a câmera sai de `dji_camera` no formato lido pela biblioteca, 640 por 360 por 3.
- **Quem chama a inferência:** um processo próprio de percepção, fora do laço de `rms1d`, como `duck-detect` fica fora do laço do pato. A inferência não pode nunca atrasar o laço de 50 Hz, e a Coral vive na USB, com acesso que pode falhar sem o robô parar.
- **Como o resultado volta:** como cliente, pelo mesmo socket: detecção vira `robot.move` e `robot.look`, ou notificação para o console, como `media.detections` no pato. Assim, a inteligência no simulador e no robô é o mesmo programa.

A confirmar na etapa 5, por pesquisa e medição: biblioteca da Coral para o sistema do S1, carga do firmware dela, e formato de modelo que o treino precisa gerar. O encaixe tem implementação de teste no Mac, para o simulador. Rodar na Coral real exige o robô, e entra na prova da fase.

## Modos

São os de `RobotCtrl.set_mode_attr` (`rm_ctrl.py`). `0x3f/0x46` vai a `dji_hdvt_uav`, o processo que paramos, então os modos saem dos comandos de chassi e torre.

| Skill | Chassi | Torre | Giro pedido |
|---|---|---|---|
| `modo_livre` | `0x3f/0x20` | `0x04/0x4c` 0, `0x3f/0xb0` com os dois eixos | rodas |
| `chassi_segue_torre` | `0x3f/0x22` | `0x04/0x4c` 0, `0x3f/0xb0` uma vez por mudança | taxa da torre, `0x04/0x0c`, e o chassi vai atrás |
| `torre_segue_chassi` | `0x3f/0x20` | `0x04/0x4c` 2, `0x3f/0xb0` só pitch | rodas |

## Telemetria

Etapa 3. O `rms1d` assina a telemetria do controlador como a DJI faz, com os mesmos quadros, e lê o pacote com um decodificador próprio, em `rms1-proto/src/barramento.rs`. A sequência mora em `rms1-control/src/telemetria.rs`, sem E/S.

```text
observar 2,5 s ─► cancelar o que viu ─► registrar o nó 09 ─► assinar a mensagem 1, 50 Hz, 9 dados ─► esperar ─► receber
                                                                     │ código 4: a mensagem existia calada
                                                                     └─► cancelar a mensagem 1, uma vez, e assinar de novo
```

- **Cada pacote é respondido,** como a DJI faz, com a mesma sequência.
- **Recusa para e diz o código.** Sem resposta, desiste depois de `tentativas`, e só recomeça com quadros chegando, sem martelar. Pacotes parados por `recomecar_parado_ms` recomeçam a sequência.
- **Fora da assinatura:** torre (`0x3f/0x76`), estado do chassi (`0x3f/0x29`), toque em cada placa (`0x3f/0x02`, pelo remetente) e infravermelho (`0x3f/0x10`).
- **No contrato:** `s1.state`, por assinatura com `s1.subscribe {hz}`, e `s1.telemetry`, uma vez. `robot.state` ganha IMU, odometria, segurança e, com pacote recente, `head` medido. A saúde diz "telemetria parada" pela idade do último pacote (`idade_maxima_ms`).
- **Eixos:** o fio usa x à frente, y à direita e z para baixo; o contrato, y à esquerda e z para cima. A conversão é uma só, em `Medidas::de`.
- **Parâmetros:** `[telemetria]` em `rms1d.toml`. O corpo falso não assina.
- **Nunca sai `0x48/0x02`,** o reset de nó: um teste confere em todos os caminhos.

## O que o controlador calcula e o que calculamos, em modo sombra

Medido no robô em 2026-11-03, nos dois modos, gravando por fora o que o controlador manda (`out/2026-11-03-firmware-controlador/`, no laboratório):
- **Acompanham o movimento real:** aceleração, giro, rotação das rodas e atitude. A atitude segue a gravidade, com até 3,0° de diferença parado.
- **O controlador erra:** velocidade e posição, que ele integra da aceleração. Inclinado na mão, parado, chegou a 13,1 m/s, e antes de o robô andar a posição dele se afastou até 0,60 m no modo DJI e 0,62 m no nosso. O bit de levantado nunca acendeu: 0 em 12636 pacotes, com cinco manuseios.

O pato faz o mesmo que este desenho: odometria e levantado próprios (`odometry/`, `duck-control/src/pickup.rs`), e a política não observa velocidade (`duck-control/src/obs.rs`).

`rms1-control/src/estimativa.rs` calcula, por pacote e pelo relógio do pacote, ao lado do que o controlador manda:
- **Velocidade:** das quatro rodas, pela inversa da cinemática (`twist_de_rpm`), na média de `janela_ms`. É zero quando o controlador marca parado, porque parado as rodas só dão ruído.
- **Posição:** essa velocidade, girada pelo rumo do controlador e integrada. Acima de `lacuna_ms` entre dois pacotes, não integra.
- **Levantado:** o controlador acusa mais velocidade do que as rodas, acima de `levantado_limiar_m_s`, por `levantado_tempo_ms`. Ou o giro pelas rodas passa do giro z da IMU por mais de `giro_levantado_rad_s`, por `giro_levantado_tempo_ms`.
- **Gravidade:** pela atitude do controlador, conferida contra a aceleração com o robô quieto. Quieto é parado pelo controlador, com o giro e a aceleração dentro dos limites. Acima de `divergencia_grau` há aviso: é o caso dos -24° presos de 2026-11-03.

Tudo sai em `s1.state.estimate`, ao lado do `body`. **O `robot.state` não muda:** `picked_up`, `odom` e `gravity` seguem com o controlador até a prova no robô em modo sombra, e a troca é decisão do dono. Os limiares ficam em `[estimativa]`, no `rms1d.toml`, e não são números do robô.

**Para validar sem o robô:** com `corpo.gravar_recebido`, o `rms1d` grava cada leitura do corpo. `rms1d-reprocessar <gravação>` reprocessa esse arquivo, ou o `strace` das leituras, e mostra, por pacote ou por segundo, o controlador ao lado do nosso cálculo. `scripts/test-rms1-telemetria.sh` confere o cálculo contra as duas gravações de 2026-11-03.

Nas gravações de 2026-11-03:
- **Na mão:** com o controlador acima de 2 m/s, a nossa velocidade fica em até 0,05 m/s.
- **Andando:** a diferença para o controlador é de até 0,044 m/s por segundo.
- **Levantado:** cada manuseio marca, e andando nenhum.
- **Gravidade:** nenhum aviso; a divergência máxima é de 3,0° no modo DJI e 1,2° no nosso.

**Sem prova:** o levantado com o robô carregado e as rodas girando, ou levantado devagar demais para o controlador acusar velocidade; o escorregamento real das rodas.

## Ativação pela bancada

`scripts/s1/rms1d-bench.sh up` escolhe pelo que o robô tem: o mesmo `rms1d` no ar, com o mesmo binário e a mesma configuração pela soma `sha256`, e saudável, só religa a ponte ("Conectando ao robô"); outro `rms1d` no ar, "Atualizando o robô"; nada nosso, "Preparando o robô". Os três terminam em "Robô pronto". O binário só vai ao robô quando a soma muda, e os processos da DJI só param se estiverem no ar.

Com `[midia]` no arquivo, a ativação sobe também o `rms1-agente`, depois do `rms1d`, que ele comanda, e o de antes sai primeiro; "o mesmo no ar" inclui o agente vivo e com o mesmo binário. Sem o binário (`cargo s1 -p rms1-agente`), ou sem ele subir, a ativação avisa com o motivo e segue: dá para pilotar, e o robô não cumpre os pedidos da estação. A volta o encerra.

As gravações do `rms1d` que sai (`corpo.gravar` e `corpo.gravar_recebido`) mudam de nome no robô para `<arquivo>.robo-<data>-<hora>`, um nome por ativação, e são copiadas ao fundo, logo, para `$XDG_STATE_HOME/rms1/gravacoes/<data>/`; só são apagadas no robô depois de a cópia chegar. A ativação seguinte e a volta à DJI esperam a cópia em curso, com prazo. Prova: `scripts/s1/test-rms1d-bench.sh`, com contraprova de cada caso.

**O disco do robô, antes de parar a DJI.** Com gravação no arquivo, a ativação lê o espaço livre do `/data` (`busybox df -k /data`) e exige o limite da gravação, que ela pergunta ao próprio `rms1d` (`--limite-da-gravacao`): 64 MiB com o padrão (até 2026-11-15 eram duas vezes o limite, por margem). Abaixo disso, ou sem resposta legível, para dizendo os números, sem parar nada no robô. Uma gravação de ativação anterior que ficou no robô, porque a cópia dela não chegou, entra na cópia de toda ativação: copiada, conferida pela soma SHA-256 dos dois lados, e só então apagada; com pouco espaço, isso roda à vista, no passo "Guardando as gravações antigas do robô", antes da conferência. A que não copia, ou não confere, fica no robô, e a ativação diz o nome e o tamanho. Em 2026-11-15, duas esquecidas, de 62 e 39 MB, tinham deixado o disco com 1,4 MB livres.

## A mira da DJI

O app desenha a mira numa posição fixa da tela, que o usuário ajusta e o robô guarda: `DJIRobomasterSystemSightBeadPosition`, com `0x3f/0x56` para gravar e `0x3f/0x57` para ler, os dois no `dji_hdvt_uav`. Ele guarda quatro números em `/data/rm_user_setting.json`: com zoom x e y, sem zoom x e y (`front_sight_zoom_inx`, `_iny`, `_outx`, `_outy`), em décimos de milésimo da tela. O padrão de fábrica é 5000 nos quatro, o centro: o `dji_hdvt_uav` os grava antes de ler o arquivo (`0x1d2f8`). Não há correção pela distância; o cano fica 4,3 cm abaixo da câmera (`assets/robomaster-s1/dados/robo_fisica.json`, no laboratório: câmera a 0,231 m e boca do cano a 0,188 m), e o tiro cai abaixo da mira de perto.

No modo do `rms1d` o `dji_hdvt_uav` está parado, e quem lê e grava o mesmo arquivo é o `rms1d`, por `s1.sight`: sem valores, responde `{"zoom_in": [x, y], "zoom_out": [x, y]}`; com os dois pares, inteiros de 0 a 10000, grava e responde o que ficou. A gravação muda só os quatro números, confere o resultado e troca o arquivo por um temporário e `rename`, com o dono e as permissões do original, uma gravação de cada vez. Assim, o ajuste feito na página deve valer no app, e o contrário: falta a prova do dono, ajustando de um lado e lendo do outro. O caminho vem de `[mira] arquivo`, em `rms1d.toml`; no robô, `/mnt/dji/rm_user_setting.json`, porque a bancada liga o `/data` da DJI dentro do chroot. Sem `[mira] arquivo`, `s1.sight` responde o padrão e recusa gravar.

**Não verificado:** qual dos dois pares o app usa sem o zoom digital; se y conta do alto da tela (o firmware entrega `1 - y / 10000` ao Python, `rm_ctrl.py:5016`); e a ordem dos quatro na resposta de `0x3f/0x57`, que vem dos quatro campos seguidos do leitor em `0x1d670` e da ordem das chaves no binário, sem ter sido conferida campo a campo. Com 5000 nas quatro, nada disso muda o que se vê. A prova é ajustar no app e ler aqui.

## Limitação conhecida

Com telemetria, ainda não há malha fechada: `rms1d` manda rpm às rodas, e a velocidade medida não volta ao comando. No simulador, pelo mesmo caminho, a 1,2 m/s por 3 s o desvio é de até 0,006 m, em 8 fases de início. A 3,5 m/s varia de 0,013 a 0,768 m, sem tendência: compatível com o contato dos roletes, mas a causa não está provada. A malha fechada pede plano próprio. Fonte: `tools/simulador/test_quadros.mjs`, no laboratório.

`head`, no estado, é onde a torre está quando há pacote recente; sem ele, é o último alvo pedido e limitado, e nunca um dado velho.
