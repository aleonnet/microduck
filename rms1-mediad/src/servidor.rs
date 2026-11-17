//! O socket local do `rms1-mediad`.
//!
//! - `hello` e `media.frame`, no formato do pato (`mediad/src/frame.rs`): uma linha JSON-RPC com o cabeçalho do contrato e,
//!   em seguida, exatamente `bytes` bytes crus. O quadro nunca passa por JSON, e a conexão fecha depois dele.
//! - `robot.sound`, o do contrato, como requisição ou como notificação: é o que o `rms1d` repassa.
//! - `s1.media.subscribe {"heard": true, "detections": true}`: a conexão passa a receber `s1.audio.heard {"kind"}`, já com
//!   a trava aplicada, e `media.detections`, no formato do pato, a cada entrega do detector da estação. Cada pedido diz as
//!   duas assinaturas: a que não vem é desligada. Com `"voice": true`, a conexão recebe também `s1.station.result`: cada
//!   acontecimento de um pedido que a voz do robô começou na estação (o texto ouvido, a resposta enquanto é gerada, o fim
//!   com o que a função entregou), com a função e o tipo do que ela entrega.
//! - `s1.media.volume {"volume": 0..100}`: o volume do alto-falante, na conta da DJI, pelo mixer do robô.
//! - `s1.media.status`: o que o serviço está fazendo.
//! - `s1.media.preview {"width": 640}`: o quadro reduzido a RGB, para quem só quer olhar (a tela "Sentidos"): o cabeçalho de
//!   `media.frame`, com `format: "RGB"`, e os bytes. A conexão fica aberta, e serve o pedido seguinte. Com
//!   `"format": "jpeg"` (e `"quality"`, de 30 a 90, padrão 70) o quadro vem em JPEG, `format: "JPEG"`: é o que a tela usa,
//!   porque o Wi-Fi do robô não leva 691 KB por quadro a um ritmo de vídeo.
//!
//! O mesmo atendimento serve uma porta TCP, se `[midia] porta_video` a pede ([`servir_tcp`]): por ela só passam `hello`,
//! `s1.media.preview` e `s1.media.status`. É o caminho do vídeo até a tela, direto pela rede: pelo `adb`, um quadro de
//! 691 KB levava mais de 4 s (medido em 2026-11-15, `out/2026-11-14-s1-midia/medida-tela-no-robo.txt`, no laboratório).

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::Result;
use duck_ipc_proto as proto;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, UnixListener, UnixStream};
use tokio::sync::broadcast;

use crate::alto_falante::Tocador;
use crate::microfone::Ouvido;
use crate::mixer::Controles;
use crate::percepcao::Estado;
use crate::quadros::Quadros;

/// O maior pedido que o socket lê.
const PEDIDO_MAXIMO: usize = 4096;
const MODO_SOCKET: u32 = 0o660;
/// Quantos eventos de som ficam guardados para um assinante lento. Quem fica para trás perde os mais velhos.
const OUVIDOS_NA_FILA: usize = 32;
/// A largura do quadro reduzido de `s1.media.preview`, sem pedido de outra, e a menor que se aceita.
const PREVIA_LARGURA: usize = 640;
const PREVIA_MINIMA: usize = 64;
/// A qualidade do JPEG da prévia, sem pedido de outra, e a faixa aceita.
const JPEG_QUALIDADE: u64 = 70;
const JPEG_FAIXA: (u64, u64) = (30, 90);

/// Codifica em JPEG um quadro RGB, com o codificador do `mediad` do pato.
pub fn jpeg_de_rgb(rgb: &[u8], largura: u32, altura: u32, qualidade: u8) -> Option<Vec<u8>> {
    use image::ImageEncoder as _;
    let mut jpeg = Vec::with_capacity(rgb.len() / 8);
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, qualidade)
        .write_image(rgb, largura, altura, image::ExtendedColorType::Rgb8)
        .ok()?;
    Some(jpeg)
}
/// Das detecções só a mais nova interessa: a fila do pato (`mediad/src/detect.rs`), 8.
const DETECCOES_NA_FILA: usize = 8;
/// Os acontecimentos de um pedido vêm em rajada (um por pedaço da resposta), e nenhum pode faltar a quem os junta.
const RESULTADOS_NA_FILA: usize = 512;

