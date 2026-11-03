//! O decodificador contra as gravações do laboratório. Não rodam no `cargo test` comum: as gravações são do repositório
//! privado, e este é público. `scripts/test-rms1-telemetria.sh` roda estes testes, e reprova se o laboratório não estiver lá.

use std::path::{Path, PathBuf};

use rms1_proto::Quadro;
use rms1_proto::barramento::{
    ATITUDE, BATERIA, ESTADO, IMU, Layout, POSICAO, Pacote, RODAS, SERVO, TORRE_BASICO, VELOCIDADE,
    id,
};

fn lab() -> PathBuf {
    PathBuf::from(std::env::var("S1_LAB").expect("S1_LAB tem de apontar para o laboratório"))
}

/// Os quadros lidos de um `strace`, por descritor, com o verificador conferido.
fn quadros(caminho: &Path) -> Vec<Quadro> {
    let texto = std::fs::read(caminho).unwrap_or_else(|e| panic!("{}: {e}", caminho.display()));
    rms1_proto::strace::quadros(&texto)
        .into_iter()
        .map(|(_, q)| q)
        .collect()
}

fn pacotes(caminho: &str, l: &Layout) -> Vec<Pacote> {
    quadros(&lab().join(caminho))
        .iter()
        .filter(|q| (q.cmd_set, q.cmd_id) == id::PACOTE && q.dados.len() == 163)
        .map(|q| Pacote::ler(&q.dados, l).expect("163 bytes"))
        .collect()
}

fn nas_faixas(p: &Pacote, onde: &str) {
    let ok = (9000..13500).contains(&p.bateria.milivolts)
        && p.bateria.percentual <= 100
        && (-200..800).contains(&p.bateria.temperatura)
        && p.rodas.rotacao.iter().all(|r| r.abs() <= 1000)
        && (-3.0..3.0).contains(&p.aceleracao[2])
        && p.estado <= 2047
        && (-360.0..360.0).contains(&p.atitude[0])
        && (-180.0..180.0).contains(&p.atitude[1])
        && (-180.0..180.0).contains(&p.atitude[2])
        && p.velocidade_corpo.iter().all(|v| v.abs() < 5.0);
    assert!(ok, "{onde}: fora da faixa: {p:?}");
}

fn correlacao(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len() as f64;
    let (ma, mb) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
    let cov: f64 = a.iter().zip(b).map(|(x, y)| (x - ma) * (y - mb)).sum();
    let (sa, sb): (f64, f64) = (
        a.iter().map(|x| (x - ma).powi(2)).sum(),
        b.iter().map(|y| (y - mb).powi(2)).sum(),
    );
    cov / (sa * sb).sqrt()
}

fn ordem_de_hoje() -> Layout {
    Layout::da_assinatura()
}

fn ordem_antiga() -> Layout {
    Layout::de(&[
        TORRE_BASICO,
        VELOCIDADE,
        RODAS,
        BATERIA,
        POSICAO,
        IMU,
        ESTADO,
        ATITUDE,
        SERVO,
    ])
    .unwrap()
}

const DE_HOJE: [&str; 4] = [
    "out/2026-11-02-rms1d-telemetria/s1.strace",
    "out/2026-11-02-rms1d-telemetria/s2.strace",
    "out/2026-10-30-poc2/telemetry-cleanboot/hdvt_cleanboot_reads.strace",
    "out/2026-10-30-poc2/telemetry-cleanboot/hdvt_app_connected_reads.strace",
];
const ANTIGAS: [&str; 2] = [
    "out/2026-10-30-poc2/capture/hdvt_uav.strace",
    "out/2026-10-30-poc2/capture-20261030-165821/hdvt_uav.strace",
];

