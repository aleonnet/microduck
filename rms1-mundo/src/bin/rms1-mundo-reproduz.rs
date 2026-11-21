//! Passa uma sessão gravada (`[midia.mundo] gravar`) pelo mesmo caminho do robô, sem o robô, e escreve o que `s1.world` diria
//! depois de cada grade.
//!
//!   rms1-mundo-reproduz <gravação> [--params <rms1d.toml>] [--camera <câmera.json>] [--esperado <metros>] [--calado]
//!
//! - `--params`: os ajustes de `[midia.mundo]` vêm deste arquivo; sem ele, valem os padrões.
//! - `--camera`: troca a câmera da gravação pela deste arquivo (a lente e a posição dela, em JSON, como a gravação as guarda):
//!   é como a calibração experimenta uma lente, ou um atraso, sem voltar ao robô.
//! - `--esperado`: confere a folga à frente (a mediana de `obstacle.ahead_m`, da frente do chassi ao ocupado mais perto)
//!   contra esta distância, pela regra da frente: erro de até 10% ou 5 cm, o que for maior. Dentro da regra, a última linha é
//!   `REPRODUCAO CONFERE`; fora, `REPRODUCAO NAO CONFERE`, e a saída é 1.
//! - `--calado`: só o resumo, sem uma linha por grade.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rms1_mundo::gravacao;
use rms1_mundo::{Ajustes, Olho, Percepcao};
use rms1d_params::Params;

/// A regra da frente para a distância: a maior entre uma fração da distância e estes centímetros.
const REGRA_FRACAO: f64 = 0.1;
const REGRA_CM: f64 = 5.0;

struct Pedido {
    gravacao: PathBuf,
    params: Option<PathBuf>,
    camera: Option<PathBuf>,
    esperado: Option<f64>,
    calado: bool,
}

fn pedido() -> Result<Pedido, String> {
    let uso = "uso: rms1-mundo-reproduz <gravação> [--params <rms1d.toml>] [--camera <câmera.json>] [--esperado <metros>] [--calado]";
    let mut args = std::env::args().skip(1);
    let mut p = Pedido {
        gravacao: PathBuf::new(),
        params: None,
        camera: None,
        esperado: None,
        calado: false,
    };
    while let Some(a) = args.next() {
        let mut valor = |nome: &str| args.next().ok_or(format!("{nome} pede um valor\n{uso}"));
        match a.as_str() {
            "--params" => p.params = Some(valor("--params")?.into()),
            "--camera" => p.camera = Some(valor("--camera")?.into()),
            "--esperado" => {
                let v = valor("--esperado")?;
                p.esperado = Some(
                    v.parse::<f64>()
                        .ok()
                        .filter(|m| m.is_finite() && *m >= 0.0)
                        .ok_or(format!("--esperado pede metros, e veio {v}"))?,
                );
            }
            "--calado" => p.calado = true,
            outro if outro.starts_with("--") => return Err(format!("não conheço {outro}\n{uso}")),
            caminho if p.gravacao.as_os_str().is_empty() => p.gravacao = caminho.into(),
            sobra => return Err(format!("sobrou {sobra}\n{uso}")),
        }
    }
    if p.gravacao.as_os_str().is_empty() {
        return Err(uso.to_owned());
    }
    Ok(p)
}

fn mediana(valores: &mut [f64]) -> f64 {
    valores.sort_by(f64::total_cmp);
    valores[valores.len() / 2]
}

fn rodar() -> Result<bool, String> {
    let p = pedido()?;
    let ajustes: Ajustes = match &p.params {
        Some(caminho) => {
            Params::carregar(caminho, true)
                .map_err(|e| e.to_string())?
                .midia
                .unwrap_or_default()
                .mundo
        }
        None => Ajustes::default(),
    };
    let camera: Option<Olho> = match &p.camera {
        Some(caminho) => Some(
            std::fs::read(caminho)
                .map_err(|e| format!("não consegui ler {}: {e}", caminho.display()))
                .and_then(|b| {
                    serde_json::from_slice(&b)
                        .map_err(|e| format!("a câmera de {} não se lê: {e}", caminho.display()))
                })?,
        ),
        None => None,
    };
    let sessao = gravacao::ler(Path::new(&p.gravacao))?;
    let mundos = gravacao::reproduzir(&sessao, camera, &ajustes)?;
    let mut estados: Vec<(&'static str, usize)> = Vec::new();
    let mut a_frente: Vec<f64> = Vec::new();
    for (t, agora) in &mundos {
        if !p.calado {
            println!("{}", serde_json::json!({"t": t, "world": agora.linha()}));
        }
        let nome = agora.percepcao.nome();
        match estados.iter_mut().find(|(n, _)| *n == nome) {
            Some((_, quantos)) => *quantos += 1,
            None => estados.push((nome, 1)),
        }
        if agora.percepcao == Percepcao::Ok
            && let Some(o) = agora.obstaculo
            && o.ahead_m.is_finite()
        {
            a_frente.push(o.ahead_m);
        }
    }
    println!(
        "registros: {} | mundos: {}",
        sessao.registros.len(),
        mundos.len()
    );
    println!(
        "estados: {}",
        estados
            .iter()
            .map(|(n, q)| format!("{n} {q}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    if a_frente.is_empty() {
        println!("à frente: nenhum mundo com ocupado no corredor");
    } else {
        let m = mediana(&mut a_frente);
        let mut desvios: Vec<f64> = a_frente.iter().map(|v| (v - m).abs()).collect();
        println!(
            "à frente: mediana {m:.3} m, desvio mediano {:.3} m, de {:.3} a {:.3} m, em {} mundos",
            mediana(&mut desvios),
            a_frente[0],
            a_frente[a_frente.len() - 1],
            a_frente.len()
        );
    }
    let Some(esperado) = p.esperado else {
        return Ok(true);
    };
    let folga = (esperado * REGRA_FRACAO).max(REGRA_CM / 100.0);
    let confere = !a_frente.is_empty() && {
        let m = mediana(&mut a_frente);
        println!(
            "esperado {esperado:.3} m, com folga de {folga:.3} m: o erro é de {:.3} m",
            m - esperado
        );
        (m - esperado).abs() <= folga
    };
    println!(
        "{}",
        if confere {
            "REPRODUCAO CONFERE"
        } else {
            "REPRODUCAO NAO CONFERE"
        }
    );
    Ok(confere)
}

fn main() -> ExitCode {
    match rodar() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(motivo) => {
            eprintln!("PAROU: {motivo}");
            ExitCode::from(2)
        }
    }
}