/// O que as conexões partilham.
pub struct Servico {
    pub quadros: Quadros,
    pub tocador: Mutex<Tocador>,
    pub ouvidos: broadcast::Sender<Ouvido>,
    /// As notificações `media.detections`, prontas, uma linha JSON cada.
    pub deteccoes: broadcast::Sender<Arc<str>>,
    /// As notificações `s1.station.result`, prontas, uma linha JSON cada.
    pub resultados: broadcast::Sender<Arc<str>>,
    /// O que a percepção pela estação está fazendo: quem escreve é [`crate::percepcao::rodar`].
    pub percepcao: Arc<Mutex<Estado>>,
    /// Por que a câmera não entrega quadro, se não entrega: quem escreve é a fonte da câmera.
    pub camera: Arc<Mutex<Option<String>>>,
    /// Por que o microfone não ouve, se não ouve, e o nível do último bloco (os bits de um `f32`, de 0 a 1).
    /// Quantos quadros a câmera do robô entregou e quantos o driver pulou.
    pub contagem_da_camera: Arc<crate::camera_s1::Contagem>,
    pub microfone: Arc<Mutex<Option<String>>>,
    pub nivel_do_microfone: Arc<AtomicU32>,
    /// O mixer do robô, se o serviço é o dono dele, e o volume em uso, de 0 a 100.
    pub mixer: Mutex<Option<Box<dyn Controles>>>,
    pub volume: AtomicU32,
    /// O codificador de hardware do robô, se há um (`[midia] venc`): a imagem da tela sai dele.
    pub venc: std::sync::OnceLock<Arc<Mutex<crate::venc::Venc>>>,
}

impl Servico {
    pub fn novo(quadros: Quadros, tocador: Tocador) -> Arc<Self> {
        let (ouvidos, _) = broadcast::channel(OUVIDOS_NA_FILA);
        let (deteccoes, _) = broadcast::channel(DETECCOES_NA_FILA);
        let (resultados, _) = broadcast::channel(RESULTADOS_NA_FILA);
        Arc::new(Self {
            quadros,
            tocador: Mutex::new(tocador),
            ouvidos,
            deteccoes,
            resultados,
            percepcao: Arc::default(),
            camera: Arc::default(),
            contagem_da_camera: Arc::default(),
            microfone: Arc::default(),
            nivel_do_microfone: Arc::default(),
            mixer: Mutex::new(None),
            volume: AtomicU32::new(u32::from(crate::mixer::VOLUME_DE_PARTIDA)),
            venc: std::sync::OnceLock::new(),
        })
    }

    /// O volume do alto-falante, de 0 a 100, pelo mixer do robô. Sem mixer (teste e simulador), só guarda o número: no
    /// simulador quem aplica o volume é a página.
    pub fn definir_volume(&self, v: u8) -> Result<(), String> {
        if let Some(m) = self.mixer.lock().expect("trava do mixer").as_mut() {
            crate::mixer::volume(m.as_mut(), v)?;
        }
        self.volume.store(u32::from(v), Ordering::Relaxed);
        Ok(())
    }

    /// Toca o que um `robot.sound` pede.
    fn som(&self, p: proto::SoundParams) {
        let agora = Instant::now();
        let mut tocador = self.tocador.lock().expect("trava do tocador");
        match (p.tag, p.hold) {
            (proto::SoundTag::Wheee, Some(segurar)) => tocador.wheee(segurar, agora),
            (tag, _) => tocador.tocar(tag.as_str(), agora),
        }
    }
}

