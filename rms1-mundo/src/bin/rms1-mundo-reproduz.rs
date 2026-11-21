//! Passa uma sessão gravada (`[midia.mundo] gravar`) pelo mesmo caminho do robô, sem o robô, e escreve o que `s1.world` diria
//! depois de cada grade.
//!
//!   rms1-mundo-reproduz <gravação> [--params <rms1d.toml>] [--camera <câmera.json>] [--esperado <metros>]
//!                       [--janela <de> <até> <metros>]... [--exame] [--calado]
//!
//! - `--params`: os ajustes de `[midia.mundo]` vêm deste arquivo; sem ele, valem os padrões.
//! - `--camera`: troca a câmera da gravação pela deste arquivo (a lente e a posição dela, em JSON, como a gravação as guarda):
//!   é como a calibração experimenta uma lente, ou um atraso, sem voltar ao robô.
//! - `--esperado`: confere a folga à frente (a mediana de `obstacle.ahead_m`, da frente do chassi ao ocupado mais perto)
//!   contra esta distância, pela regra da frente: erro de até 10% ou 5 cm, o que for maior.
//! - `--janela`: a mesma conferência só nos mundos entre as horas `de` e `até` da gravação, em segundos, contra a distância
//!   dada; pode vir várias vezes (a caixa em várias marcas, numa sessão só).
//! - `--exame`: cada linha leva também `exam` (a escala que levou a grade aos metros, de onde veio, e o exame da faixa de
//!   baixo do quadro: quantos pontos, as duas contas da escala, o desvio entre elas e por que a do quadro não valeu) e `fresh`
//!   (o meio de cada célula que este quadro marcou como ocupada, na odometria, em metros). É o que a calibração mede.
//! - `--calado`: só o resumo, sem uma linha por grade.
//!
//! Com `--esperado` ou `--janela`, a última linha é `REPRODUCAO CONFERE` se todas as conferências ficam dentro da regra, e
//! `REPRODUCAO NAO CONFERE`, com a saída 1, se alguma não fica.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rms1_mundo::gravacao::{self, Passo};
use rms1_mundo::{Ajustes, Escala, Olho, Percepcao};
use rms1d_params::Params;

/// A regra da frente para a distância: a maior entre uma fração da distância e estes centímetros.
const REGRA_FRACAO: f64 = 0.1;
const REGRA_CM: f64 = 5.0;

struct Pedido {
    gravacao: PathBuf,
    params: Option<PathBuf>,
    camera: Option<PathBuf>,
    esperado: Option<f64>,
    /// As janelas: de, até, e a distância esperada.
    janelas: Vec<[f64; 3]>,
    exame: bool,
    calado: bool,
}

