//! O socket local do `rms1-mediad`: `hello` e `media.frame`, no formato do pato (`mediad/src/frame.rs`): uma linha JSON-RPC
//! com o cabeçalho do contrato e, em seguida, exatamente `bytes` bytes crus. O quadro nunca passa por JSON.

use std::time::UNIX_EPOCH;

use anyhow::Result;
use duck_ipc_proto as proto;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;

use crate::quadros::Quadros;

/// O maior pedido que o socket lê. `media.frame` não leva parâmetro que valha um nome.
const PEDIDO_MAXIMO: usize = 4096;

/// Atende uma conexão até ela fechar, até um erro de leitura do pedido, ou até um quadro ser entregue.
pub async fn atender(conexao: UnixStream, quadros: Quadros) -> Result<()> {
    let (leitura, mut escrita) = conexao.into_split();
    let mut leitor = BufReader::new(leitura);
    loop {
        // O limite vale antes de a linha ser guardada: um cliente não faz este processo segurar uma linha sem fim.
        let mut linha = Vec::new();
        let lidos = (&mut leitor)
            .take(PEDIDO_MAXIMO as u64 + 1)
            .read_until(b'\n', &mut linha)
            .await?;
        if lidos == 0 {
            return Ok(());
        }
        if linha.len() > PEDIDO_MAXIMO {
            return responder(
                &mut escrita,
                erro(None, proto::code::INVALID_PARAMS, "request is too large"),
            )
            .await;
        }
        let pedido: proto::Request = match serde_json::from_slice(&linha) {
            Ok(p) => p,
            Err(e) => {
                return responder(
                    &mut escrita,
                    erro(None, proto::code::PARSE_ERROR, &e.to_string()),
                )
                .await;
            }
        };
        if pedido.method == proto::method::HELLO {
            let ola = proto::HelloResult {
                api_version: proto::API_VERSION,
                daemon_version: proto::semver::Version::parse(env!("CARGO_PKG_VERSION")).ok(),
                revision: proto::build_info!().revision.map(str::to_owned),
            };
            responder(&mut escrita, proto::Response::ok(pedido.id, &ola)).await?;
            continue;
        }
        if pedido.method != proto::method::MEDIA_FRAME {
            let texto = format!("{} is not served by rms1-mediad", pedido.method);
            responder(
                &mut escrita,
                erro(pedido.id, proto::code::METHOD_NOT_FOUND, &texto),
            )
            .await?;
            continue;
        }
        // `proximo` marca o pedido e espera numa condvar: não pode rodar na thread do runtime.
        let q = quadros.clone();
        let Some(quadro) = tokio::task::spawn_blocking(move || q.proximo()).await? else {
            let texto = "no frame arrived within the capture timeout";
            return responder(
                &mut escrita,
                erro(pedido.id, proto::code::INTERNAL_ERROR, texto),
            )
            .await;
        };
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
        responder(&mut escrita, proto::Response::ok(pedido.id, &cabecalho)).await?;
        escrita.write_all(&quadro.dados).await?;
        escrita.flush().await?;
        return Ok(());
    }
}

fn erro(id: Option<proto::Id>, codigo: i32, texto: &str) -> proto::Response {
    proto::Response::err(id, proto::Error::new(codigo, texto))
}

async fn responder(escrita: &mut OwnedWriteHalf, resposta: proto::Response) -> Result<()> {
    let mut linha = serde_json::to_vec(&resposta)?;
    linha.push(b'\n');
    escrita.write_all(&linha).await?;
    escrita.flush().await?;
    Ok(())
}
