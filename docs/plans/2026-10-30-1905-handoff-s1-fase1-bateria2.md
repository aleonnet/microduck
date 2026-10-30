---
status: aceito
---

# Handoff — S1 ← microduck, Fase 1, fim da bateria 2 (2026-10-30, 19:05)

Escrito a pedido do dono para outra sessão continuar após `/clear`. A conversa não é o estado; o
disco é. Leia nesta ordem antes de qualquer ato:

1. `~/.claude/protocolo/PROTOCOLO.md` e o `MEMORY.md` do projeto (os hooks injetam; releia mesmo assim).
2. Memória viva da bancada: `~/.claude/projects/-Users-alessandro-Development-microduck/memory/s1-bancada-2026-10-30.md`
   (último bloco = esta tarde/noite) e `feedback-bronca-e-freio-param-o-turno.md` (as regras do dono, com as frases dele).
3. O registro `docs/project/s1-bringup.md`, seções "Robot session, 2026-10-30 (night)", "Captures of 17:05 and 17:12",
   "Stage 7, the camera", "Stage 8, audio", "Before the next battery".
4. `git log --oneline -12` no branch `s1-fase1` (fork `aleonnet/microduck`); árvore limpa no commit deste handoff.

## Estado medido

- Branch `s1-fase1`, frente da Fase 1 aberta pelo rito Roadworthy (plano `pasted-content-id-caf1-retomando-o-cuddly-bear.md`,
  base `4d9ca3c`, 10 globs, 8 portões). Portões de código verdes no último commit de código (72 testes, clippy 0, fmt);
  `verify-record` 83 OK; lychee 0; o portão de escopo acusa só `docs/plans/2026-10-30-0045-handoff-overnight-s1-fase1.md`
  (artefato do rito) — e agora este arquivo, pela mesma razão.
- Robô: ligado quando este handoff foi escrito, stock parada (`dji_hdvt_uav` morto por `bench.sh stop`), `dji_camera`
  parado, nossos daemons NÃO rodando. Root adb cai no power-cycle: reabrir com o app DA DJI FECHADO via
  `cd ~/Development/robomaster-s1-hacks && python3 lab-cli/scripts/s1_lab.py --host 192.168.1.236 run root-adb/scripts/enable_root_adb.py; adb connect 192.168.1.236:5555`.
- Laboratório (artefatos, não é git): `~/Development/RoboMaster-S1/out/2026-10-30-poc2/` — cada estágio em sua pasta;
  corridas anteriores arquivadas como `<estágio>-<carimbo>`.

## Provado no robô, com o dono vendo (não retestar)

Pad fim a fim (Start, sticks, strafe, gimbal com sinais da DJI, Home, gatilhos), rodas F16, deadman, LEDs do corpo F10,
luz de trajetória F9, lanterna, SOM (quack, wheee, som do disparo, wavs da stock — o dono ouviu VÁRIAS vezes; nunca
mais perguntar), eventos (placas 1–4, botão códigos 264/265, infravermelho recebido), F17 completo (disparo + flash
verde `0x3f/0x55` ctrl 7 efeito 3 + som), `s1ctl version`. Capturados da UART com a stock viva: modos de seguir
(gimbal workmode 0 = chassi segue gimbal; 2 = gimbal segue chassi, repetido a 8 Hz), quadros de conexão
(`3f/19 00`, `3f/66 0200`), o flash do lançador, o desligar pelo botão (`capture-follow/sys_poweroff.strace`, ainda
não lido).

## Aberto, com a próxima medição definida