#[test]
#[ignore = "precisa do laboratório; rode scripts/test-rms1-telemetria.sh"]
fn as_duas_ordens_gravadas_decodificam_nas_faixas() {
    let (mut hoje, mut antiga) = (0, 0);
    for g in DE_HOJE {
        for p in pacotes(g, &ordem_de_hoje()) {
            nas_faixas(&p, g);
            hoje += 1;
        }
    }
    for g in ANTIGAS {
        for p in pacotes(g, &ordem_antiga()) {
            nas_faixas(&p, g);
            antiga += 1;
        }
    }
    // 2501 e 4867 eram os do leitor que perdia as leituras retomadas; com elas, 2517 e 6396, os mesmos de uma contagem
    // independente, em Python, em 2026-11-03 (`out/2026-11-03-firmware-controlador/conta_retomadas.py`, no laboratório).
    assert_eq!(
        (hoje, antiga),
        (2517, 6396),
        "os pacotes das gravações, com as leituras retomadas"
    );
}

/// A ordem errada tem de reprovar: é o que torna a faixa uma prova.
#[test]
#[ignore = "precisa do laboratório; rode scripts/test-rms1-telemetria.sh"]
fn a_ordem_errada_sai_das_faixas() {
    let p = pacotes(DE_HOJE[1], &ordem_antiga());
    assert!(!p.is_empty());
    assert!(
        !(9000..13500).contains(&p[0].bateria.milivolts),
        "{:?}",
        p[0].bateria
    );
}

/// Rumo em graus e giro em rad/s, com o mesmo sinal, e as quatro rodas com o mesmo sinal ao girar no lugar.
#[test]
#[ignore = "precisa do laboratório; rode scripts/test-rms1-telemetria.sh"]
fn giro_rumo_e_rodas_batem_na_gravacao_que_gira() {
    let p = pacotes(
        "out/2026-10-30-poc2/telemetry-gimbal/hdvt_gimbal_reads.strace",
        &ordem_de_hoje(),
    );
    // 1341 sem as leituras retomadas; 1367 com elas, igual à contagem independente de 2026-11-03.
    assert_eq!(p.len(), 1367);
    let (mut d_rumo, mut giro) = (Vec::new(), Vec::new());
    for par in p.windows(2) {
        let dt = f64::from(par[1].tempo_ms.wrapping_sub(par[0].tempo_ms)) / 1000.0;
        if dt > 0.0 && dt < 0.1 {
            let mut d = f64::from(par[1].atitude[0] - par[0].atitude[0]);
            d = (d + 180.0).rem_euclid(360.0) - 180.0;
            d_rumo.push(d / dt);
            giro.push(f64::from(par[0].giro[2]));
        }
    }
    let c = correlacao(&d_rumo, &giro);
    assert!(c > 0.99, "correlação entre rumo e giro: {c}");
    let razao = d_rumo.iter().zip(&giro).map(|(a, b)| a * b).sum::<f64>()
        / giro.iter().map(|b| b * b).sum::<f64>();
    assert!(
        (50.0..62.0).contains(&razao),
        "graus por segundo por unidade de giro: {razao}"
    );
    let giro_todos: Vec<f64> = p.iter().map(|x| f64::from(x.giro[2])).collect();
    for r in 0..4 {
        let rot: Vec<f64> = p.iter().map(|x| f64::from(x.rodas.rotacao[r])).collect();
        assert!(correlacao(&rot, &giro_todos) < -0.85, "roda {r}");
    }
}

#[test]
#[ignore = "precisa do laboratório; rode scripts/test-rms1-telemetria.sh"]
fn parado_a_aceleracao_em_z_e_menos_um_g() {
    for p in pacotes(DE_HOJE[1], &ordem_de_hoje()) {
        assert!((-1.1..-0.9).contains(&p.aceleracao[2]), "{p:?}");
    }
}

#[test]
#[ignore = "precisa do laboratório; rode scripts/test-rms1-telemetria.sh"]
fn o_relogio_anda_20_ms_por_pacote() {
    let p = pacotes(DE_HOJE[1], &ordem_de_hoje());
    let mut d: Vec<u32> = p
        .windows(2)
        .map(|x| x[1].tempo_ms.wrapping_sub(x[0].tempo_ms))
        .collect();
    d.sort_unstable();
    assert_eq!(d[d.len() / 2], 20);
}
