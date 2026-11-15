//! `rms1-mediad`: lê `[midia]` do `rms1d.toml`, sobe as fontes escolhidas e serve o socket local.
//! Ligar não toca nada: som só sai por `robot.sound`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use clap::Parser;
use rms1_mediad::alto_falante::{Aplay, Nula, Saida, Tocador};
use rms1_mediad::camera;
use rms1_mediad::microfone::{self, BLOCO, Ouvinte};
use rms1_mediad::quadros::Quadros;
use rms1_mediad::servidor::{self, Servico};
use rms1_mediad::trava::Trava;
use rms1d_params::{FonteCamera, FonteMicrofone, Midia, Params, SaidaDeSom};

#[derive(Parser, Debug)]
#[command(about = "O serviço de mídia do RoboMaster S1", version)]
struct Args {
    /// O `rms1d.toml`. Ausente, usa o caminho padrão se existir, ou os padrões. Se dado, tem de existir.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Onde servir, no lugar de `[midia] socket`.
    #[arg(long)]
    socket: Option<PathBuf>,
}

/// O ritmo da fonte de teste: o da câmera do robô, 30 quadros por segundo. Sem pedido, nada é copiado.
const PERIODO_DE_TESTE: Duration = Duration::from_millis(33);
/// De quanto em quanto o tocador recolhe o som que terminou e confere o `wheee`: o tique do pato, 20 ms.
const TIQUE: Duration = Duration::from_millis(20);
const HZ_DO_MICROFONE: usize = 16_000;

fn subir_microfone(
    m: &Midia,
    trava: Trava,
    servico: Arc<Servico>,
    parar: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let FonteMicrofone::Arquivo = m.microfone.fonte else {
        return Ok(());
    };
    // `validar` já exigiu o arquivo.
    let caminho = m.microfone.arquivo.clone().unwrap_or_default();
    let amostras = microfone::ler_arquivo(&caminho)?;
    anyhow::ensure!(!amostras.is_empty(), "{caminho} não tem som");
    std::thread::Builder::new()
        .name("microfone".into())
        .spawn(move || {
            let mut ouvinte = Ouvinte::novo(trava);
            let bloco = Duration::from_secs_f64(BLOCO as f64 / HZ_DO_MICROFONE as f64);
            // Em laço, no ritmo real: o vigia mede a sala no tempo, e a trava também.
            while !parar.load(Ordering::Relaxed) {
                for pedaco in amostras.chunks(BLOCO) {
                    if parar.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(bloco);
                    for o in ouvinte.empurrar(pedaco, Instant::now()) {
                        // Sem assinante, o evento não tem para quem ir.
                        let _ = servico.ouvidos.send(o);
                    }
                }
            }
        })?;
    Ok(())
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
    let midia = match Params::carregar(&caminho, explicito) {
        Ok(p) => p.midia.unwrap_or_default(),
        Err(e) => {
            eprintln!("rms1-mediad: {e}");
            return ExitCode::FAILURE;
        }
    };
    let socket = args
        .socket
        .clone()
        .unwrap_or_else(|| PathBuf::from(&midia.socket));
    let executor = match tokio::runtime::Runtime::new() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("rms1-mediad: sem executor: {e}");
            return ExitCode::FAILURE;
        }
    };
    executor.block_on(async move {
        // A trava antes de qualquer dispositivo: um segundo serviço não pode abrir a câmera nem o som.
        let (_trava_do_socket, ouvinte) = match servidor::abrir(&socket).await {
            Ok(a) => a,
            Err(e) => {
                eprintln!("rms1-mediad: {}: {e}", socket.display());
                return ExitCode::FAILURE;
            }
        };
        let trava = Trava::nova(Duration::from_millis(midia.trava_ms));
        let saida: Box<dyn Saida> = match midia.alto_falante.saida {
            SaidaDeSom::Aplay => Box::new(Aplay { dispositivo: midia.alto_falante.dispositivo.clone() }),
            SaidaDeSom::Nula => Box::new(Nula),
        };
        let tocador = Tocador::novo(PathBuf::from(&midia.alto_falante.banco), saida, trava.clone());
        let quadros = Quadros::default();
        let servico = Servico::novo(quadros.clone(), tocador);
        let parar = Arc::new(AtomicBool::new(false));

        match midia.camera.fonte {
            FonteCamera::Teste => {
                let (q, p) = (quadros.clone(), parar.clone());
                std::thread::spawn(move || camera::rodar_teste(q, PERIODO_DE_TESTE, p));
            }
        }
        if let Err(e) = subir_microfone(&midia, trava, servico.clone(), parar.clone()) {
            eprintln!("rms1-mediad: microfone: {e}");
            return ExitCode::FAILURE;
        }
        let (s, p) = (servico.clone(), parar.clone());
        std::thread::spawn(move || {
            while !p.load(Ordering::Relaxed) {
                s.tocador.lock().expect("trava do tocador").tique(Instant::now());
                std::thread::sleep(TIQUE);
            }
        });

        tracing::warn!(socket = %socket.display(), camera = ?midia.camera.fonte, microfone = ?midia.microfone.fonte, som = ?midia.alto_falante.saida, "rms1-mediad no ar");
        tokio::select! {
            () = servidor::servir(ouvinte, servico.clone()) => {}
            _ = tokio::signal::ctrl_c() => {}
            () = termino() => {}
        }
        // Sair com um som tocando deixaria o `aplay` órfão segurando a saída.
        parar.store(true, Ordering::Relaxed);
        servico.tocador.lock().expect("trava do tocador").parar(Instant::now());
        let _ = std::fs::remove_file(&socket);
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
