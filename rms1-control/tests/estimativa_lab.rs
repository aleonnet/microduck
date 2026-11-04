//! O cálculo próprio contra as gravações do robô de 2026-11-03, nos dois modos: o controlador da DJI medido por fora
//! (`strace` de `dji_hdvt_uav`) e o `rms1d` ativo (`strace` do `rms1d`). As gravações são do laboratório, que é privado:
//! não rodam no `cargo test` comum. `scripts/test-rms1-telemetria.sh` roda estes testes, e reprova sem o laboratório.
//!
//! Os trechos são os do roteiro do dono, anotados por ele e achados nos dados (`docs/2026-11-02-1620-levantamentos-s1.md`,
//! no laboratório), em segundos desde o primeiro pacote: no ar e inclinado, de lado na mão, levantado nivelado, e andando
//! pelo app.

use std::path::PathBuf;

use rms1_control::estimativa::{Linha, de_recebido, de_strace};
use rms1_control::gravacao::ler_recebido;
use rms1_control::telemetria::{BIT_LEVANTADO, BIT_PARADO};
use rms1d_params::Params;

fn gravacao(nome: &str) -> Vec<Linha> {
    let lab =
        PathBuf::from(std::env::var("S1_LAB").expect("S1_LAB tem de apontar para o laboratório"));
    let c = lab.join("out/2026-11-03-firmware-controlador").join(nome);
    let texto = std::fs::read(&c).unwrap_or_else(|e| panic!("{}: {e}", c.display()));
    de_strace(&texto, &Params::default()).linhas
}

fn modo_dji() -> Vec<Linha> {
    gravacao("ctrl_fora.strace")
}

fn modo_nosso() -> Vec<Linha> {
    gravacao("rms1d_fora.strace")
}

/// O teste do dono no robô em modo sombra, 2026-11-03: manuseios de 145 a 186 s, e andando a até 2,9 m/s, com freadas fortes,
/// de 330 a 470 s. É o caso que derrubou o levantado com 300 ms: as rodas param antes do robô.
fn sombra() -> Vec<Linha> {
    let lab =
        PathBuf::from(std::env::var("S1_LAB").expect("S1_LAB tem de apontar para o laboratório"));
    let c = lab.join("out/2026-11-03-firmware-controlador/sombra/recebido.txt");
    de_recebido(
        &ler_recebido(&c).unwrap_or_else(|e| panic!("{}: {e}", c.display())),
        &Params::default(),
    )
    .linhas
}

/// No robô, em modo sombra: levantado nos manuseios, e nunca andando, nem freando forte.
#[test]
#[ignore = "precisa do laboratório; rode scripts/test-rms1-telemetria.sh"]
fn no_robo_em_sombra_freada_nao_e_levantado() {
    let l = sombra();
    assert_eq!(l.len(), 25686, "os pacotes da gravação");
    let marcado = |a: f64, b: f64| {
        l.iter()
            .filter(|x| x.t >= a && x.t <= b)
            .any(|x| x.calculo.levantado)
    };
    assert!(marcado(145.0, 186.0), "manuseio sem levantado");
    let andando: Vec<f64> = l
        .iter()
        .filter(|x| x.t >= 330.0 && x.t <= 470.0 && x.calculo.levantado)
        .map(|x| x.t)
        .collect();
    assert!(
        andando.is_empty(),
        "levantado andando, freando: {:?}",
        andando.first()
    );
}

/// Os manuseios: (início, fim) em segundos.
const MANUSEIOS_DJI: [(f64, f64); 3] = [(35.0, 41.0), (52.0, 61.0), (75.0, 82.0)];
const MANUSEIOS_NOSSO: [(f64, f64); 2] = [(46.0, 55.0), (69.0, 79.0)];
const ANDANDO_DJI: (f64, f64) = (114.0, 154.0);

fn plano(v: [f64; 3]) -> f64 {
    v[0].hypot(v[1])
}

#[test]
#[ignore = "precisa do laboratório; rode scripts/test-rms1-telemetria.sh"]
fn as_gravacoes_tem_os_pacotes_medidos() {
    assert_eq!(modo_dji().len(), 5226);
    assert_eq!(modo_nosso().len(), 7410);
}

