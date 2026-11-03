//! O socket: trava de um só dono, conexões, notificações e requisições, e o estado por assinante.
//!
//! A trava `<socket>.lock` nunca é apagada: o núcleo a solta na saída, mesmo com `SIGKILL`, e apagar poderia
//! deixar dois processos travando arquivos diferentes com o mesmo nome (`robotd-design.md`, 1.1).

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use duck_ipc_proto as proto;
use rms1_control::Estado;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;

use crate::respostas::{self, Contexto};

/// Maior linha aceita de um cliente.
const LINHA_MAX: usize = 64 * 1024;
const MODO_SOCKET: u32 = 0o660;

/// A trava ao lado do socket.
pub fn travar(socket: &Path) -> std::io::Result<std::fs::File> {
    use std::fs::{OpenOptions, TryLockError};
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
        Ok(()) => Ok(trava),
        Err(TryLockError::WouldBlock) => Err(std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            "outro processo é dono deste socket",
        )),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

/// Trava e abre. Arquivo comum no caminho é recusado; socket morto só sai depois de recusar conexão.
pub async fn abrir(socket: &Path) -> std::io::Result<(std::fs::File, UnixListener)> {
    use std::io::ErrorKind;
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    let trava = travar(socket)?;
    match std::fs::symlink_metadata(socket) {
        Ok(m) if !m.file_type().is_socket() => {
            return Err(std::io::Error::new(
                ErrorKind::AddrInUse,
                "o caminho do socket existe e não é um socket",
            ));
        }
        Ok(_) | Err(_) => {}
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

pub async fn servir(
    ouvinte: UnixListener,
    contexto: Arc<Contexto>,
    estados: broadcast::Sender<Arc<Estado>>,
) {
    loop {
        match ouvinte.accept().await {
            Ok((conexao, _)) => {
                let c = contexto.clone();
                let e = estados.clone();
                tokio::spawn(async move {
                    if let Err(erro) = atender(conexao, c, e).await {
                        tracing::debug!(%erro, "conexão terminou");
                    }
                });
            }
            Err(erro) => tracing::warn!(%erro, "accept falhou"),
        }
    }
}

async fn escrever<T: serde::Serialize>(
    saida: &mut tokio::net::unix::OwnedWriteHalf,
    mensagem: &T,
) -> std::io::Result<()> {
    let mut linha = serde_json::to_vec(mensagem)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    linha.push(b'\n');
    saida.write_all(&linha).await?;
    saida.flush().await
}

/// Cliente lento perde quadros de estado e nunca atrasa o laço: o canal é limitado, e quem fica para trás
/// recebe um buraco.
async fn atender(
    conexao: UnixStream,
    c: Arc<Contexto>,
    estados: broadcast::Sender<Arc<Estado>>,
) -> std::io::Result<()> {
    let (leitura, mut saida) = conexao.into_split();
    let mut linhas = BufReader::new(leitura).lines();
    let mut assinatura: Option<broadcast::Receiver<Arc<Estado>>> = None;
    let mut intervalo = Duration::ZERO;
    let mut ultimo: Option<Instant> = None;
    let mut quer_robo = false;
    // `s1.state`, na mesma assinatura: a mesma conexão pode querer os dois, cada um no seu ritmo.
    let mut intervalo_s1: Option<Duration> = None;
    let mut ultimo_s1: Option<Instant> = None;

    loop {
        let linha = match assinatura.as_mut() {
            None => linhas.next_line().await?,
            Some(rx) => tokio::select! {
                l = linhas.next_line() => l?,
                r = rx.recv() => {
                    match r {
                        Ok(e) => {
                            if quer_robo && ultimo.is_none_or(|u| u.elapsed() >= intervalo) {
                                ultimo = Some(Instant::now());
                                let estado = respostas::estado_do_contrato(&e);
                                escrever(&mut saida, &proto::Request::notify_state(&estado)).await?;
                            }
                            if let Some(i) = intervalo_s1
                                && ultimo_s1.is_none_or(|u| u.elapsed() >= i)
                            {
                                ultimo_s1 = Some(Instant::now());
                                let params = respostas::estado_s1(e.corpo.as_deref(), e.idade_pacote_ms, e.corpo_recente);
                                let n = serde_json::json!({"jsonrpc": "2.0", "method": "s1.state", "params": params});
                                escrever(&mut saida, &n).await?;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::debug!(perdidos = n, "assinante ficou para trás");
                        }
                        Err(broadcast::error::RecvError::Closed) => return Ok(()),
                    }
                    continue;
                }
            },
        };
        let Some(linha) = linha else { return Ok(()) };
        if linha.trim().is_empty() {
            continue;
        }
        if linha.len() > LINHA_MAX {
            let r = proto::Response::err(
                None,
                proto::Error::new(proto::code::INVALID_REQUEST, "pedido grande demais"),
            );
            escrever(&mut saida, &r).await?;
            continue;
        }
        let pedido: proto::Request = match serde_json::from_str(&linha) {
            Ok(p) => p,
            Err(e) => {
                let r = proto::Response::err(
                    None,
                    proto::Error::new(proto::code::PARSE_ERROR, e.to_string()),
                );
                escrever(&mut saida, &r).await?;
                continue;
            }
        };
        // `s1.gimbal` é do S1, e não do contrato comum: velocidade da torre, com o sinal da DJI.
        if pedido.method == "s1.gimbal" {
            let r = respostas::aplicar_taxa_da_torre(&c.intencoes, pedido.params.as_ref());
            if let Some(id) = pedido.id.clone() {
                let resposta = match r {
                    Ok(()) => proto::Response::ok(Some(id), &proto::IntentResult::accepted()),
                    Err(e) => proto::Response::err(
                        Some(id),
                        proto::Error::new(proto::code::INVALID_PARAMS, e),
                    ),
                };
                escrever(&mut saida, &resposta).await?;
            }
            continue;
        }
        // `s1.subscribe` e `s1.telemetry`: o que o S1 tem a mais, fora do contrato comum, sem esperar o laço.
        if pedido.method == "s1.subscribe" || pedido.method == "s1.telemetry" {
            let Some(id) = pedido.id.clone() else {
                continue;
            };
            let resposta = if pedido.method == "s1.subscribe" {
                let hz = pedido
                    .params
                    .as_ref()
                    .and_then(|p| p["hz"].as_f64())
                    .filter(|h| *h > 0.0);
                intervalo_s1 =
                    Some(hz.map_or(Duration::ZERO, |h| Duration::from_secs_f64(1.0 / h)));
                ultimo_s1 = None;
                if assinatura.is_none() {
                    assinatura = Some(estados.subscribe());
                }
                proto::Response::ok(Some(id), &proto::IntentResult::accepted())
            } else {
                let agora = Instant::now();
                let idade = c.saude.idade_do_pacote(agora);
                let recente = idade.is_some_and(|i| {
                    i < Duration::from_millis(c.params.telemetria.idade_maxima_ms)
                });
                let a = c.saude.amostra();
                proto::Response::ok(
                    Some(id),
                    &respostas::estado_s1(
                        a.as_deref(),
                        idade.map(|i| i.as_millis() as u64),
                        recente,
                    ),
                )
            };
            escrever(&mut saida, &resposta).await?;
            continue;
        }
        let call = pedido.as_call();
        // Notificação não tem resposta: é assim que os pedidos contínuos chegam.
        let Some(id) = pedido.id.clone() else {
            if let Ok(call) = call {
                respostas::aplicar_intencao(&c.intencoes, &call);
            }
            continue;
        };
        if let Ok(proto::Call::RobotSubscribe(p)) = &call {
            intervalo = p.hz.filter(|h| *h > 0).map_or(Duration::ZERO, |h| {
                Duration::from_secs_f64(1.0 / f64::from(h))
            });
            if assinatura.is_none() {
                assinatura = Some(estados.subscribe());
            }
            quer_robo = true;
            ultimo = None;
        }
        let resposta = match call {
            Ok(call) => respostas::responder(&c, id, &call),
            Err(e) => proto::Response::err(Some(id), e),
        };
        escrever(&mut saida, &resposta).await?;
    }
}
