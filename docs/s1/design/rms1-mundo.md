# `rms1-mundo`: a camada de mundo do S1

**Resumo.** O pacote recebe a grade de profundidade que a estação Hailo entrega (a função Distância, com o `scdepthv3`), as
amostras da torre e da pose que o `rms1d` publica e a câmera, e devolve o chão em volta do robô em metros: cada célula
livre, ocupada ou não vista, a folga por direção em volta do corpo, o obstáculo à frente na forma que o cérebro consome, e o
estado da percepção. É puro, sem soquete e sem relógio próprio, como o `rms1-control`: quem fala com a estação e com o
`rms1d` é o `rms1-mediad`. O molde da memória é o do cérebro do pato (`autod/src/obstacle.rs`).

**Estado em 2026-11-20:** o pacote existe e é provado com cenas desenhadas (43 testes com os de `rms1d-params`, e duas
contraprovas). Ainda não está ligado ao `rms1-mediad`, e a câmera do robô não foi medida: sem ela, o mundo do robô só diz
`no_camera`. Nenhum número deste documento vem do robô.

**Estado em 2026-11-21:** o pacote está ligado ao `rms1-mediad` e publica (seção da camada de mundo em
[`rms1-mediad.md`](rms1-mediad.md)). **A câmera é uma só, para o robô e para o simulador:** a entrada `camera` da tabela
física. Hoje é a nominal: 54 graus na vertical, o campo com que o código da DJI trata o quadro (96 por 54), com o ponto
quadrado e o centro no meio, sem distorção, e a origem `provisorio`. **O robô mostra o mundo sem esperar a calibração:** com a
lente provisória a linha diz `lens.calibrated: false`; a calibração grava a lente medida na tabela, para os dois, e só diz se
a medida está certa. No simulador a mesma lente é exata por construção (a página desenha com ela), e a linha diz `true`.
O cano do lançador, que a lente do robô vê embaixo, no meio do quadro, sai da conta pela
geometria dele (`cano_na_lente`, da malha do app, com a folga `corpo_folga_m`). E a conferência das duas contas da escala
tem o limite dela, `reta_tolerancia`: no quadro real guardado (no laboratório,
`out/2026-11-20-o-s1-ve-em-metros/exame-da-grade-real.txt`) a foto indica a torre a 6,5 graus para baixo, e nessa
inclinação as duas contas diferem 18,8%; com o limite antigo (10%) o chão real era recusado. Com 0,3, na lente do robô, o
erro de inclinação de 20 graus ou mais é sempre recusado, e o que passa erra a escala em até 32,4% (o teste
`com_a_inclinacao_muito_errada_o_quadro_nao_e_dado_por_medido` mede e limita): o erro pequeno quem mede é a calibração.

## 1. O que entra e o que sai

| Entra | De onde | Forma |
|---|---|---|
| a câmera | a tabela física (`rms1d-params/robo_s1.json`) | a lente (`camera`, uma só: a nominal até a calibração a medir; o simulador usa a mesma, `Fisica::camera_no_simulador`), onde ela fica (`camera_no_corpo`, da malha do app) e o cano do lançador que ela vê (`cano_na_lente`, da malha; só na câmera do robô) |
| uma amostra | o `rms1d`, por `s1.subscribe` | a hora, o giro e a inclinação da torre (a inclinação em relação ao chão) e a pose `[x, y, rumo]` |
| uma grade | a estação, pela entrega `mapa` | 16 bytes de cabeçalho e a grade, como o leitor de profundidade a escreve; com o nome do leitor e a hora em que o quadro foi tirado |
| uma falha | a marca do quadro, da estação | a hora e o motivo |

| Sai | Para quem | Forma |
|---|---|---|
| `Mundo::agora(t)` | `s1.world` | a folga por setor, o obstáculo `{ahead_m, away, age_s}`, o estado da percepção com o motivo, de onde veio a escala, a idade do quadro e o campo da lente |
| `Mundo::linha_da_grade(t)` | `s1.world.grid`, para a tela | as células vistas e ainda lembradas, na odometria, com a pose |

A hora é sempre a de quem chama, em segundos de um relógio que não volta. O mesmo relógio para as amostras e para a hora de
captura das grades.

## 2. As contas

**A geometria (`geometria.rs`).** Um ponto do quadro, normalizado pela lente (`xn` para a direita, `yn` para baixo), é o
raio `[1, -xn, -yn]` no referencial da lente (frente, esquerda, cima), e a profundidade `z` é medida ao longo do eixo da
lente, como a dos modelos. A lente gira pela inclinação em volta do eixo de inclinação da torre (e por isso a altura dela
muda com a inclinação), depois pelo giro da torre, e o corpo é posto na pose. A linha `yn` desce `yn·cos(i) - sen(i)` por
metro de profundidade: o chão, ali, está a `altura / (yn·cos(i) - sen(i))`.