/// Critério 16: com o controlador acima de 2 m/s, o robô estava na mão, e a nossa velocidade fica abaixo de 0,1 m/s.
#[test]
#[ignore = "precisa do laboratório; rode scripts/test-rms1-telemetria.sh"]
fn na_mao_a_velocidade_falsa_do_controlador_nao_entra_na_nossa() {
    for (nome, l) in [("modo DJI", modo_dji()), ("modo nosso", modo_nosso())] {
        let falsos: Vec<&Linha> = l
            .iter()
            .filter(|x| plano(x.medidas.velocidade_corpo) > 2.0)
            .collect();
        assert!(
            falsos.len() > 100,
            "{nome}: {} pacotes acima de 2 m/s",
            falsos.len()
        );
        for x in falsos {
            let v = x.calculo.velocidade_corpo[0].hypot(x.calculo.velocidade_corpo[1]);
            assert!(v < 0.1, "{nome}, {:.2} s: nossa {v:.3} m/s", x.t);
        }
    }
}

/// Critério 17, no robô: parado pelo controlador, a nossa velocidade é zero.
#[test]
#[ignore = "precisa do laboratório; rode scripts/test-rms1-telemetria.sh"]
fn parado_pelo_controlador_e_zero() {
    for l in [modo_dji(), modo_nosso()] {
        let parados: Vec<&Linha> = l.iter().filter(|x| x.estado & BIT_PARADO != 0).collect();
        assert!(parados.len() > 1000);
        assert!(
            parados
                .iter()
                .all(|x| x.calculo.velocidade_corpo == [0.0, 0.0])
        );
    }
}

/// Critério 18: andando no chão pelo app, a nossa velocidade fica a até 0,1 m/s da do controlador, em média por segundo.
#[test]
#[ignore = "precisa do laboratório; rode scripts/test-rms1-telemetria.sh"]
fn andando_a_nossa_velocidade_bate_com_a_do_controlador() {
    let l = modo_dji();
    for s in 118..126 {
        let seg: Vec<&Linha> = l
            .iter()
            .filter(|x| x.t >= f64::from(s) && x.t < f64::from(s + 1))
            .collect();
        assert!(!seg.is_empty(), "{s} s sem pacote");
        let n = seg.len() as f64;
        let dji = seg
            .iter()
            .map(|x| plano(x.medidas.velocidade_corpo))
            .sum::<f64>()
            / n;
        let nossa = seg
            .iter()
            .map(|x| x.calculo.velocidade_corpo[0].hypot(x.calculo.velocidade_corpo[1]))
            .sum::<f64>()
            / n;
        assert!(
            (dji - nossa).abs() <= 0.1,
            "{s} s: controlador {dji:.3}, nossa {nossa:.3}"
        );
    }
}

/// Critério 19: cada manuseio tem levantado; andando, nenhum.
#[test]
#[ignore = "precisa do laboratório; rode scripts/test-rms1-telemetria.sh"]
fn levantado_em_todo_manuseio_e_nunca_andando() {
    let marcado = |l: &[Linha], (a, b): (f64, f64)| {
        l.iter()
            .filter(|x| x.t >= a && x.t <= b)
            .any(|x| x.calculo.levantado)
    };
    let dji = modo_dji();
    for m in MANUSEIOS_DJI {
        assert!(marcado(&dji, m), "modo DJI, manuseio {m:?} sem levantado");
    }
    let nosso = modo_nosso();
    for m in MANUSEIOS_NOSSO {
        assert!(
            marcado(&nosso, m),
            "modo nosso, manuseio {m:?} sem levantado"
        );
    }
    assert!(!marcado(&dji, ANDANDO_DJI), "levantado andando");
    // O controlador não marca nenhum: é o motivo do cálculo próprio.
    assert!(
        dji.iter()
            .chain(&nosso)
            .all(|x| x.estado & BIT_LEVANTADO == 0)
    );
}

/// Critério 20: nas gravações, a atitude do controlador e a aceleração concordam com o robô quieto: nenhum aviso.
#[test]
#[ignore = "precisa do laboratório; rode scripts/test-rms1-telemetria.sh"]
fn nas_gravacoes_a_gravidade_confere_sem_aviso() {
    for (nome, l) in [("modo DJI", modo_dji()), ("modo nosso", modo_nosso())] {
        let quietos = l.iter().filter(|x| x.calculo.divergencia.is_some()).count();
        assert!(quietos > 1000, "{nome}: {quietos} pacotes quietos");
        let aviso = l.iter().find(|x| x.calculo.divergente);
        assert!(aviso.is_none(), "{nome}: {aviso:?}");
    }
}