/// Trava `<socket>.lock` e abre o socket. Arquivo comum no caminho é recusado; socket morto só sai depois de recusar conexão.
/// O mesmo cuidado do `rms1d` (`rms1d/src/servidor.rs`): dois serviços não podem ser donos da câmera e do som.
pub async fn abrir(socket: &Path) -> std::io::Result<(std::fs::File, UnixListener)> {
    use std::fs::{OpenOptions, TryLockError};
    use std::io::ErrorKind;
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    if let Some(pai) = socket.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(pai)?;
    }
    let mut caminho = socket.as_os_str().to_os_string();
    caminho.push(".lock");
    let trava = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(caminho)?;
    match trava.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            return Err(std::io::Error::new(
                ErrorKind::AddrInUse,
                "outro rms1-mediad é dono deste socket",
            ));
        }
        Err(TryLockError::Error(e)) => return Err(e),
    }
    if let Ok(m) = std::fs::symlink_metadata(socket)
        && !m.file_type().is_socket()
    {
        return Err(std::io::Error::new(
            ErrorKind::AddrInUse,
            "o caminho do socket existe e não é um socket",
        ));
    }
    let ouvinte = match UnixListener::bind(socket) {
        Ok(o) => o,
        Err(e) if e.kind() == ErrorKind::AddrInUse => {
            match tokio::time::timeout(Duration::from_secs(1), UnixStream::connect(socket)).await {
                Ok(Err(p)) if p.kind() == ErrorKind::ConnectionRefused => {
                    std::fs::remove_file(socket)?;
                    UnixListener::bind(socket)?
                }
                _ => return Err(e),
            }
        }
        Err(e) => return Err(e),
    };
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(MODO_SOCKET))?;
    Ok((trava, ouvinte))
}

pub async fn servir(ouvinte: UnixListener, servico: Arc<Servico>) {
    loop {
        match ouvinte.accept().await {
            Ok((conexao, _)) => {
                let s = servico.clone();
                tokio::spawn(async move {
                    if let Err(erro) = atender(conexao, s).await {
                        tracing::debug!(%erro, "conexão terminou");
                    }
                });
            }
            Err(erro) => tracing::warn!(%erro, "accept falhou"),
        }
    }
}

/// A porta do vídeo: o mesmo atendimento, pela rede, só para olhar (`hello`, `s1.media.preview` e `s1.media.status`).
pub async fn servir_tcp(ouvinte: TcpListener, servico: Arc<Servico>) {
    loop {
        match ouvinte.accept().await {
            Ok((conexao, _)) => {
                let _ = conexao.set_nodelay(true);
                let s = servico.clone();
                tokio::spawn(async move {
                    if let Err(erro) = atender_em(conexao, s, true).await {
                        tracing::debug!(%erro, "conexão de vídeo terminou");
                    }
                });
            }
            Err(erro) => tracing::warn!(%erro, "accept da porta do vídeo falhou"),
        }
    }
}

/// Atende uma conexão do socket local até ela fechar, até um erro de leitura do pedido, ou até um quadro ser entregue.
pub async fn atender(conexao: UnixStream, s: Arc<Servico>) -> Result<()> {
    atender_em(conexao, s, false).await
}

