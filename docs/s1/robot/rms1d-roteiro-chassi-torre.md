# `rms1d`: roteiro da prova, etapas 1 e 2

O mesmo caminho do simulador. O robô só é tocado com o dono presente, e cada movimento é avisado antes. Nada grava em `/system`, e reiniciar volta à DJI.

## No simulador

```sh
scripts/rms1d-sim
```

Abre a página do simulador e o console. O simulador segue rodando com a página atrás, mas para ver o robô é melhor deixar as duas janelas lado a lado.

| Passo | Faça | Esperado |
|---|---|---|
| 1 | no console, clique em `connect` | nome `rms1-sim`, `control open`, sem alerta vermelho, `healthy`, `held` |
| 2 | segure `W` sem armar | nada se move; `applied` mostra `disarmed` |
| 3 | clique em `enable` | `modo_livre` |
| 4 | segure `W`, `S`, `A`, `D`, `Q`, `E` | anda e gira no sentido da tecla, a 0,3 m/s e 1,5 rad/s. O robô EP fica à esquerda do início, e `A` bate nele |
| 5 | arraste na imagem | a torre olha para onde a imagem foi arrastada, a 30°/s, a velocidade padrão da DJI. Ao começar, ela sobe cerca de 10°: é a conta de `robot.look` no console do pato |
| 6 | em `do`, escolha `torre_segue_chassi` | `Q` gira chassi e torre juntos, e arrastar só muda a inclinação |
| 7 | escolha `chassi_segue_torre` | `W` anda; `A` anda para a esquerda; `Q` gira a torre, e o chassi vai atrás |
| 8 | feche o console com `W` apertado | o robô para sozinho em 500 ms |
| 9 | abra "Quadros do corpo", na página | contagem por comando; nenhum em vermelho |

Encerrar: `scripts/rms1d-sim down`.

## No robô

Robô ligado com a DJI, na mesma rede, `rodas fora do chão` para os passos 2 a 4.

| Passo | Faça | Esperado | Reprova se |
|---|---|---|---|
| 1 | `scripts/s1/rms1d-bench.sh up` | `robot healthy`, e nada se move. Pode rodar de novo: o `rms1d` anterior sai primeiro. `AVISO` de serial muda não para a subida, mas anote: a leitura de 2026-10-30 diz que o corpo fala sozinho | doente, moveu, ou parou dizendo que a serial não ficou livre |
| 2 | `target/debug/rms1d-roteiro --socket /tmp/rms1d-bench/robot.sock` | `ROTEIRO COMPLETO`; cada roda no sentido da simulação; sem comando, para em 500 ms; a torre obedece e os modos trocam | sentido errado ou segue andando |
| 3 | `scripts/s1/rms1d-bench.sh up` de novo: a gravação do roteiro fica em `out/` do laboratório, em `robo-<hora>.quadros`. Compare: `target/debug/rms1d-roteiro --comparar <robo-hora.quadros> <sim.quadros>`. Para ter `sim.quadros`: `scripts/rms1d-sim`, só o roteiro do passo 2 com `--socket /tmp/rms1d-sim/robot.sock`, `scripts/rms1d-sim down`, e fica em `/tmp/rms1d-sim/sim.quadros` | `IGUAIS` | diferença sem razão |
| 4 | se as rodas não obedecerem | em `deploy/s1/rms1d.toml`, `permissao = "proroklab"`, e `up` de novo. Se a torre errar, `coordenada = 3`. Registre qual valeu | os dois falham |
| 5 | no chão, `scripts/s1/rms1d-bench.sh console`, e pilote nos três modos. Em `chassi_segue_torre`, confira `A` e `D`, e veja se `W` é a frente do chassi ou a da torre | obedece, a até 0,3 m/s | não obedece, ou lado trocado |
| 6 | em espaço que o dono liberar: `target/debug/rms1d-roteiro --socket /tmp/rms1d-bench/robot.sock --so-andar --velocidade 1.2`. Ele volta ao modo livre antes de andar | anda e para | não anda ou não para |
| 7 | com as rodas fora do chão, ande pelo console e rode `scripts/s1/rms1d-bench.sh volta` | as rodas param ao sair; a DJI volta; a gravação, que termina no comando de parar, fica em `out/` do laboratório | segue andando, ou não volta |

Ao terminar, desligue na chave: só isso fecha a porta root.
