# `rms1d`: o serviço do corpo do S1

Status: em uso no simulador; prova no robô pendente. Data: 2026-11-03.

O que conta como pronto, pelo dono: atuar, ler e receber telemetria para controlar o S1 no mesmo nível do pato, com lista finita de pontos de controle. O teste só vale no caminho da entrega: controlar, interagir, treinar e colocar inteligência no S1.

Este serviço é novo. O `s1d` fica sem mudança, como comparação. A origem de bytes, unidades e sentidos é a biblioteca do laboratório, `~/Development/RoboMaster-S1/biblioteca`, e o firmware copiado do robô.

## O caminho

Um caminho só. Entre simulador e robô, só muda `[corpo] tipo` em `rms1d.toml`.

```text
console, robotctl, rms1d-roteiro, e depois a inteligência
        │  JSON-RPC, um socket unix, o contrato do pato
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
- Ligar não move nada. Desarmada, ela não envia nada que mova: só a telemetria e, uma vez, a sensibilidade das placas (`0x3f/0x07`, da central para a 2401, como a DJI). Armar não leva a torre a lugar nenhum enquanto ninguém pediu.
- Seguindo a torre, parar é um `0x3f/0x22` a zero, uma vez, como a DJI: repetido, ele manteve o controlador alinhando o chassi no ar, e as rodas foram a 850 rpm na mão do dono (2026-11-03).
- Levantado, pelo cálculo próprio ou pelo bit do controlador, as rodas param em qualquer modo e ficam paradas até o robô voltar ao chão e o cliente parar de pedir movimento; o estado diz `picked_up`. Pela regra de 2 s do levantado, a parada chega cerca de 3,5 s depois de as rodas dispararem (medido na gravação de 2026-11-03).
- O laço nunca espera cliente. Intenções são trocas atômicas; o estado sai num canal limitado, e só é montado com assinante.
- A saúde é publicada pelo laço, então um laço travado se denuncia pela idade do último ciclo.
- Sair para o robô. `SIGTERM` e Ctrl-C passam pelo laço, que desarma e descarrega a gravação antes de o processo terminar. `kill -9` não tem esse caminho.
- Armar não cumpre pedido antigo: o que foi pedido antes de armar é esquecido, mesmo dentro do prazo.
- `<socket>.lock` garante um só dono, e nunca é apagado.
- `MissedTickBehavior::Skip`, em executor próprio.

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
- **Levantado:** o controlador acusa mais velocidade do que as rodas, acima de `levantado_limiar_m_s`, por `levantado_tempo_ms`.
- **Gravidade:** pela atitude do controlador, conferida contra a aceleração com o robô quieto. Quieto é parado pelo controlador, com o giro e a aceleração dentro dos limites. Acima de `divergencia_grau` há aviso: é o caso dos -24° presos de 2026-11-03.

Tudo sai em `s1.state.estimate`, ao lado do `body`. **O `robot.state` não muda:** `picked_up`, `odom` e `gravity` seguem com o controlador até a prova no robô em modo sombra, e a troca é decisão do dono. Os limiares ficam em `[estimativa]`, no `rms1d.toml`, e não são números do robô.

**Para validar sem o robô:** com `corpo.gravar_recebido`, o `rms1d` grava cada leitura do corpo. `rms1d-reprocessar <gravação>` reprocessa esse arquivo, ou o `strace` das leituras, e mostra, por pacote ou por segundo, o controlador ao lado do nosso cálculo. `scripts/test-rms1-telemetria.sh` confere o cálculo contra as duas gravações de 2026-11-03.

Nas gravações de 2026-11-03:
- **Na mão:** com o controlador acima de 2 m/s, a nossa velocidade fica em até 0,05 m/s.
- **Andando:** a diferença para o controlador é de até 0,044 m/s por segundo.
- **Levantado:** cada manuseio marca, e andando nenhum.
- **Gravidade:** nenhum aviso; a divergência máxima é de 3,0° no modo DJI e 1,2° no nosso.

**Sem prova:** o levantado com o robô carregado e as rodas girando, ou levantado devagar demais para o controlador acusar velocidade; o escorregamento real das rodas.

## Limitação conhecida

Com telemetria, ainda não há malha fechada: `rms1d` manda rpm às rodas, e a velocidade medida não volta ao comando. No simulador, pelo mesmo caminho, a 1,2 m/s por 3 s o desvio é de até 0,006 m, em 8 fases de início. A 3,5 m/s varia de 0,013 a 0,768 m, sem tendência: compatível com o contato dos roletes, mas a causa não está provada. A malha fechada pede plano próprio. Fonte: `tools/simulador/test_quadros.mjs`, no laboratório.

`head`, no estado, é onde a torre está quando há pacote recente; sem ele, é o último alvo pedido e limitado, e nunca um dado velho.
