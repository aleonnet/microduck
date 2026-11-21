//! `rms1-mediad`: lê `[midia]` do `rms1d.toml`, sobe as fontes escolhidas e serve o socket local.
//! Ligar não toca nada: som só sai por `robot.sound`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use clap::Parser;
use rms1_mediad::alto_falante::{Aplay, Nula, Saida, Simulador, Tocador};
use rms1_mediad::camera;
use rms1_mediad::corpo::{self, Relogio};
use rms1_mediad::microfone::{self, BLOCO, Ouvinte, Voz};
use rms1_mediad::mixer;
use rms1_mediad::percepcao;
use rms1_mediad::quadros::Quadros;
use rms1_mediad::servidor::{self, Servico};
use rms1_mediad::trava::Trava;
use rms1d_params::{Fisica, FonteCamera, FonteMicrofone, Midia, Params, SaidaDeSom};

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
/// De quanto em quanto a câmera do simulador tenta de novo, com o serviço local fora do ar: o do pato, 1 s.
const RECONECTAR_AO_SIMULADOR: Duration = Duration::from_secs(1);
/// De quanto em quanto o microfone do robô sobe de novo o `arecord` que caiu.
const RELIGAR_O_MICROFONE: Duration = Duration::from_secs(2);

/// A câmera do robô, aberta pela sequência da DJI. Só existe no robô: em outra máquina, o serviço diz e não sobe.
#[cfg(target_os = "linux")]
fn subir_camera_do_robo(
    dispositivo: &str,
    quadros: Quadros,
    servico: Arc<Servico>,
    parar: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    use rms1_mediad::camera_s1::{self, linux::Video0};
    let dev =
        Video0::abrir(dispositivo).map_err(|e| anyhow::anyhow!("{dispositivo} não abriu: {e}"))?;
    *servico.camera.lock().expect("estado da câmera") =
        Some("a câmera do robô está aquecendo".to_owned());
    std::thread::Builder::new()
        .name("camera".into())
        .spawn(move || {
            let s = servico.clone();
            let dizer = move |motivo| *s.camera.lock().expect("estado da câmera") = motivo;
            let contagem = servico.contagem_da_camera.clone();
            camera_s1::rodar_contando(
                quadros,
                dev,
                camera_s1::AQUECIMENTO,
                dizer,
                &contagem,
                parar,
            );
        })?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn subir_camera_do_robo(
    dispositivo: &str,
    _quadros: Quadros,
    _servico: Arc<Servico>,
    _parar: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "a câmera do robô ({dispositivo}) só existe no robô; aqui use a fonte de teste ou a do simulador"
    )
}

