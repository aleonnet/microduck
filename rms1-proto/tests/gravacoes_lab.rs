//! O decodificador contra as gravações do laboratório. Não rodam no `cargo test` comum: as gravações são do repositório
//! privado, e este é público. `scripts/test-rms1-telemetria.sh` roda estes testes, e reprova se o laboratório não estiver lá.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rms1_proto::barramento::{
    ATITUDE, BATERIA, ESTADO, IMU, Layout, POSICAO, Pacote, RODAS, SERVO, TORRE_BASICO, VELOCIDADE,
    id,
};
use rms1_proto::{Leitor, Quadro};

fn lab() -> PathBuf {
    PathBuf::from(std::env::var("S1_LAB").expect("S1_LAB tem de apontar para o laboratório"))
}

/// Desfaz o escape do `strace`.
fn desfazer(s: &[u8]) -> Vec<u8> {
    let mut o = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] != b'\\' {
            o.push(s[i]);
            i += 1;
            continue;
        }
        let n = s[i + 1];
        let simples = match n {
            b'n' => Some(10),
            b't' => Some(9),
            b'r' => Some(13),
            b'f' => Some(12),
            b'v' => Some(11),
            b'a' => Some(7),
            b'b' => Some(8),
            b'e' => Some(27),
            b'\\' => Some(92),
            b'"' => Some(34),
            _ => None,
        };
        if let Some(v) = simples {
            o.push(v);
            i += 2;
        } else if n == b'x' {
            o.push(u8::from_str_radix(std::str::from_utf8(&s[i + 2..i + 4]).unwrap(), 16).unwrap());
            i += 4;
        } else {
            let mut j = i + 1;
            let mut v = 0u32;
            while j < s.len() && j < i + 4 && (b'0'..=b'7').contains(&s[j]) {
                v = v * 8 + u32::from(s[j] - b'0');
                j += 1;
            }
            o.push(v as u8);
            i = j;
        }
    }
    o
}

/// Os quadros lidos de um `strace`, por descritor, com o verificador conferido.
fn quadros(caminho: &Path) -> Vec<Quadro> {
    let texto = std::fs::read(caminho).unwrap_or_else(|e| panic!("{}: {e}", caminho.display()));
    let mut por_fd: HashMap<Vec<u8>, Leitor> = HashMap::new();
    let mut saida = Vec::new();
    for linha in texto.split(|b| *b == b'\n') {
        let Some(p) = linha.windows(5).position(|w| w == b"read(") else {
            continue;
        };
        let resto = &linha[p + 5..];
        let Some(v) = resto.iter().position(|b| *b == b',') else {
            continue;
        };
        let fd = resto[..v].to_vec();
        let Some(a) = resto.iter().position(|b| *b == b'"') else {
            continue;
        };
        let corpo = &resto[a + 1..];
        let mut f = 0;
        while f < corpo.len() && corpo[f] != b'"' {
            f += if corpo[f] == b'\\' { 2 } else { 1 };
        }
        if f > corpo.len() {
            continue;
        }
        saida.extend(
            por_fd
                .entry(fd)
                .or_default()
                .empurrar(&desfazer(&corpo[..f])),
        );
    }
    saida
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
    assert_eq!(
        (hoje, antiga),
        (2501, 4867),
        "os pacotes medidos no levantamento"
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
    assert_eq!(p.len(), 1341);
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
