//! `rms1-agente`: cumpre no corpo do S1 o pedido que a estação Hailo validou. Lê `rms1d.toml` para saber onde está o
//! `rms1-mediad` e conferir a tabela das habilidades com o corpo. Ligar não move nada, e o agente não arma o robô.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use duck_ipc_proto as proto;
use rms1_agente::ligacoes::{self, Config};
use rms1_agente::tabela::Tabela;
use rms1d_params::Params;

#[derive(Parser, Debug)]
#[command(
    about = "O agente do RoboMaster S1: cumpre no corpo o pedido validado pela estação",
    version
)]
struct Args {
    /// O `rms1d.toml`. Ausente, usa o caminho padrão se existir, ou os padrões. Se dado, tem de existir.
    #[arg(long)]
    config: Option<PathBuf>,
    /// O socket do `rms1d`.
    #[arg(long, default_value = proto::socket::ROBOT)]
    robo: PathBuf,
    /// O socket do `rms1-mediad`, no lugar de `[midia] socket`.
    #[arg(long)]
    midia: Option<PathBuf>,
    /// A tabela das habilidades, no lugar da que vem com o agente.
    #[arg(long)]
    habilidades: Option<PathBuf>,
    /// Lê e confere a tabela com o corpo, diz as habilidades e sai, sem ligar a nada.
    #[arg(long)]
    conferir: bool,
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    let args = Args::parse();
    let explicito = args.config.is_some();
    let caminho = args
        .config
        .clone()
        .unwrap_or_else(|| PathBuf::from(rms1d_params::CAMINHO_PADRAO));
    let params = match Params::carregar(&caminho, explicito) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("rms1-agente: {e}");
            return ExitCode::FAILURE;
        }
    };
    let tabela = match &args.habilidades {
        None => Tabela::padrao(),
        Some(arquivo) => std::fs::read_to_string(arquivo)
            .map_err(|e| format!("{}: {e}", arquivo.display()))
            .and_then(|t| Tabela::ler(&t).map_err(|e| format!("{}: {e}", arquivo.display()))),
    };
    let tabela = tabela.and_then(|t| {
        t.conferir_com_o_corpo(
            params.seguranca.prazo_ms,
            params.seguranca.velocidade_max_m_s,
        )
        .map(|()| t)
    });
    let tabela = match tabela {
        Ok(t) => t,
        Err(e) => {
            eprintln!("rms1-agente: habilidades: {e}");
            return ExitCode::FAILURE;
        }
    };
    if args.conferir {
        let nomes: Vec<&str> = tabela.habilidades.iter().map(|h| h.ferramenta()).collect();
        println!("{}", nomes.join(" "));
        return ExitCode::SUCCESS;
    }
    let Some(midia) = args
        .midia
        .clone()
        .or_else(|| params.midia.as_ref().map(|m| PathBuf::from(&m.socket)))
    else {
        eprintln!(
            "rms1-agente: sem [midia] em rms1d.toml e sem --midia: não há de onde receber o pedido"
        );
        return ExitCode::FAILURE;
    };
    let executor = match tokio::runtime::Runtime::new() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("rms1-agente: sem executor: {e}");
            return ExitCode::FAILURE;
        }
    };
    executor.block_on(async move {
        tracing::warn!(robo = %args.robo.display(), midia = %midia.display(), "rms1-agente no ar");
        tokio::select! {
            () = ligacoes::rodar(Config { robo: args.robo.clone(), midia, tabela }) => {}
            _ = tokio::signal::ctrl_c() => {}
            () = termino() => {}
        }
        // Sem o agente renovando o pedido, o `rms1d` zera o corpo sozinho, no prazo dele.
        ExitCode::SUCCESS
    })
}

async fn termino() {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut s) => {
            s.recv().await;
        }
        Err(_) => std::future::pending().await,
    }
}