**A grade (`grade.rs`).** Só a do `scdepthv3` vira metros. O número dela é o de antes da conta da Hailo: o inverso da
profundidade, a menos de uma escala, é `10·sigmoide(x) + 0,009`. A conta é feita uma vez por valor possível (65.536, em
16 bits) e guardada.

**A escala pelo chão (`escala.rs`).** Num ponto que é chão, `escala = z_chao · u`, com `u` o inverso da profundidade que a
grade diz. O que está mais perto que o chão puxa essa razão para cima. Na faixa de baixo do quadro (`semente_de`), a escala
é a mediana das razões que ficam a menos de `tolerancia` do quartil de baixo. Ela só vale se:

1. a fração da faixa que entrou na conta passa de `chao_minimo`; e
2. ela bate, dentro de `reta_tolerancia`, com a escala tirada por outro caminho: a inclinação da reta de `u` contra `yn`,
   que no chão vale `escala·cos(i)/altura` e não depende de onde o horizonte está. Com uma parede enchendo a faixa, ou com
   a grade casada com uma inclinação muito errada, as duas se separam. O desvio entre elas é perto de 0,8 do erro que a
   escala teria (medido nas cenas desenhadas); em imagem real o chão do modelo é plano só a menos de uns 10%, e por isso o
   limite é largo: a conferência recusa a parede e o erro grande, e não mede o pequeno.

Os pontos da grade que olham para o cano do lançador (`Raios::tapado`) ficam fora da faixa e da conta, e não viram ocupado.

Quando o quadro não dá escala, vale a mediana das escalas boas dos últimos `escala_vale_ms`, e o mundo diz que ela é
lembrada; sem nenhuma, não há metros (`no_floor`).

**O chão não é ajustado livremente.** Medido na grade guardada em 2026-11-20, o horizonte de um plano ajustado à grade vai
da linha 11 à 108 de 180 conforme a faixa que se toma (no laboratório, `out/2026-11-20-distancia-na-estacao/reta_do_chao.py`).

**A ocupação (`ocupacao.rs`).** Células de `celula_m`, na odometria, numa janela de `janela_m` que anda com o robô. Cada
ponto da grade, em metros, é chão (a célula em que o raio para foi vista livre, e o trecho entre dois pontos de chão
seguidos da coluna), objeto (mais perto que o chão: candidato a ocupado, se a altura dele fica entre `altura_minima_m` e
`altura_maxima_m` e ele é o primeiro de dois seguidos na coluna) ou nada (mais longe que o chão). O livre nunca é marcado
além de onde o raio para: marcar no chão geométrico pintava de livre o que fica atrás da base de um objeto (achado nos
testes, e consertado).

**A folga e o obstáculo (`folga.rs`).** Do mapa, reexpresso no corpo de agora. Cada setor diz a distância, do centro do
corpo, até o ocupado mais perto; sem ocupado, até onde o chão foi visto livre sem interrupção; ou que nada foi visto. O
obstáculo é a folga da frente do chassi ao ocupado mais perto no corredor da largura do corpo (mais uma célula), o lado
mais livre, e a idade.

## 3. O que veio do cérebro do pato

| No pato (`autod/src/obstacle.rs`) | Aqui |
|---|---|
| `History`: um segundo de amostras, e o quadro casado com a mais perto da captura | `historia.rs`, igual |
| o confirmado mora na odometria e é reexpresso no corpo a cada passo | o mapa mora na odometria; a folga e o obstáculo saem dele a cada pedido |
| um ponto só vale com vizinho, no mesmo quadro ou no anterior | uma candidata só vale com outra, de outra coluna da grade, na mesma célula ou nas oito em volta, neste quadro ou no anterior |
| "livre" não apaga o lembrado | a célula ocupada só volta a livre vista livre em `confirma_livre` quadros seguidos |
| o lembrado é esquecido depois de `STALE_S` | depois de `lembra_s`, para o livre e para o ocupado |
| `Obstacle {ahead_m, away, age_s}`, do bico | o mesmo, da frente do chassi |
| `FRESH_S` 0,9 s e `STALE_S` 1,5 s | `fresco_s` e `velho_s` nascem com os mesmos, ditos como do pato; quem os acerta para o S1 é o cérebro dele |

O que o S1 tem a mais: a grade inteira no lugar de 64 zonas, o chão visto como livre (e não só o obstáculo), a folga em
todas as direções (o chassi anda para qualquer lado), e a torre inclinando para ver o chão de mais perto.

## 4. O estado da percepção

`ok`, `stale` (sem grade há mais de `velho_s`), `failed` (a estação disse a falha, com o motivo), `no_floor` (a grade
chegou, e não há escala), `no_metres` (o leitor ligado não é o `scdepthv3`), `no_pose` (sem amostra perto do instante do
quadro, por `torre_vale_ms`) e `no_camera` (a tabela sem a lente). À parte do estado, `lens.calibrated` diz se a lente já foi
medida: com a provisória o mundo aparece do mesmo jeito, e quem mostra os metros diz que não estão calibrados. Em qualquer estado o mapa segue com o que lembra, cada célula com a idade.

