//! `rms1d`: o serviço do corpo do S1. Lê `rms1d.toml`, é dono do corpo e serve o socket do contrato.
//! Ligar não move nada: só `robot.enable` ou `robot.init` arma.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use duck_ipc_proto as proto;
use rms1_control::corpo::{Corpo, CorpoFalso, CorpoRede, CorpoSerial};
use rms1_control::gravacao::Gravando;
use rms1_control::{Estado, Intencoes, Laco, Saude};
use rms1d::respostas::Contexto;
use rms1d_params::{Params, TipoCorpo};
use tokio::sync::broadcast;

#[derive(Parser, Debug)]
#[command(about = "O serviço do corpo do RoboMaster S1", version)]
struct Args {
    /// Arquivo de parâmetros. Ausente, usa o caminho padrão se existir, ou os padrões. Se dado, tem de existir.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Onde servir o contrato.
    #[arg(long, default_value = proto::socket::ROBOT)]
    socket: PathBuf,
    /// Valida o arquivo, imprime os processos da DJI a parar e sai, sem tocar em nada. É a lista que
    /// `scripts/s1/rms1d-bench.sh` usa, para a recusa vir antes de parar qualquer processo.
    #[arg(long)]
    processos_a_parar: bool,
}

/// Quadros de estado guardados para quem ficou para trás. Cliente mais lento que isso recebe um buraco.
const ESTADOS_NA_FILA: usize = 64;
/// Quanto esperar o laço parar o robô. Um corpo travado não pode prender o processo para sempre.
const ESPERA_ENCERRAR: Duration = Duration::from_secs(2);

fn montar_corpo(p: &Params) -> std::io::Result<Box<dyn Corpo>> {
    let corpo: Box<dyn Corpo> = match p.corpo.tipo {
        TipoCorpo::Serial => Box::new(CorpoSerial::abrir(&p.corpo.porta, p.corpo.baud)?),
        TipoCorpo::Rede => Box::new(CorpoRede::new(p.corpo.endereco.clone())),
        TipoCorpo::Falso => Box::new(CorpoFalso::default()),
    };
    // `gravar_recebido` sem `gravar` é recusado em `Params::validar`.
    Ok(match (&p.corpo.gravar, &p.corpo.gravar_recebido) {
        (Some(enviado), Some(recebido)) => {
            Box::new(Gravando::new(corpo, enviado)?.com_recebido(recebido)?)
        }
        (Some(enviado), None) => Box::new(Gravando::new(corpo, enviado)?),
        (None, _) => corpo,
    })
}

/// O laço, numa thread e num executor só dele: nada do socket fica na frente dele.
fn rodar_laco(
    corpo: Box<dyn Corpo>,
    p: Params,
    intencoes: Arc<Intencoes>,
    saude: Arc<Saude>,
    estados: broadcast::Sender<Arc<Estado>>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    let periodo = Duration::from_secs_f64(1.0 / f64::from(p.laco.hz));
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?;
    std::thread::Builder::new()
        .name("laco".to_owned())
        .spawn(move || {
            executor.block_on(async move {
                let mut laco = Laco::new(corpo, &p, intencoes.clone(), saude);
                let mut ritmo = tokio::time::interval(periodo);
                // `Burst` empilharia comandos atrasados; `Delay` faria o período escorregar.
                ritmo.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    ritmo.tick().await;
                    if intencoes.encerrando() {
                        laco.encerrar();
                        return;
                    }
                    let inicio = Instant::now();
                    let quer = estados.receiver_count() > 0;
                    if let Some(e) = laco.ciclo(inicio, quer) {
                        // Enviar num broadcast nunca espera: assinante lento fica para trás sozinho.
                        let _ = estados.send(Arc::new(e));
                    }
                    laco.contar_trabalho(inicio.elapsed());
                }
            });
        })
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
            eprintln!("rms1d: {e}");
            return ExitCode::FAILURE;
        }
    };

    if args.processos_a_parar {
        println!("{}", params.dji.parar.join(" "));
        return ExitCode::SUCCESS;
    }

    let executor = match tokio::runtime::Runtime::new() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("rms1d: sem executor: {e}");
            return ExitCode::FAILURE;
        }
    };
    executor.block_on(async move {
        // A trava antes do corpo: um segundo processo não pode abrir a serial.
        let (_trava, ouvinte) = match rms1d::servidor::abrir(&args.socket).await {
            Ok(a) => a,
            Err(e) => {
                eprintln!("rms1d: {}: {e}", args.socket.display());
                return ExitCode::FAILURE;
            }
        };
        let corpo = match montar_corpo(&params) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("rms1d: corpo {:?}: {e}", params.corpo.tipo);
                return ExitCode::FAILURE;
            }
        };
        let intencoes = Arc::new(Intencoes::default());
        let saude = Arc::new(Saude::default());
        let (estados, _) = broadcast::channel(ESTADOS_NA_FILA);
        let laco = match rodar_laco(
            corpo,
            params.clone(),
            intencoes.clone(),
            saude.clone(),
            estados.clone(),
        ) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("rms1d: sem laço: {e}");
                return ExitCode::FAILURE;
            }
        };
        tracing::warn!(corpo = ?params.corpo.tipo, socket = %args.socket.display(), "rms1d no ar, desarmado");
        let contexto = Arc::new(Contexto {
            params,
            intencoes: intencoes.clone(),
            saude,
        });
        let socket = args.socket.clone();
        tokio::select! {
            () = rms1d::servidor::servir(ouvinte, contexto, estados) => {}
            _ = tokio::signal::ctrl_c() => {}
            () = termino() => {}
        }
        // Sair armado deixaria o último comando valendo no robô. É o laço quem para, porque só ele tem o corpo.
        intencoes.pedir_encerrar();
        let inicio = Instant::now();
        while !laco.is_finished() && inicio.elapsed() < ESPERA_ENCERRAR {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        if !laco.is_finished() {
            tracing::error!("o laço não parou a tempo: o corpo pode ter ficado sem o comando de parar");
        }
        let _ = std::fs::remove_file(socket);
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
