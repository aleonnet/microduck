# `rms1d`: pontos de controle, etapas 1 e 2

Cada ponto diz como a DJI faz, como `rms1d` faz, o que o dono viu, o que falta provar, e o que esperar. Fontes: `~/Development/RoboMaster-S1/biblioteca/guia/chassi.md` e `torre.md`, e o firmware copiado do robô.

"Dono viu" é o testemunho de 2026-11-01: chassi, torre, câmera, microfone, alto-falantes, sensores, luzes, canhão, LED e sons funcionando. Nada aqui foi rodado ainda no robô pelo `rms1d`.

| Ponto | DJI | `rms1d` | Unidade e sentido | Falta provar no robô | Reprova se |
|---|---|---|---|---|---|
| Permissão de andar | `0x3f/0x60`, 900 para 306, 14 bytes de alavanca neutra, cerca de 48 por segundo | o mesmo, a cada ciclo de 50 Hz, só armado | centro 1024 em todos os campos | que estes bytes armam; plano B: `permissao = "proroklab"` | rodas não obedecem |
| Modo do chassi | `0x3f/0x19`, 906 para 306 | 1, kit de programação, cinco vezes ao armar | `rm_define.chassis_sdk_mode` | | rodas não obedecem |
| Rodas | `0x3f/0x20`, int16 × 4 | cinemática mecanum, a cada ciclo | rpm, ±1000; roda 2 e 3 invertidas no fio (`rm_ctrl.py:394`) | sentido de frente, lado e giro | sentido errado |
| Seguir a torre | `0x3f/0x22`, float × 3, sem resposta | vx, vy e ângulo 0, a cada ciclo | m/s e graus; y à direita na DJI (`_move_ctrl`), à esquerda no contrato: o sinal inverte | que o chassi persegue a torre; que anda para o lado certo; se a frente é a do chassi ou a da torre | chassi não acompanha, ou lado errado |
| Sobreposição da alavanca | `0x3f/0x28` | 0, desligada, ao armar e ao trocar de modo | 0, 1, 2 | | |
| Torre por ângulo | `0x3f/0xb0`, 906 para 400; identificador de tarefa novo a cada envio (`rm_module.py:429`); velocidade padrão 30°/s (`rm_ctrl.py:1369`); o `rms1d` usa `velocidade_torre`, da tabela física | coordenada 4, reenvio a cada 100 ms, salvo seguindo a torre; identificador de 1 a 254; 30°/s em `rms1d.toml` | décimos de grau; yaw à direita e pitch para cima na DJI; o contrato inverte os dois | coordenada 4; plano B: `coordenada = 3`. Sentido. Se a torre vai a 30°/s, como o simulador faz | olha para o lado errado |
| Torre por taxa | `0x04/0x0c`, int16 × 3 e controle | só seguindo a torre: giro pedido; `0xdc` em movimento, zero com controle 0 ao parar | décimos de grau por segundo | que a torre gira e para | não para |
| Modo da torre | `0x04/0x4c` | 0 ou 2, conforme o modo | 0 livre, 1 FPV, 2 seguindo | | giro errado |
| Prazo sem comando | não lido; o pato usa 500 ms | 500 ms, em `rms1d.toml`; zera o giro e a velocidade, nos três modos | | que para em 500 ms | segue andando |
| Parar ao desarmar ou sair | `_set_chassis_stop_depend_mode`, `_stop_rotate` | rodas ou seguir a zero, dez vezes, conforme o modo; taxa zero com controle 0, se ligada. `SIGTERM` faz o mesmo | | que para. O que o corpo faz com `kill -9`, sem o comando de parar, não foi lido | segue andando |
| Reset de nó, `0x48/0x02` | deixou o barramento morto até desligar | nenhum comando de `rms1-proto` é desse conjunto, e um teste confere | | | |
| Processos parados | | só `dji_monitor` e `dji_hdvt_uav`, nessa ordem; `rms1d.toml` recusa qualquer outro nome, antes de o script tocar no robô | | `up` confere que os dois saíram e que ninguém segura `/dev/ttyS3` | `up` para e diz quem |

## O que a leitura não fecha, e só o robô responde

- `0x3f/0x19` sai de 906, como `rm_module.py`; a gravação que moveu o robô em 2026-10-29 saiu de 900. Se as rodas não obedecerem depois de tentar `permissao = "proroklab"`, é o próximo suspeito.
- `0x3f/0x28` e `0x04/0x4c` saem sem pedir resposta; a DJI pede. A gravação de `0x3f/0x19` sem pedir moveu o robô, e as outras duas não têm gravação nossa.
- A ordem frente-direita, frente-esquerda, trás-esquerda, trás-direita vem do `s1d`. A gravação só cobre andar reto: lado e giro, com as rodas fora do chão, respondem.
- Sair de "chassi segue a torre" manda rodas. `rm_ctrl.py:402` recusa rodas nesse modo, na camada de script; o que o controlador faz, só ele diz.
- Um alvo da torre em coordenada 4, uma vez, com o chassi seguindo: o simulador trata como rumo no mundo, e o chassi gira até lá e para. Se no robô o chassi correr atrás sem parar, é aqui.
- Identificador de tarefa novo a cada 100 ms: a DJI só manda quando muda. Se a torre tremer ou parar, é o próximo suspeito, e `torre.reenvio_ms` em `rms1d.toml` resolve.
- Sem o comando de parar, com `kill -9`, o que o corpo faz não foi lido. O simulador desarma em 0,5 s, e isso é hipótese nossa. Os scripts da DJI contam com um envio de rodas durando até 20 s (`_move`, `rm_ctrl.py`): o prazo sem batimento é o que a prova precisa medir, com as rodas fora do chão.
- Se os dez quadros de parar saem da fila da serial antes de o processo fechar: passo 7 do roteiro.

## Comparação com o `s1d`

O `s1d` não mudou. Cada diferença, com a razão:

| O quê | `s1d` | `rms1d` | Razão |
|---|---|---|---|
| Permissão | bytes do proroklab, ao nó 3 | bytes da DJI, ao nó `0xC3` | vale a biblioteca; o antigo fica como plano B em `rms1d.toml` |
| Coordenada da torre | 3 | 4 | padrão de `angle_ctrl`, `rm_ctrl.py:1662`, e `rm_define.py:189`, yaw no chassi e pitch NED; o antigo fica como plano B |
| Ritmo | 100 Hz | 50 Hz, parâmetro | a DJI manda a alavanca a cerca de 48 por segundo |
| Limite de rodas | corta cada roda | encolhe as quatro juntas | cortar só uma muda o rumo |
| Identificador de tarefa da torre | fixo em 7 | troca a cada envio, de 1 a 254 | como `rm_module.py:429` |
| Valor não finito | não trata | recusado, e o robô para | invariante do desenho do pato |
| Meia soma do chassi | 0,2 m, de URDF de terceiros | 0,195 m, média da malha do app | fonte da DJI em vez de terceiros |
| Giro seguindo a torre | ignorado | vira taxa da torre, e o chassi vai atrás | a alavanca da DJI faz assim, e o console só tem esse giro |
| Parâmetros | flags e constantes | `rms1d.toml`, com fonte e chave desconhecida recusada | pedido do dono |
| Modos | `s1.follow`, que o console recusa | skills de `robot.do` | caminho que o console já permite |
| Telemetria, luzes, som, tiro, câmera | presentes | etapas 3, 4 e 5 | plano próprio para cada uma, com prova no robô |
| Saúde sem cliente | | laço publica; `system.info` e `telemetry.status` respondem | o console mostraria alerta sem eles |
