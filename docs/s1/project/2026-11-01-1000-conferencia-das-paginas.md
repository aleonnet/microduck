---
status: aceito
---

# Conferência das páginas do S1, frase contra código (2026-11-01)

Ordem do dono: "feche a arrumação com qualidade". Feita por mim, antes do leitor frio. "Conferido" quer dizer que abri a fonte, não que não sobrou erro. Nada aqui tocou o robô.

| Página | Conferido contra | Corrigido nesta passada | Fica "não medido" |
|---|---|---|---|
| `robot/s1-commands.md` | `s1d/src/ipc.rs`, `api.rs`, `tick.rs`, `intents.rs` inteiros; corpo de `s1d/tests/dispatch.rs`, `s1_api.rs`, `fake_body.rs`; funções de `pad-map/src/mapper.rs`; `robotctl`, `netpadd`, `mediad` nos pontos citados; `drive/e2e.log`, `drive/move.log`, `items/e2e.log` | quem envia e o que os logs guardam; `robot.relax` só com teste de rota; `robot.init` também na corrida falsa; `robot.policies` sem artefato no robô; Select na soltura; bytes de seguir; defeito novo: `robot.stop` não para a taxa do gimbal | sentido de `s1.goto`; efeito de `s1.armor`, `s1.pwm`, `s1.gimbal sleep`; tudo do navegador |
| `design/s1-body.md` | `s1d/src/tick.rs`, `state.rs`, `main.rs`, `alsa.rs`, `button.rs`, `sound.rs`; `scripts/s1/bench.sh`, `s1-push.sh`; `deploy/s1/` | condição de `s1.wheels`; estado depois da junção | a arquitetura alvo inteira, que é proposta |
| `robot/s1-bench.md` | `scripts/s1/bench.sh` e `s1-push.sh`, passo a passo, e `scripts/s1/bench-selftest.sh` | `S1_FRAMES`; quais scripts usam `sed -i ''` | `up`, `floor`, `app`, restaurar imagem |
| `project/s1-bringup.md` | texto inteiro; artefatos de texto abertos; capturas decodificadas de novo com `duml-strace.py` | 29, 19, 19 e 48 comandos em vez de 30, 20, 20 e 50; 1314 de 2361 intervalos; quadros do lançador na serial, antes dados como nada; faixa do yaw no campo 8; nariz abaixo −0,32 | ver abaixo |
| `project/s1-changelog.md` | `git log`, `git show --stat`, tabela de hashes | feito nas rodadas anteriores | agrupamento é leitura, só em parte cercado |
| Comentários e ajuda | as 56 linhas da varredura, uma a uma, e os arquivos em volta | `button.rs`, `sound.rs`, `state.rs`, `tick.rs`, `api.rs`, `ipc.rs`, `fake_body.rs`, `s1_api.rs`, `params.rs`, `robotd.toml`, `s1-push.sh`, `dji-oracle.py`, `bench.sh`, `cmd.rs`, `real_frames.rs`, `stock_push.rs`; cinco mensagens de recusa, não quatro como o plano dizia, porque a de `robot.model` também citava fase | "Phase 1" no nome do `s1-e2e` fica: fase é vocabulário definido no diário técnico |

**Refeito agora a partir dos artefatos, por `~/.claude/plans/2026-11-01-1000-conferencia-artefatos.py`, que só lê:** 2362 + 59 empurrões; 1388 do gimbal e quatérnion em −94,7° e 122,6°; modo 2 em 122 quadros; sequência de 395 quadros do driver e luma média 94 da câmera; 76 valores do oráculo; `audio.ini` 48000, 480, 4, 16; eco −51 a −40 dB; rota 16 kHz −50,8 dB; biblioteca NEON 19 188 004 bytes e 1070 regras; 53 amostras de `RUNNING`; golpes 6, 5, 4, 8; `dji_sys` morto às 16:59:24, pid 246, de volta como 27689. Saída em `~/.claude/plans/2026-11-01-1000-conferencia-artefatos.saida.txt`.

**Não refeito, e por quê:**
- As 5442 ordens de bloco da telemetria: frase do diário, as capturas existem, o teste só cobre as amostras em `s1-duml/tests/stock_push.rs`.
- Valor de `robot.health` às 23:5x e `pidof` às 15:43: procurados com `grep` nos `.txt`, `.log` e `.json` de 30/10, sem resultado; capturas grandes não varridas; o texto fica como relato daquela hora.
- Páginas externas (Hackaday, wiki HU-ICT-LAB, `robomaster_sim`, SDK da DJI): citadas, não reabertas.
- Áudio, vídeo, imagens e imagens de partição: abertos só para os níveis de quatro gravações (eco, rota, `pet_run2.wav`, áudio do vídeo da DJI), a luma de três quadros e os bytes de `audio.ini`; o resto é hash; o que foi ouvido ou visto é testemunho do dono.
- Firmware da DJI: só onde a frase afirma algo sobre ele (`rm_ctrl.py`, cadeias de `dji_sys`, `dji_hdvt_uav`, `dji_camera`).

**Depois do leitor frio:** ele achou 12 erros meus nesta passada, todos conferidos e corrigidos; o caminho errado de captura foi varrido em todo o S1. Lista: `~/.claude/plans/pasted-content-id-b9f7-leia-inteiros-pure-badger.diff-review-r3.md`. Uma segunda e última leitura, só sobre essas correções, achou mais 7, também conferidos e corrigidos (`...diff-review-r4.md`, ao lado); essas 7 correções não tiveram leitor depois. Isto mostra que a releitura minha não bastou sozinha; a lista prova o que foi conferido, não que não sobrou erro.