| Item | Fato medido | Próximo passo (sem hipótese) |
|---|---|---|
| Câmera (est. 7, 11, 12) | `v4l2-ctl --stream-mmap` REINICIA o robô (2×). Sequência da stock decodificada em `camera/dji_camera_v4l2_decoded.strace`: QUERYCAP(comip) → S_INPUT 0 → S_CTRL CAMERA+0x191b=0, +0x1934=30, +0x1935=0 → S_FMT 1280x720 NV12 field ANY colorspace JPEG → REQBUFS MMAP 5 → QUERYBUF ×5 → STREAMON → DQBUF/QBUF → S_CTRL +0x1928 | Escrever `s1-cam` (bin em `s1d`, ioctls via libc) que replica isso byte a byte, salva N quadros NV12 e mede fps; testar no corpo falso onde couber; depois no robô (power-cycle antes: após matar o dji_camera, `open(/dev/video0)` dá ENODEV até reboot). Nunca mais ferramenta genérica no stream. |
| Mic (est. 8) | Dispositivo de captura = `hw:0,0` (a stock abre `pcmC0D0c`); canal esquerdo = constante DC, direito = piso de ruído; loopback com o próprio alto-falante não chega (−73 dB RMS). `hw:0,1` (comip_voice) aceita S16 1–2 canais 8–16 kHz, não testado. Mixer: `I2sL Out`=ADC2, `I2sR Out`=ADC1, `PcmL/R Out`=PDM; dumps iguais com dji_camera parado/rodando | Medir o que a stock liga quando o APP grava áudio: dump do `amixer contents` durante uma gravação de voz no app (20 s) e `strace -e ioctl` do `dji_camera`/`dji_hdvt_uav` nesse instante; comparar com `audio/amixer_contents.txt`. Só então mudar algo no mixer. |
| Telemetria F18 | add_node/add_sub recusados (`02 03`/`21 03`) com nó 9 e 0xC9; reset de nó MATA o barramento até power-cycle (nunca mandar); `stop dji_scratch` matou o `dji_sys` (nunca mais) | Ler `rm_module.py` (`Tank.set_sub_node`/`add_gimbal_and_chassis_sub_msg`: via controlador da stock, cmdset 0x48, uuid alto 0x20009) e capturar a assinatura repassada pela stock à UART num boot com strace de escritas do hdvt desde o início (init.rc: `dji_hdvt_uav -g`). |
| F5 dormir/acordar, F6 goto, F12 PWM | bytes do oráculo da DJI; nunca rodados no robô | `bench.sh drive` → `s1ctl gimbal sleep/wake`; goto no chão com 2 m; PWM com carga. |
| Est. 10 restauração | stock volta sozinha após reboot (medido 2× hoje) | gravar `restore/after.txt` com `bench.sh restore` numa ligada que termine com a stock viva. |

## Regras do dono que valem para quem continuar (ditas hoje, literais na memória)

- Nenhum quadro ao robô sem captura da UART ou oráculo da DJI byte a byte. Tentativa e erro é PROIBIDO.
- Nunca pedir ao dono ação (falar, apertar, abrir o app) antes de esgotar o que se mede sem ele; nunca perguntar de novo o que ele já reportou.
- Mensagens só com resultado; "aguarde" não existe. Bronca ou freio = responder curto e terminar o turno, zero comandos.
- Captura com a stock viva: só escritas do `dji_hdvt_uav` (`-e trace=write`), senão o app fica inutilizável; a linha "static capture done" é o sinal para o dono abrir o app (nunca esconder a saída atrás de `tail`).
- Usar SEMPRE os estágios do `bench.sh` (eles remontam o chroot; comandos ad hoc sem `/dev`, `/proc`, `/sys` falham com "Cannot open device").

## Prompt para colar na próxima sessão

```
Retome a frente Fase 1 do S1 (branch s1-fase1). Leia PRIMEIRO docs/plans/2026-10-30-1905-handoff-s1-fase1-bateria2.md,
depois a memória s1-bancada-2026-10-30.md e o registro docs/project/s1-bringup.md (seções da noite de 30/10).
Declare o que leu e o estado confirmado por comando (git log, git status, adb get-state). Próximo passo sem robô:
escrever o s1-cam que replica a sequência V4L2 da stock. Nada vai ao robô sem captura ou oráculo.
```