fn pedido() -> Result<Pedido, String> {
    let uso = "uso: rms1-mundo-reproduz <gravação> [--params <rms1d.toml>] [--camera <câmera.json>] [--esperado <metros>] [--janela <de> <até> <metros>]... [--exame] [--calado]";
    let mut args = std::env::args().skip(1);
    let mut p = Pedido {
        gravacao: PathBuf::new(),
        params: None,
        camera: None,
        esperado: None,
        janelas: Vec::new(),
        exame: false,
        calado: false,
    };
    while let Some(a) = args.next() {
        let mut valor = |nome: &str| args.next().ok_or(format!("{nome} pede um valor\n{uso}"));
        let numero = |nome: &str, v: String| {
            v.parse::<f64>()
                .ok()
                .filter(|m| m.is_finite() && *m >= 0.0)
                .ok_or(format!("{nome} pede um número, e veio {v}"))
        };
        match a.as_str() {
            "--params" => p.params = Some(valor("--params")?.into()),
            "--camera" => p.camera = Some(valor("--camera")?.into()),
            "--esperado" => p.esperado = Some(numero("--esperado", valor("--esperado")?)?),
            "--janela" => {
                let mut j = [0.0; 3];
                for campo in &mut j {
                    *campo = numero("--janela", valor("--janela")?)?;
                }
                if j[1] <= j[0] {
                    return Err(format!(
                        "--janela pede o fim depois do começo, e veio de {} a {}",
                        j[0], j[1]
                    ));
                }
                p.janelas.push(j);
            }
            "--exame" => p.exame = true,
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

/// O exame de um passo, como a linha o leva.
fn exame_de(passo: &Passo) -> serde_json::Value {
    let Some(tratada) = &passo.tratada else {
        return serde_json::Value::Null;
    };
    let origem = tratada.escala.map(|e| match e {
        Escala::Medida => "measured",
        Escala::Lembrada => "remembered",
    });
    let do_quadro = tratada.exame.map(|e| {
        serde_json::json!({
            "total": e.total,
            "points": e.pontos,
            "kept": e.ficaram,
            "ratio": e.pela_razao,
            "line": e.pela_reta,
            "deviation": e.desvio,
            "dispersion": e.dispersao,
            "refused": e.recusa.map(|r| r.nome()),
        })
    });
    serde_json::json!({"scale": tratada.valor, "origin": origem, "frame": do_quadro})
}

/// A folga à frente dos mundos com medida e com ocupado no corredor, entre as horas dadas.
fn a_frente(passos: &[Passo], de: f64, ate: f64) -> Vec<f64> {
    passos
        .iter()
        .filter(|p| p.t >= de && p.t <= ate && p.agora.percepcao == Percepcao::Ok)
        .filter_map(|p| p.agora.obstaculo)
        .map(|o| o.ahead_m)
        .filter(|m| m.is_finite())
        .collect()
}

/// Confere a mediana contra o esperado, pela regra, e diz.
fn conferir(rotulo: &str, valores: &mut [f64], esperado: f64) -> bool {
    let folga = (esperado * REGRA_FRACAO).max(REGRA_CM / 100.0);
    if valores.is_empty() {
        println!("{rotulo}: esperado {esperado:.3} m, e nenhum mundo com ocupado no corredor");
        return false;
    }
    let m = mediana(valores);
    let dentro = (m - esperado).abs() <= folga;
    println!(
        "{rotulo}: mediana {m:.3} m em {} mundos; esperado {esperado:.3} m, com folga de {folga:.3} m: o erro é de {:+.3} m, {}",
        valores.len(),
        m - esperado,
        if dentro {
            "dentro da regra"
        } else {
            "FORA da regra"
        }
    );
    dentro
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
    let passos = gravacao::reproduzir_em_passos(&sessao, camera, &ajustes)?;
    let mut estados: Vec<(&'static str, usize)> = Vec::new();
    for passo in &passos {
        if !p.calado {
            let mut linha = serde_json::json!({"t": passo.t, "world": passo.agora.linha()});
            if p.exame {
                linha["exam"] = exame_de(passo);
                linha["fresh"] = serde_json::json!(passo.ocupadas);
            }
            println!("{linha}");
        }
        let nome = passo.agora.percepcao.nome();
        match estados.iter_mut().find(|(n, _)| *n == nome) {
            Some((_, quantos)) => *quantos += 1,
            None => estados.push((nome, 1)),
        }
    }
    println!(
        "registros: {} | mundos: {}",
        sessao.registros.len(),
        passos.len()
    );
    println!(
        "estados: {}",
        estados
            .iter()
            .map(|(n, q)| format!("{n} {q}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut todos = a_frente(&passos, f64::NEG_INFINITY, f64::INFINITY);
    if todos.is_empty() {
        println!("à frente: nenhum mundo com ocupado no corredor");
    } else {
        let m = mediana(&mut todos);
        let mut desvios: Vec<f64> = todos.iter().map(|v| (v - m).abs()).collect();
        println!(
            "à frente: mediana {m:.3} m, desvio mediano {:.3} m, de {:.3} a {:.3} m, em {} mundos",
            mediana(&mut desvios),
            todos[0],
            todos[todos.len() - 1],
            todos.len()
        );
    }
    if p.esperado.is_none() && p.janelas.is_empty() {
        return Ok(true);
    }
    let mut confere = true;
    if let Some(esperado) = p.esperado {
        confere &= conferir("a sessão inteira", &mut todos, esperado);
    }
    for [de, ate, esperado] in &p.janelas {
        let mut valores = a_frente(&passos, *de, *ate);
        confere &= conferir(&format!("de {de:.2} a {ate:.2} s"), &mut valores, *esperado);
    }
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
