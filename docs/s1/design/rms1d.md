# `rms1d`: o serviço do corpo do S1

Status: em uso no simulador; prova no robô pendente. Data: 2026-11-02.

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
- Ligar não move nada. Desarmada, ela não envia nenhum quadro; e armar não leva a torre a lugar nenhum enquanto ninguém pediu.
- O laço nunca espera cliente. Intenções são trocas atômicas; o estado sai num canal limitado, e só é montado com assinante.
- A saúde é publicada pelo laço, então um laço travado se denuncia pela idade do último ciclo.
- Sair para o robô. `SIGTERM` e Ctrl-C passam pelo laço, que desarma e descarrega a gravação antes de o processo terminar. `kill -9` não tem esse caminho.
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

## Limitação conhecida

Sem telemetria, não há malha fechada: `rms1d` manda rpm às rodas. No simulador, pelo mesmo caminho, a 1,2 m/s por 3 s o desvio é de até 0,006 m, em 8 fases de início. A 3,5 m/s varia de 0,013 a 0,768 m, sem tendência: compatível com o contato dos roletes, mas a causa não está provada. A malha fechada entra com a telemetria, na etapa 3. Fonte: `tools/simulador/test_quadros.mjs`, no laboratório.

Pelo mesmo motivo, `head`, no estado, é o último alvo pedido e limitado, e não onde a torre está. No modo em que o chassi segue a torre, a taxa move a torre sem mudar esse alvo.
