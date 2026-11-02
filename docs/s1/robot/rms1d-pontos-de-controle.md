# `rms1d`: pontos de controle, etapas 1 e 2

Cada ponto diz como a DJI faz, como `rms1d` faz, o que o dono viu, o que falta provar, e o que esperar. Fontes: `~/Development/RoboMaster-S1/biblioteca/guia/chassi.md` e `torre.md`, e o firmware copiado do robô.

"Dono viu" é o testemunho de 2026-11-01: chassi, torre, câmera, microfone, alto-falantes, sensores, luzes, canhão, LED e sons funcionando. Nada aqui foi rodado ainda no robô pelo `rms1d`.

| Ponto | DJI | `rms1d` | Unidade e sentido | Falta provar no robô | Reprova se |
|---|---|---|---|---|---|
| Permissão de andar | `0x3f/0x60`, 900 para 306, 14 bytes de alavanca neutra, cerca de 48 por segundo | o mesmo, a cada ciclo de 50 Hz, só armado | centro 1024 em todos os campos | que estes bytes armam; plano B: `permissao = "proroklab"` | rodas não obedecem |
| Modo do chassi | `0x3f/0x19`, 906 para 306 | 1, kit de programação, cinco vezes ao armar | `rm_define.chassis_sdk_mode` | | rodas não obedecem |
| Rodas | `0x3f/0x20`, int16 × 4 | cinemática mecanum, a cada ciclo | rpm, ±1000; roda 2 e 3 invertidas no fio (`rm_ctrl.py:394`) | sentido de frente, lado e giro | sentido errado |
| Seguir a torre | `0x3f/0x22`, float × 3, sem resposta | vx, vy e ângulo 0, a cada ciclo | m/s e graus | que o chassi persegue a torre | chassi não acompanha |
| Sobreposição da alavanca | `0x3f/0x28` | 0, desligada, ao armar e ao trocar de modo | 0, 1, 2 | | |
| Torre por ângulo | `0x3f/0xb0`, 906 para 400 | coordenada 4, reenvio a cada 100 ms, salvo seguindo a torre | décimos de grau; yaw à direita e pitch para cima na DJI; o contrato inverte os dois | coordenada 4; plano B: `coordenada = 3`. Sentido | olha para o lado errado |
| Torre por taxa | `0x04/0x0c`, int16 × 3 e controle | só seguindo a torre: giro pedido; `0xdc` em movimento, zero com controle 0 ao parar | décimos de grau por segundo | que a torre gira e para | não para |
| Modo da torre | `0x04/0x4c` | 0 ou 2, conforme o modo | 0 livre, 1 FPV, 2 seguindo | | giro errado |
| Prazo sem comando | não lido; o pato usa 500 ms | 500 ms, em `rms1d.toml`; zera o giro e a velocidade | | que para em 500 ms | segue andando |
| Reset de nó, `0x48/0x02` | deixou o barramento morto até desligar | não existe no código, e um teste varre | | | |
| Processos parados | | só `dji_hdvt_uav` e `dji_monitor`; `dji_scratch`, `dji_sys` e `dji_network` são recusados em `rms1d.toml` | | que a serial fica livre | `up` não sobe |

## Comparação com o `s1d`

O `s1d` não mudou. Cada diferença, com a razão:

| O quê | `s1d` | `rms1d` | Razão |
|---|---|---|---|
| Permissão | bytes do proroklab, ao nó 3 | bytes da DJI, ao nó `0xC3` | vale a biblioteca; o antigo fica como plano B em `rms1d.toml` |
| Coordenada da torre | 3 | 4 | padrão de `angle_ctrl`, `rm_ctrl.py:1662`, e `rm_define.py:189`, yaw no chassi e pitch NED; o antigo fica como plano B |
| Ritmo | 100 Hz | 50 Hz, parâmetro | a DJI manda a alavanca a cerca de 48 por segundo |
| Limite de rodas | corta cada roda | encolhe as quatro juntas | cortar só uma muda o rumo |
| Valor não finito | não trata | recusado, e o robô para | invariante do desenho do pato |
| Meia soma do chassi | 0,2 m, de URDF de terceiros | 0,195 m, média da malha do app | fonte da DJI em vez de terceiros |
| Giro seguindo a torre | ignorado | vira taxa da torre, e o chassi vai atrás | a alavanca da DJI faz assim, e o console só tem esse giro |
| Parâmetros | flags e constantes | `rms1d.toml`, com fonte e chave desconhecida recusada | pedido do dono |
| Modos | `s1.follow`, que o console recusa | skills de `robot.do` | caminho que o console já permite |
| Telemetria, luzes, som, tiro, câmera | presentes | etapas 3, 4 e 5 | plano próprio para cada uma, com prova no robô |
| Saúde sem cliente | | laço publica; `system.info` e `telemetry.status` respondem | o console mostraria alerta sem eles |
