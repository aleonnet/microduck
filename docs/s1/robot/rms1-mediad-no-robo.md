# `rms1-mediad` no robô: a ativação, o que fica onde, e a volta

Status: em uso no S1 desde 2026-11-15. O desenho do serviço está em [`../design/rms1-mediad.md`](../design/rms1-mediad.md); a prova de cada peça no robô, com os números, está no laboratório (`~/Development/RoboMaster-S1`), em `out/2026-11-14-s1-midia/` e no runbook `docs/2026-11-16-1515-runbook-midia-s1.md`.

## O que a ativação faz pela mídia

`scripts/s1/rms1d-bench.sh up` (ou o botão "Ativar robô" do simulador) cuida da mídia quando `deploy/s1/rms1d.toml` tem `[midia]`. Na ordem:

1. **O `rms1-mediad` anterior sai primeiro, por `SIGTERM`.** É assim que ele desliga o fluxo da câmera antes de soltá-la. Se não sair, a ativação para: "o rms1-mediad anterior não saiu; desligue o robô na chave".
2. **O `dji_camera` sai só pelo `init`,** nunca por `kill`: morto no meio do fluxo, ele deixa a câmera travada até reiniciar. Ele está em `[dji] parar`, ao lado do `dji_monitor` e do `dji_hdvt_uav`. Se não sair pelo `init`, a ativação para e pede para reiniciar o robô.
3. **A mídia sobe antes do `rms1d`:** o binário vai para `/opt/robot/bin/rms1-mediad` do chroot (só se mudou), o `deploy/s1/asound.conf` para `/etc/asound.conf`, e os sons da DJI são copiados de `/system/audio/sound/` para `/var/lib/robot/sounds/`. O serviço sobe com `--config /etc/robot/rms1d.toml`, e o registro dele fica em `/data/rms1-mediad.log`, no robô.
4. **O codificador de hardware sobe antes do `rms1-mediad`,** que é o cliente dele: com `[midia] venc` no arquivo, o `rms1-venc` (`target/s1-android/rms1-venc`, de `sh rms1-mediad/venc/compila.sh`) vai para `/data/rms1-venc`, do lado Android do robô, e sobe em `servir` no socket do arquivo. O de antes sai por `SIGTERM`, depois do `rms1-mediad`. Se o binário não foi compilado, ou o programa não sobe, a ativação avisa e segue, e o quadro vai cru à estação. Com o robô já ativado, um `rms1-venc` caído faz a ativação preparar de novo, em vez de só conectar.
5. **Este Mac ganha a ponte até ele:** uma porta do `adb` e o socket `media.sock` na pasta de estado da bancada (`/tmp/rms1d-bench`, por padrão), como o `robot.sock` do `rms1d`.

**Uma falha da mídia não impede pilotar.** Se o serviço não subir, a ativação avisa com a última linha do registro e segue: a câmera, o microfone e o som ficam fora. Se o arquivo tem `[midia]` e não manda parar o `dji_camera`, a mídia não é ligada, e a ativação diz isso.

## O que o arquivo do robô liga

`deploy/s1/rms1d.toml`, seção `[midia]`:

| Chave | Valor no robô | O que faz |
|---|---|---|
| `mixer` | `true` | o serviço grava no chip de áudio o estado que a partida da DJI deixa, e confere lendo de volta (`mixer.rs`); um valor que não pega impede o serviço de subir |
| `porta_video` | `47010` | a porta TCP por onde a tela busca a imagem em JPEG, direto pela rede; por ela só se olha |
| `venc` | `"/run/rms1-venc/venc.sock"` | o socket do `rms1-venc`, o codificador de hardware: com ele o quadro vai à estação em H.264 ou em JPEG, o que ela pedir, e a imagem da tela sai do chip |
| `[midia.camera] fonte` | `"s1"` | a câmera aberta pelos 27 passos da DJI, cada resposta conferida com a gravada (`camera_s1.rs`) |
| `[midia.microfone] fonte` | `"arecord"` | o microfone pelo `arecord` do chroot; se o programa cai, o serviço o sobe de novo |
| `[midia.alto_falante] saida` | `"aplay"` | o alto-falante pelo `aplay` do chroot |

## A volta

`scripts/s1/rms1d-bench.sh volta` manda `SIGTERM` ao `rms1-mediad`, ao `rms1d` e ao `rms1-venc`, guarda a gravação, encerra as pontes deste Mac e reinicia o robô: a DJI volta inteira, com o `dji_camera` dela. Só desligar na chave fecha a porta root.

## O que não se faz

- Não matar o `dji_camera` nem o `rms1-mediad` com `kill -9`: a câmera fica respondendo `ENODEV` até reiniciar.
- Não abrir a câmera com um fluxo genérico (`v4l2-ctl --stream-mmap`): dois fluxos assim reiniciaram o robô em 2026-10-30.
- Não parar o `dji_scratch`: a lista `[dji] parar` o recusa (`rms1d-params`).