## 5. O medido nas cenas desenhadas (`cargo test -p rms1-mundo`, 2026-11-20)

- **A distância:** uma caixa a 0,6, 1 e 2 m, com a torre a -20°, -10°, 0° e +5°, com a escala do modelo em 0,05, 0,1 e 0,2,
  pela lente do simulador e por uma de teste: a escala e a distância saem com erro de até 2%; com 5% de ruído em cada
  ponto, até 5%; com uma caixa tomando 60% da faixa de baixo, até 2%.
- **O limite da conferência da escala:** com a grade casada com a inclinação errada por 10° ou mais, nenhum quadro é dado
  por medido. Um erro de 2° passa, e custa de 3,5% a 10,7% de escala, conforme a lente e a inclinação; um de 5° é recusado
  em quase todos os casos, e nos que passam custa até 10,2%. **A inclinação da torre, e o zero dela, valem muito:** é o que
  a calibração no robô tem de medir bem.
- **Não medido:** nada disto no robô. A lente do robô, o zero da inclinação, o atraso do quadro, a estabilidade da escala do
  `scdepthv3` entre quadros, o erro contra a trena e o tempo de tratar uma grade no processador do S1 são da calibração.

## 6. Os ajustes (`[midia.mundo]`, em `rms1d.toml`)

Os padrões são de partida, e a origem de cada um está em `rms1d-params/src/lib.rs`, em `Default` de `MidiaMundo`: `ritmo`,
`ritmo_da_grade`, `setores`, `alcance_m`, `semente_de`, `tolerancia`, `chao_minimo`, `reta_tolerancia`, `corpo_folga_m`,
`escala_vale_ms`, `celula_m`,
`janela_m`, `altura_minima_m`, `altura_maxima_m`, `confirma_livre`, `lembra_s`, `fresco_s`, `velho_s`, `torre_vale_ms`,
`gravar` e `gravar_limite_mb`. Cada um tem faixa, e o valor fora dela é recusado pelo nome.

## 7. Gravar e reproduzir

Com `gravar`, o mundo grava a sessão: a câmera e o corpo, cada amostra, cada grade como chegou e cada falha, com a hora
(`gravacao.rs` diz o formato). A grade do `scdepthv3` tem 115 kB: a 15 por segundo são 104 MB por minuto, e a gravação para
sozinha em `gravar_limite_mb` (512 por padrão), sem parar o serviço.

```sh
cargo run -p rms1-mundo --bin rms1-mundo-reproduz -- sessao.mundo --calado --esperado 0.84
```

O programa passa a sessão pelo mesmo caminho do robô e escreve o que `s1.world` diria depois de cada grade. Com
`--esperado`, confere a mediana da folga à frente contra a distância dada, pela regra da frente (10% ou 5 cm), e termina em
`REPRODUCAO CONFERE`. Com `--camera`, troca a câmera da gravação: é como a calibração experimenta uma lente ou um atraso
sem voltar ao robô.

O que a calibração usa a mais:

- `--janela <de> <até> <metros>`, quantas vezes for preciso: a mesma conferência só nos mundos entre as duas horas da
  gravação, contra a distância daquela janela (a caixa em várias marcas, numa sessão só). O veredito é um só: `REPRODUCAO
  CONFERE` se todas as janelas, e o `--esperado`, ficam dentro da regra.
- `--exame`: cada linha leva também `exam` e `fresh`. `exam` é a escala que levou a grade aos metros (`scale`), de onde ela
  veio (`origin`: `measured` ou `remembered`) e o exame da faixa de baixo do quadro (`frame`): quantos pontos a faixa tem
  (`total`), quantos podiam ser chão (`points`) e quantos ficaram perto do quartil de baixo (`kept`); a escala pela razão
  (`ratio`) e pela inclinação da reta (`line`), o desvio entre as duas (`deviation`) e a dispersão (`dispersion`); e por que
  a escala do quadro não valeu (`refused`: `few_points`, `scattered`, `no_slope` ou `deviation`; nulo, se valeu). `fresh` é o
  meio de cada célula que aquele quadro marcou como ocupada, na odometria, em metros.

No código, o exame é `escala::examinar` (a conta da escala passo a passo; `escala::do_quadro` é o exame aceito), cada grade
devolve em `Tratada` o exame e a escala usada, e `gravacao::reproduzir_em_passos` devolve, de cada grade, o mundo, o
`Tratada` e as ocupadas do quadro. As ferramentas da calibração, no laboratório (`tools/calibracao/`), leem estas linhas.

## 8. Provar

```sh
nice -n 19 cargo test -j 2 -p rms1d-params
nice -n 19 cargo test -j 2 -p rms1-mundo
```

As duas contraprovas (o roteiro `refute.sh` do rito restaura o arquivo e confere a impressão digital): tirar a altura da
câmera da conta do chão (`escala.rs`) tem de reprovar com "a 1,00 m"; contar todo ponto como chão (`ocupacao.rs`) tem de
reprovar com "barrado".