/// Sobe o microfone escolhido. Cada trecho de voz que ele guarda vai a `trechos`: é o que a percepção manda à estação.
fn subir_microfone(
    m: &Midia,
    trava: Trava,
    servico: Arc<Servico>,
    parar: Arc<AtomicBool>,
    trechos: std::sync::mpsc::Sender<Vec<i16>>,
) -> anyhow::Result<()> {
    let voz = Voz {
        folga: Duration::from_millis(m.voz.folga_ms),
        silencio: Duration::from_millis(m.voz.silencio_ms),
        teto: Duration::from_millis(m.voz.teto_ms),
        minimo: Duration::from_millis(m.voz.minimo_ms),
        entra: m.voz.entra,
        sai: m.voz.sai,
    };
    let mut ouvinte = Ouvinte::com_voz(trava, voz);
    if m.microfone.fonte == FonteMicrofone::Simulador {
        let (s, d) = (servico.clone(), servico.clone());
        let saidas = microfone::Saidas {
            // Sem a percepção de pé, o trecho não tem para quem ir.
            trecho: Box::new(move |t| drop(trechos.send(t))),
            // Sem assinante, o evento não tem para quem ir.
            ouvido: Box::new(move |o| drop(s.ouvidos.send(o))),
            nivel: servico.nivel_do_microfone.clone(),
            dizer: Box::new(move |motivo| {
                if let Some(m) = &motivo {
                    tracing::warn!(motivo = m, "o microfone do simulador");
                }
                *d.microfone.lock().expect("estado do microfone") = motivo;
            }),
        };
        let endereco = m.microfone.endereco.clone();
        std::thread::Builder::new()
            .name("microfone".into())
            .spawn(move || {
                microfone::rodar_simulador(&endereco, ouvinte, saidas, RELIGAR_O_MICROFONE, parar)
            })?;
        return Ok(());
    }
    if m.microfone.fonte == FonteMicrofone::Arecord {
        // O mesmo pedido do trabalhador do pato: PCM cru, 16 bits, 16 kHz, um canal.
        let args: Vec<String> = [
            "-q",
            "-D",
            &m.microfone.dispositivo,
            "-f",
            "S16_LE",
            "-r",
            "16000",
            "-c",
            "1",
            "-t",
            "raw",
        ]
        .iter()
        .map(|a| (*a).to_owned())
        .collect();
        let (s, d) = (servico.clone(), servico.clone());
        let saidas = microfone::Saidas {
            // Sem a percepção de pé, o trecho não tem para quem ir.
            trecho: Box::new(move |t| drop(trechos.send(t))),
            // Sem assinante, o evento não tem para quem ir.
            ouvido: Box::new(move |o| drop(s.ouvidos.send(o))),
            nivel: servico.nivel_do_microfone.clone(),
            dizer: Box::new(move |motivo| {
                if let Some(m) = &motivo {
                    tracing::warn!(motivo = m, "o microfone do robô");
                }
                *d.microfone.lock().expect("estado do microfone") = motivo;
            }),
        };
        std::thread::Builder::new()
            .name("microfone".into())
            .spawn(move || {
                microfone::rodar_comando(
                    "arecord",
                    &args,
                    ouvinte,
                    saidas,
                    RELIGAR_O_MICROFONE,
                    parar,
                )
            })?;
        return Ok(());
    }
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
            let bloco = Duration::from_secs_f64(BLOCO as f64 / HZ_DO_MICROFONE as f64);
            // Em laço, no ritmo real: o vigia mede a sala no tempo, e a trava também. Cada bloco tem a sua hora, contada do
            // começo: dormir um bloco por vez somava o tempo de cada volta, e o som atrasava 3% (10 s em 5 minutos, medido
            // pela bancada de voz do laboratório em 2026-11-18).
            let (comeco, mut blocos) = (Instant::now(), 0u32);
            while !parar.load(Ordering::Relaxed) {
                for pedaco in amostras.chunks(BLOCO) {
                    if parar.load(Ordering::Relaxed) {
                        return;
                    }
                    blocos += 1;
                    std::thread::sleep((comeco + bloco * blocos).saturating_duration_since(Instant::now()));
                    for o in ouvinte.empurrar(pedaco, Instant::now()) {
                        // Sem assinante, o evento não tem para quem ir.
                        let _ = servico.ouvidos.send(o);
                    }
                    if let Some(trecho) = ouvinte.trecho() {
                        let _ = trechos.send(trecho);
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
            SaidaDeSom::Simulador => Box::new(Simulador { endereco: midia.alto_falante.endereco.clone() }),
        };
        // Sem saída de som, o S1 não diz à estação que recebe a fala.
        let tem_alto_falante = midia.alto_falante.saida != SaidaDeSom::Nula;
        let tocador = Tocador::novo(PathBuf::from(&midia.alto_falante.banco), saida, trava.clone());
        let quadros = Quadros::default();
        let servico = Servico::novo(quadros.clone(), tocador);
        let parar = Arc::new(AtomicBool::new(false));

        // O chip de áudio antes do microfone e do alto-falante: um valor que não pega é dito, e o serviço não sobe.
        if midia.mixer {
            let mut chip: Box<dyn mixer::Controles> = Box::new(mixer::Amixer);
            if let Err(e) = mixer::configurar(chip.as_mut()) {
                eprintln!("rms1-mediad: mixer: {e}");
                return ExitCode::FAILURE;
            }
            *servico.mixer.lock().expect("trava do mixer") = Some(chip);
            if let Err(e) = servico.definir_volume(mixer::VOLUME_DE_PARTIDA) {
                eprintln!("rms1-mediad: mixer: {e}");
                return ExitCode::FAILURE;
            }
            tracing::info!("o mixer está como a partida da DJI deixa, conferido controle a controle");
        }

        match midia.camera.fonte {
            FonteCamera::Teste => {
                let (q, p) = (quadros.clone(), parar.clone());
                std::thread::spawn(move || camera::rodar_teste(q, PERIODO_DE_TESTE, p));
            }
            FonteCamera::S1 => {
                if let Err(e) = subir_camera_do_robo(&midia.camera.dispositivo, quadros.clone(), servico.clone(), parar.clone()) {
                    eprintln!("rms1-mediad: câmera: {e}");
                    return ExitCode::FAILURE;
                }
            }
            FonteCamera::Simulador => {
                let (q, p, s) = (quadros.clone(), parar.clone(), servico.clone());
                let endereco = midia.camera.endereco.clone();
                std::thread::spawn(move || {
                    let dizer = move |motivo| *s.camera.lock().expect("estado da câmera") = motivo;
                    camera::rodar_simulador(q, &endereco, RECONECTAR_AO_SIMULADOR, dizer, p)
                });
            }
        }
        // A conversa com a estação só existe com um microfone: é dele que sai a voz.
        let (dizer_trecho, trechos) = std::sync::mpsc::channel();
        let mut conversa = (midia.microfone.fonte != FonteMicrofone::Nenhuma).then(|| {
            let s = servico.clone();
            percepcao::Conversa {
                trechos,
                // Sem assinante, o resultado não tem para quem ir.
                publicar: Arc::new(move |linha: String| drop(s.resultados.send(linha.into()))),
            }
        });
        if let Err(e) = subir_microfone(&midia, trava, servico.clone(), parar.clone(), dizer_trecho) {
            eprintln!("rms1-mediad: microfone: {e}");
            return ExitCode::FAILURE;
        }
        // O caminho de quem cumpre os pedidos da estação: o pedido vai a quem assina `requests` no socket, e o desfecho
        // volta por aqui. Sem ninguém assinando, a percepção recusa o pedido na hora.
        let (devolver, desfechos) = std::sync::mpsc::channel();
        let _ = servico.desfechos.set(devolver.clone());
        let mut agente = Some({
            let s = servico.clone();
            percepcao::Agente {
                pedir: Arc::new(move |linha: String| s.pedidos.send(linha.into()).is_ok()),
                desfechos,
                devolver,
            }
        });
        // A camada de mundo: com a câmera do robô ou a do simulador, a grade de profundidade da estação vira o chão em volta
        // do robô. A lente do robô só existe na tabela depois de medida: sem ela, o mundo só diz que ela falta. A câmera de
        // teste não olha para lugar nenhum, e fica sem mundo.
        let fisica = Fisica::da_tabela();
        let lente = match midia.camera.fonte {
            FonteCamera::S1 => Some(fisica.camera),
            FonteCamera::Simulador => Some(Some(fisica.camera_simulada)),
            FonteCamera::Teste => None,
        };
        let mut mundo = None;
        if let Some(lente) = lente {
            let olho = lente.map(|lente| rms1_mundo::Olho { lente, no_corpo: fisica.camera_no_corpo });
            let corpo_do_robo = rms1_mundo::Corpo { largura_m: fisica.largura_m, frente_m: fisica.frente_m };
            let partilhado = match rms1_mundo::Mundo::novo(olho, corpo_do_robo, midia.mundo.clone()) {
                Ok(m) => Arc::new(std::sync::Mutex::new(m)),
                Err(e) => {
                    eprintln!("rms1-mediad: [midia.mundo] gravar = {:?}: {e}", midia.mundo.gravar);
                    return ExitCode::FAILURE;
                }
            };
            let relogio = Relogio::default();
            // A torre e a pose vêm do `rms1d`, por um cliente que só lê.
            let (m, p, robo) = (partilhado.clone(), parar.clone(), PathBuf::from(&midia.robo));
            let subiu = std::thread::Builder::new().name("corpo".into()).spawn(move || {
                corpo::rodar(&robo, relogio, move |a| m.lock().expect("o mundo").amostra(a), p);
            });
            if let Err(e) = subiu {
                eprintln!("rms1-mediad: o cliente do rms1d não subiu: {e}");
                return ExitCode::FAILURE;
            }
            let (s, g, q) = (servico.clone(), servico.clone(), servico.clone());
            // No simulador, a profundidade pode ser a de verdade da cena, que a página entrega pelo socket.
            let da_cena = midia.mundo.profundidade == rms1d_params::FonteProfundidade::Simulador;
            let do_mundo = percepcao::Mundo {
                mundo: partilhado,
                relogio,
                // Sem assinante, a linha não tem para quem ir.
                publicar: Arc::new(move |linha: String| drop(s.mundo.send(linha.into()))),
                publicar_grade: Arc::new(move |linha: String| drop(g.grade_do_mundo.send(linha.into()))),
                quer: Arc::new(move || [q.mundo.receiver_count() > 0, q.grade_do_mundo.receiver_count() > 0]),
                da_estacao: !da_cena,
            };
            if da_cena {
                let _ = servico.grade_de_fora.set(percepcao::grade_de_fora(&do_mundo, servico.percepcao.clone()));
            }
            mundo = Some(do_mundo);
            tracing::info!(robo = %midia.robo, lente = if olho.is_some() { "na tabela" } else { "por medir" }, profundidade = if da_cena { "da cena do simulador" } else { "da estação" }, "a camada de mundo está de pé");
        }
        // O codificador de hardware do robô, se o arquivo o dá: a percepção e a imagem da tela usam o mesmo, um pedido por vez.
        if !midia.venc.is_empty() {
            let _ = servico.venc.set(Arc::new(std::sync::Mutex::new(rms1_mediad::venc::Venc::novo(&midia.venc))));
            tracing::info!(socket = %midia.venc, "o quadro pode sair do codificador de hardware");
        }
        // A percepção pela estação: sem estação na rede, só ouve a porta do anúncio. Se nem isso consegue, o serviço
        // segue de pé sem ela, e `s1.media.status` diz por quê.
        let (q, s, p) = (quadros.clone(), servico.clone(), parar.clone());
        std::thread::Builder::new()
            .name("percepcao".into())
            .spawn(move || {
                let cfg = percepcao::Config {
                    porta_anuncio: midia.percepcao.porta_anuncio,
                    ritmo_max: f64::from(midia.percepcao.ritmo_max),
                    anuncio_vale: rms1_mediad::estacao::ANUNCIO_VALE,
                    venc: s.venc.get().cloned(),
                    conversa: conversa.take(),
                    // A fala da estação vai ao tocador, que é quem tem a saída de som e avisa a trava.
                    fala: tem_alto_falante.then(|| {
                        let s = s.clone();
                        Arc::new(move |som: percepcao::Som| {
                            let mut tocador = s.tocador.lock().expect("trava do tocador");
                            match som {
                                percepcao::Som::Pedaco { hz, pcm } => tocador.falar(hz, &pcm, Instant::now()),
                                percepcao::Som::Fim { interrompida } => tocador.fim_da_fala(interrompida, Instant::now()),
                            }
                        }) as Arc<dyn Fn(percepcao::Som) + Send + Sync>
                    }),
                    agente: agente.take(),
                    mundo: mundo.take(),
                };
                let (estado, deteccoes) = (s.percepcao.clone(), s.deteccoes.clone());
                // Sem assinante, a detecção não tem para quem ir.
                let publicar = move |linha: String| drop(deteccoes.send(linha.into()));
                if let Err(e) = percepcao::rodar(cfg, q, publicar, estado, p) {
                    tracing::error!(erro = %e, porta = midia.percepcao.porta_anuncio, "a percepção não subiu");
                    s.percepcao.lock().expect("estado da percepção").idle_reason =
                        Some(format!("a porta do anúncio não abriu: {e}"));
                }
            })
            .expect("thread da percepção");
        let (s, p) = (servico.clone(), parar.clone());
        std::thread::spawn(move || {
            while !p.load(Ordering::Relaxed) {
                s.tocador.lock().expect("trava do tocador").tique(Instant::now());
                std::thread::sleep(TIQUE);
            }
        });

        // A porta do vídeo, se o arquivo pede: sem ela o serviço segue de pé, e a tela fica sem a imagem direta.
        if midia.porta_video != 0 {
            match tokio::net::TcpListener::bind(("0.0.0.0", midia.porta_video)).await {
                Ok(porta) => {
                    tokio::spawn(servidor::servir_tcp(porta, servico.clone()));
                }
                Err(e) => tracing::warn!(erro = %e, porta = midia.porta_video, "a porta do vídeo não abriu"),
            }
        }

        tracing::warn!(socket = %socket.display(), camera = ?midia.camera.fonte, microfone = ?midia.microfone.fonte, som = ?midia.alto_falante.saida, porta_video = midia.porta_video, "rms1-mediad no ar");
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