/// O atendimento, em qualquer conexão. Com `so_olhar`, só passam os métodos que não mudam nada no robô.
pub async fn atender_em<C>(conexao: C, s: Arc<Servico>, so_olhar: bool) -> Result<()>
where
    C: AsyncRead + AsyncWrite + Unpin + Send,
{
    let (leitura, mut escrita) = tokio::io::split(conexao);
    let mut leitor = BufReader::new(leitura);
    let mut ouvindo: Option<broadcast::Receiver<Ouvido>> = None;
    let mut vendo: Option<broadcast::Receiver<Arc<str>>> = None;
    let mut conversando: Option<broadcast::Receiver<Arc<str>>> = None;
    // A linha em leitura fica fora do laço: se um evento de som chega no meio dela, o que já foi lido não se perde.
    let mut em_leitura = Vec::new();
    loop {
        // O limite vale antes de a linha ser guardada: um cliente não faz este processo segurar uma linha sem fim.
        let resto = (PEDIDO_MAXIMO + 1).saturating_sub(em_leitura.len()) as u64;
        let mut limitado = (&mut leitor).take(resto);
        let ler = limitado.read_until(b'\n', &mut em_leitura);
        let lidos = tokio::select! {
            l = ler => l?,
            o = receber(&mut ouvindo) => {
                match o {
                    Ok(o) => {
                        let n = serde_json::json!({"jsonrpc": "2.0", "method": "s1.audio.heard", "params": {"kind": o.nome()}});
                        escrever(&mut escrita, &n).await?;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => tracing::debug!(perdidos = n, "assinante ficou para trás"),
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
                continue;
            }
            d = receber(&mut vendo) => {
                match d {
                    Ok(deteccao) => {
                        escrita.write_all(deteccao.as_bytes()).await?;
                        escrita.write_all(b"\n").await?;
                        escrita.flush().await?;
                    }
                    // Quem ficou para trás perde as velhas, que já não dizem onde o pato está.
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
                continue;
            }
            r = receber(&mut conversando) => {
                match r {
                    Ok(resultado) => {
                        escrita.write_all(resultado.as_bytes()).await?;
                        escrita.write_all(b"\n").await?;
                        escrita.flush().await?;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => tracing::warn!(perdidos = n, "o assinante da conversa ficou para trás"),
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
                continue;
            }
        };
        let linha = std::mem::take(&mut em_leitura);
        if lidos == 0 {
            return Ok(());
        }
        if linha.len() > PEDIDO_MAXIMO {
            return escrever(
                &mut escrita,
                &erro(None, proto::code::INVALID_PARAMS, "request is too large"),
            )
            .await;
        }
        let pedido: proto::Request = match serde_json::from_slice(&linha) {
            Ok(p) => p,
            Err(e) => {
                return escrever(
                    &mut escrita,
                    &erro(None, proto::code::PARSE_ERROR, &e.to_string()),
                )
                .await;
            }
        };
        let metodo = pedido.method.as_str();
        if so_olhar
            && !matches!(
                metodo,
                proto::method::HELLO | "s1.media.preview" | "s1.media.status"
            )
        {
            let texto = format!("{metodo} is not served on the video port");
            escrever(
                &mut escrita,
                &erro(pedido.id, proto::code::METHOD_NOT_FOUND, &texto),
            )
            .await?;
            continue;
        }
        if metodo == proto::method::HELLO {
            let ola = proto::HelloResult {
                api_version: proto::API_VERSION,
                daemon_version: proto::semver::Version::parse(env!("CARGO_PKG_VERSION")).ok(),
                revision: proto::build_info!().revision.map(str::to_owned),
            };
            escrever(&mut escrita, &proto::Response::ok(pedido.id, &ola)).await?;
            continue;
        }
        if metodo == proto::method::ROBOT_SOUND {
            let params = pedido.params.clone().unwrap_or(serde_json::Value::Null);
            let resposta = match serde_json::from_value::<proto::SoundParams>(params) {
                Ok(p) => {
                    s.som(p);
                    proto::Response::ok(pedido.id.clone(), &proto::IntentResult::accepted())
                }
                Err(e) => erro(
                    pedido.id.clone(),
                    proto::code::INVALID_PARAMS,
                    &e.to_string(),
                ),
            };
            // Notificação não tem resposta.
            if pedido.id.is_some() {
                escrever(&mut escrita, &resposta).await?;
            }
            continue;
        }
        if metodo == "s1.media.subscribe" {
            let quer = |o: &str| {
                pedido
                    .params
                    .as_ref()
                    .is_some_and(|p| p[o].as_bool() == Some(true))
            };
            ouvindo = quer("heard").then(|| s.ouvidos.subscribe());
            vendo = quer("detections").then(|| s.deteccoes.subscribe());
            conversando = quer("voice").then(|| s.resultados.subscribe());
            if pedido.id.is_some() {
                escrever(
                    &mut escrita,
                    &proto::Response::ok(pedido.id, &proto::IntentResult::accepted()),
                )
                .await?;
            }
            continue;
        }
        if metodo == "s1.media.volume" {
            let pedido_de = pedido.params.as_ref().and_then(|p| p["volume"].as_u64());
            let resposta = match pedido_de {
                Some(v) if v <= 100 => match s.definir_volume(v as u8) {
                    Ok(()) => {
                        proto::Response::ok(pedido.id.clone(), &serde_json::json!({"volume": v}))
                    }
                    Err(e) => erro(pedido.id.clone(), proto::code::INTERNAL_ERROR, &e),
                },
                _ => erro(
                    pedido.id.clone(),
                    proto::code::INVALID_PARAMS,
                    "volume must be an integer from 0 to 100",
                ),
            };
            if pedido.id.is_some() {
                escrever(&mut escrita, &resposta).await?;
            }
            continue;
        }
        if metodo == "s1.media.status" {
            let estado = serde_json::json!({
                "frames_copied": s.quadros.copias(),
                "playing": s.tocador.lock().expect("trava do tocador").tocando(),
                // O trecho em que o que o microfone ouve é descartado: o som e a janela depois dele.
                "deaf": s.tocador.lock().expect("trava do tocador").surdo(Instant::now()),
                "camera_problem": *s.camera.lock().expect("estado da câmera"),
                "camera_frames": s.contagem_da_camera.quadros.load(Ordering::Relaxed),
                "camera_dropped": s.contagem_da_camera.perdidos.load(Ordering::Relaxed),
                "microphone_problem": *s.microfone.lock().expect("estado do microfone"),
                "mic_level": f32::from_bits(s.nivel_do_microfone.load(Ordering::Relaxed)),
                "volume": s.volume.load(Ordering::Relaxed),
                "perception": *s.percepcao.lock().expect("estado da percepção"),
            });
            if pedido.id.is_some() {
                escrever(&mut escrita, &proto::Response::ok(pedido.id, &estado)).await?;
            }
            continue;
        }
        let previa = metodo == "s1.media.preview";
        if metodo != proto::method::MEDIA_FRAME && !previa {
            let texto = format!("{metodo} is not served by rms1-mediad");
            escrever(
                &mut escrita,
                &erro(pedido.id, proto::code::METHOD_NOT_FOUND, &texto),
            )
            .await?;
            continue;
        }
        // `proximo` marca o pedido e espera numa condvar: não pode rodar na thread do runtime.
        let q = s.quadros.clone();
        let Some(quadro) = tokio::task::spawn_blocking(move || q.proximo()).await? else {
            let texto = "no frame arrived within the capture timeout";
            return escrever(
                &mut escrita,
                &erro(pedido.id, proto::code::INTERNAL_ERROR, texto),
            )
            .await;
        };
        if previa {
            // O quadro reduzido, para quem só quer olhar: a conta é feita aqui, e o Wi-Fi do robô leva a imagem pequena.
            let pedida = pedido
                .params
                .as_ref()
                .and_then(|p| p["width"].as_u64())
                .unwrap_or(PREVIA_LARGURA as u64) as usize;
            let (l, a) = (quadro.largura as usize, quadro.altura as usize);
            let ls = pedida.clamp(PREVIA_MINIMA, l.max(PREVIA_MINIMA)) & !1;
            let as_ = ((ls * a / l.max(1)) & !1).max(2);
            // Em JPEG, se pedido: é o que cabe no Wi-Fi do robô a um ritmo de vídeo.
            let quer = |o: &str| pedido.params.as_ref().map(|p| p[o].clone());
            let em_jpeg = quer("format")
                .and_then(|f| f.as_str().map(|f| f.eq_ignore_ascii_case("jpeg")))
                == Some(true);
            let qualidade = quer("quality")
                .and_then(|q| q.as_u64())
                .unwrap_or(JPEG_QUALIDADE)
                .clamp(JPEG_FAIXA.0, JPEG_FAIXA.1) as u8;
            // Pelo chip do robô, quando ele está ligado e o tamanho pedido é o dele: 2 ms, contra mais de 100 por software
            // (reduzir a RGB e codificar). A qualidade pedida, de 30 a 90, vira o nível do chip, de 3 a 9. Se o chip não
            // responde, a imagem sai por software, como antes.
            let mut do_chip = None;
            if let (true, Some(venc), "NV12", true) = (
                em_jpeg,
                s.venc.get().cloned(),
                quadro.formato,
                (ls, as_) == (crate::venc::LARGURA, crate::venc::ALTURA),
            ) {
                let q = quadro.clone();
                do_chip = tokio::task::spawn_blocking(move || {
                    let nv12 = crate::reducao::nv12_reduzido(&q.dados, l, a, ls, as_)?;
                    let nivel = u32::from(qualidade).div_ceil(10).min(10);
                    venc.lock().expect("codificador").jpeg(&nv12, nivel).ok()
                })
                .await?;
            }
            let (formato, rgb) = match do_chip {
                Some(jpeg) => ("JPEG", jpeg),
                None => {
                    let q = quadro.clone();
                    let rgb = match q.formato {
                        "NV12" => {
                            tokio::task::spawn_blocking(move || {
                                crate::reducao::nv12_para_rgb(&q.dados, l, a, ls, as_)
                            })
                            .await?
                        }
                        _ => None,
                    };
                    let Some(rgb) = rgb else {
                        let texto = "the frame is not NV12 of the size it claims";
                        escrever(
                            &mut escrita,
                            &erro(pedido.id, proto::code::INTERNAL_ERROR, texto),
                        )
                        .await?;
                        continue;
                    };
                    if em_jpeg {
                        let codificado = tokio::task::spawn_blocking(move || {
                            jpeg_de_rgb(&rgb, ls as u32, as_ as u32, qualidade)
                        })
                        .await?;
                        let Some(jpeg) = codificado else {
                            let texto = "the frame would not encode as JPEG";
                            escrever(
                                &mut escrita,
                                &erro(pedido.id, proto::code::INTERNAL_ERROR, texto),
                            )
                            .await?;
                            continue;
                        };
                        ("JPEG", jpeg)
                    } else {
                        ("RGB", rgb)
                    }
                }
            };
            let cabecalho = proto::MediaFrameHeader {
                width: ls as u32,
                height: as_ as u32,
                format: formato.to_owned(),
                bytes: rgb.len(),
                captured_at_unix_us: quadro
                    .capturado_em
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_micros(),
                rotate: 0,
            };
            escrever(&mut escrita, &proto::Response::ok(pedido.id, &cabecalho)).await?;
            escrita.write_all(&rgb).await?;
            escrita.flush().await?;
            // A conexão fica: quem olha pede o próximo por ela.
            continue;
        }
        let cabecalho = proto::MediaFrameHeader {
            width: quadro.largura,
            height: quadro.altura,
            format: quadro.formato.to_owned(),
            bytes: quadro.dados.len(),
            captured_at_unix_us: quadro
                .capturado_em
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_micros(),
            // A câmera do S1 é montada de pé.
            rotate: 0,
        };
        escrever(&mut escrita, &proto::Response::ok(pedido.id, &cabecalho)).await?;
        escrita.write_all(&quadro.dados).await?;
        escrita.flush().await?;
        return Ok(());
    }
}

/// O próximo de uma assinatura; sem assinatura, nunca.
async fn receber<T: Clone>(
    rx: &mut Option<broadcast::Receiver<T>>,
) -> Result<T, broadcast::error::RecvError> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

fn erro(id: Option<proto::Id>, codigo: i32, texto: &str) -> proto::Response {
    proto::Response::err(id, proto::Error::new(codigo, texto))
}

async fn escrever<T: serde::Serialize, W: AsyncWrite + Unpin>(
    escrita: &mut W,
    mensagem: &T,
) -> Result<()> {
    let mut linha = serde_json::to_vec(mensagem)?;
    linha.push(b'\n');
    escrita.write_all(&linha).await?;
    escrita.flush().await?;
    Ok(())
}
