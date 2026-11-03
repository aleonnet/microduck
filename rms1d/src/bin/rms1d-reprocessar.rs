//! `rms1d-reprocessar`: uma gravação do que o corpo mandou, reprocessada sem o robô. Para cada pacote, o que o controlador
//! da DJI disse e o que o `rms1d` calcula em modo sombra, com os parâmetros de `rms1d.toml`.
//!
//! Lê duas formas: o `strace` das leituras de um processo (`strace -f -tt -e trace=read -s 256 -p <pid>`, o método das
//! gravações do laboratório) e o arquivo de `corpo.gravar_recebido`. As gravações são do laboratório, que é privado: este
//! repositório é público, e nada delas entra aqui; a ferramenta as lê por caminho.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};
use rms1_control::estimativa::{Linha, Reprocessada, de_recebido, de_strace};
use rms1_control::gravacao::ler_recebido;
use rms1_control::telemetria::BIT_LEVANTADO;
use rms1d_params::Params;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Formato {
    /// `strace` das leituras.
    Strace,
    /// `corpo.gravar_recebido`: `ms hex` por leitura.
    Recebido,
}

#[derive(Parser, Debug)]
#[command(
    about = "Reprocessa uma gravação do corpo do S1: o controlador ao lado do cálculo do rms1d"
)]
struct Args {
    gravacao: PathBuf,
    #[arg(long, value_enum, default_value = "strace")]
    formato: Formato,
    /// Os parâmetros; sem ele, os padrões.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Uma linha por segundo, com a média do segundo.
    #[arg(long)]
    por_segundo: bool,
    /// Uma linha por pacote.
    #[arg(long)]
    por_pacote: bool,
}

fn ler(a: &Args, p: &Params) -> std::io::Result<Reprocessada> {
    Ok(match a.formato {
        Formato::Strace => de_strace(&std::fs::read(&a.gravacao)?, p),
        Formato::Recebido => de_recebido(&ler_recebido(&a.gravacao)?, p),
    })
}

fn plano(v: [f64; 3]) -> f64 {
    v[0].hypot(v[1])
}

fn imprimir(l: &Linha) {
    let (m, c) = (&l.medidas, &l.calculo);
    println!(
        "{:7.2} | {:5.2} {:5.2} | {:6.1} {:6.1} | {:5} {:5.2} | {:>5} | {:6.2} {:6.2} | {:6.2} {:6.2}",
        l.t,
        plano(m.velocidade_corpo),
        c.velocidade_corpo[0].hypot(c.velocidade_corpo[1]),
        m.rumo,
        c.rumo_pelo_giro,
        c.levantado,
        c.discordancia,
        c.divergencia.map_or("-".to_owned(), |d| format!("{d:.1}")),
        m.posicao[0],
        m.posicao[1],
        c.posicao[0],
        c.posicao[1],
    );
}

/// Os trechos em que o nosso levantado ficou marcado.
fn trechos(linhas: &[Linha]) -> Vec<(f64, f64)> {
    let mut saida = Vec::new();
    let mut desde = None;
    for l in linhas {
        match (desde, l.calculo.levantado) {
            (None, true) => desde = Some(l.t),
            (Some(d), false) => {
                saida.push((d, l.t));
                desde = None;
            }
            _ => {}
        }
    }
    if let (Some(d), Some(u)) = (desde, linhas.last()) {
        saida.push((d, u.t));
    }
    saida
}

fn main() -> ExitCode {
    let a = Args::parse();
    let p = match &a.config {
        Some(c) => match Params::carregar(c, true) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("rms1d-reprocessar: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => Params::default(),
    };
    let Reprocessada { quadros, linhas } = match ler(&a, &p) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("rms1d-reprocessar: {}: {e}", a.gravacao.display());
            return ExitCode::FAILURE;
        }
    };
    if linhas.is_empty() {
        eprintln!(
            "rms1d-reprocessar: nenhum pacote de telemetria na ordem da nossa assinatura em {quadros} quadros"
        );
        return ExitCode::FAILURE;
    }
    let dur = linhas.last().map_or(0.0, |l| l.t);
    println!(
        "quadros {} | pacotes {} | {:.1} s | {:.1} pacotes por segundo",
        quadros,
        linhas.len(),
        dur,
        linhas.len() as f64 / dur.max(1e-9)
    );
    if a.por_pacote || a.por_segundo {
        println!(
            "      t | v DJI  nossa | rumo DJI  giro | levantado disc. | diverg. | pos DJI x y   | pos nossa x y"
        );
    }
    if a.por_pacote {
        linhas.iter().for_each(imprimir);
    } else if a.por_segundo {
        let mut segundo = -1i64;
        for l in &linhas {
            if l.t.floor() as i64 != segundo {
                segundo = l.t.floor() as i64;
                imprimir(l);
            }
        }
    }
    let ult = linhas.last().expect("não vazio");
    println!(
        "levantado pelo controlador (bit {}): {} pacotes",
        BIT_LEVANTADO.trailing_zeros(),
        linhas
            .iter()
            .filter(|l| l.estado & BIT_LEVANTADO != 0)
            .count()
    );
    let t = trechos(&linhas);
    println!(
        "levantado pelo rms1d: {} trechos: {}",
        t.len(),
        t.iter()
            .map(|(a, b)| format!("{a:.1}-{b:.1} s"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "velocidade: a do controlador chega a {:.2} m/s, a nossa a {:.2} m/s",
        linhas
            .iter()
            .map(|l| plano(l.medidas.velocidade_corpo))
            .fold(0.0, f64::max),
        linhas
            .iter()
            .map(|l| l.calculo.velocidade_corpo[0].hypot(l.calculo.velocidade_corpo[1]))
            .fold(0.0, f64::max)
    );
    println!(
        "rumo no fim: controlador {:.1}°, giro integrado {:.1}°",
        ult.medidas.rumo, ult.calculo.rumo_pelo_giro
    );
    let quieto: Vec<f64> = linhas
        .iter()
        .filter_map(|l| l.calculo.divergencia)
        .collect();
    println!(
        "gravidade, quieto: {} pacotes, divergência máxima {:.1}°, avisos {}",
        quieto.len(),
        quieto.iter().copied().fold(0.0, f64::max),
        linhas.iter().filter(|l| l.calculo.divergente).count()
    );
    ExitCode::SUCCESS
}
