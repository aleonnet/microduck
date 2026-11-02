# `rms1d`: roteiro da prova, chassi e torre

Um comando e uma página, para os dois corpos. O robô só é tocado com o dono presente, e cada movimento é avisado antes. Nada grava em `/system`, e reiniciar volta à DJI.

```sh
scripts/rms1
```

Compila, sobe o serviço local, abre a página e fica aguardando o robô na rede. Não usa o Python do terminal. Encerrar: Ctrl-C, ou `scripts/rms1 down` de outro terminal.

## No simulador

| Passo | Faça | Esperado |
|---|---|---|
| 1 | olhe a faixa no topo da página | `Simulador`, `Desarmado, livre`, `Aguardando o robô na rede.`, sem "Ativar robô" |
| 2 | segure `W` sem armar | nada se move, e a faixa diz para apertar Armar |
| 3 | clique em `Armar` | a faixa diz `Armado`, e o botão vira `Desarmar` |
| 4 | segure `W`, `S`, `A`, `D`, `Q`, `E` | anda e gira no sentido da tecla, a 1,2 m/s e 2,4 rad/s; com `Shift`, mais rápido. O robô EP fica à esquerda do início, e `A` bate nele |
| 5 | setas, e arrastar na tela no modo Pilotar | a torre gira e inclina no sentido certo, até os limites da tabela |
| 6 | `F` até `Torre segue o chassi` | `Q` gira chassi e torre juntos, e arrastar só muda a inclinação |
| 7 | `F` até `Chassi segue a torre` | `W` anda; `Q` gira a torre, e o chassi vai atrás; ao soltar, ele alinha e as rodas param. A faixa avisa do que acontece fora do chão |
| 8 | feche a aba com `W` apertado | o robô para sozinho em 500 ms |
| 9 | abra "Quadros do corpo", na caixa de controle | contagem por comando; nenhum em vermelho |

## No robô

Robô ligado com a DJI e conectado ao mesmo roteador deste Mac: o anúncio na rede só existe nesse modo (`wifi_sta_connected_state_enter`, em `dji_network`). Rodas fora do chão até o passo 5.

| Passo | Faça | Esperado | Reprova se |
|---|---|---|---|
| 1 | espere | "Ativar robô" aparece, e a faixa diz o endereço do robô | não aparece: o anúncio real nunca foi ouvido por este Mac, e isto é o que se mede aqui |
| 2 | clique em "Ativar robô" e confirme as rodas | os passos aparecem um a um; a faixa acende em `Robô real`, e o 3D perde a cor. Nada se move. `AVISO` de serial muda não para a subida, mas anote | para com erro, ou moveu |
| 3 | `Armar`, e `W`, `S`, `A`, `D`, `Q`, `E` | cada roda gira no sentido do simulador; ao soltar, para | sentido errado, ou segue girando |
| 4 | setas e arrasto | a torre obedece na hora, sem tremer, nos dois eixos, no sentido certo e até o limite; a faixa mostra "resposta em X ms" | eixo ou sentido errado, tremor ou atraso |
| 5 | `F` até `Chassi segue a torre`, `Q` e soltar | as rodas seguem girando, como a faixa avisa: fora do chão o chassi nunca alinha | |
| 6 | no chão, em espaço que o dono liberar: `Q` e soltar, depois os três modos | o chassi alinha e para; obedece nos três | segue girando no chão |
| 7 | "Voltar ao modo DJI" | o simulador volta na hora; o robô termina ao fundo, com os passos na faixa; a DJI voltou; a gravação fica em `out/` do laboratório | não volta, ou segue andando |
| 8 | desligue na chave | só isso fecha a porta root, e a faixa lembra | |

**Se as rodas não obedecerem:** em `deploy/s1/rms1d.toml`, `permissao = "proroklab"`, e "Ativar robô" de novo. Se a torre errar, `coordenada = 3`. Registre qual valeu.

**Comparar os quadros:** com o robô ativo, `target/debug/rms1d-roteiro --socket /tmp/rms1/bancada/robot.sock`; a gravação fica em `out/` do laboratório, em `robo-<hora>.quadros`, ao voltar. Compare com `target/debug/rms1d-roteiro --comparar <robo-hora.quadros> <sim.quadros>`; `sim.quadros` é o de `/tmp/rms1/`, depois do mesmo roteiro no simulador, com `--socket /tmp/rms1/robot.sock`.