/// O teste do dono de 2026-11-03, às 18:50: armado, seguindo a torre, sem pilotar, ele levantou o robô e as rodas foram a 850 rpm,
/// porque o `rms1d` repetia o seguir a zero a cada ciclo (`out/2026-11-03-firmware-controlador/levantado/`). A gravação do que o
/// robô mandou passa aqui pelo laço inteiro, como no robô: o seguir a zero vai uma vez, e do levantado em diante só vão rodas a zero.
#[test]
#[ignore = "precisa do laboratório; rode scripts/test-rms1-telemetria.sh"]
fn no_robo_levantado_seguindo_a_torre_as_rodas_param() {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use rms1_control::intencoes::Modo;
    use rms1_control::{CorpoFalso, Intencoes, Laco, Saude};
    use rms1_proto::comando::id;
    use rms1d_params::TipoCorpo;

    let lab =
        PathBuf::from(std::env::var("S1_LAB").expect("S1_LAB tem de apontar para o laboratório"));
    let c = lab.join("out/2026-11-03-firmware-controlador/levantado/recebido.txt");
    let leituras = ler_recebido(&c).unwrap_or_else(|e| panic!("{}: {e}", c.display()));
    let mut p = Params::default();
    p.corpo.tipo = TipoCorpo::Serial;
    p.armadura.sensibilidade = 10;
    let corpo = CorpoFalso::default();
    let intencoes = Arc::new(Intencoes::default());
    let mut laco = Laco::new(
        corpo.clone(),
        &p,
        intencoes.clone(),
        Arc::new(Saude::default()),
    );
    intencoes.pedir_armar(true);
    intencoes.pedir_modo(Modo::ChassiSegueTorre);
    let base = Instant::now();
    let (mut seguir, mut no_ar, mut placas) = (0, Vec::new(), Vec::new());
    for (ms, bytes) in &leituras {
        corpo.a_ler.lock().unwrap().extend_from_slice(bytes);
        let agora = base + Duration::from_millis(*ms);
        while {
            let e = laco.ciclo(agora, true).expect("estado pedido");
            let q = corpo.quadros();
            corpo.limpar();
            let s = q
                .iter()
                .filter(|x| (x.cmd_set, x.cmd_id) == id::SEGUIR)
                .count();
            seguir += s;
            placas.extend(
                q.iter()
                    .filter(|x| (x.cmd_set, x.cmd_id) == id::ARMADURA_VOZ)
                    .cloned(),
            );
            if e.limitado_por.contains(&"picked_up") {
                let rodas: Vec<_> = q
                    .iter()
                    .filter(|x| (x.cmd_set, x.cmd_id) == id::RODAS)
                    .collect();
                assert!(
                    s == 0 && rodas.len() == 1 && rodas[0].dados[..8].iter().all(|b| *b == 0),
                    "levantado aos {} ms: {q:?}",
                    ms
                );
                no_ar.push(*ms);
            }
            !corpo.a_ler.lock().unwrap().is_empty()
        } {}
    }
    assert_eq!(seguir, 1, "sem pilotar, um seguir a zero, ao armar");
    // A sensibilidade vai uma vez, com o primeiro pacote, da central (900) para as seis placas, no nível do arquivo.
    assert_eq!(
        placas
            .iter()
            .map(|q| (q.origem, q.destino))
            .collect::<Vec<_>>(),
        [0x38, 0x58, 0x78, 0x98, 0xb8, 0xd8]
            .map(|d| (0x09, d))
            .to_vec(),
        "{placas:?}"
    );
    assert!(
        placas
            .iter()
            .all(|q| q.dados[13..] == [80, 0, 90, 0, 100, 0]),
        "nível 10: picos 80, 90 e 100"
    );
    assert!(
        !no_ar.is_empty(),
        "nenhum ciclo parou as rodas por levantado"
    );
    let (de, ate) = (
        no_ar[0] as f64 / 1000.0,
        *no_ar.last().unwrap() as f64 / 1000.0,
    );
    println!(
        "rodas paradas por levantado de {de:.1} a {ate:.1} s, em {} ciclos",
        no_ar.len()
    );
    // No tempo desta gravação, as rodas giraram no ar sem comando duas vezes: de 124,6 a 130,3 s e de 205,5 a 216,5 s
    // (`scripts/rodas.py`, no laboratório), com o giroscópio abaixo de 2,3 rad/s. A regra da velocidade só pegou a segunda, 3,5 s
    // depois; a do giro pelas rodas sem o corpo girar pega a primeira em 1,5 s. Ninguém desarmou: as rodas seguem paradas até
    // o fim da gravação, mesmo de volta ao chão.
    let fim = leituras.last().map_or(0, |(ms, _)| *ms) as f64 / 1000.0;
    assert!(
        (124.6..127.0).contains(&de) && ate >= fim - 0.1,
        "{de} a {ate}, gravação até {fim}"
    );
}
