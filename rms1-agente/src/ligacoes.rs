//! Os dois sockets locais do agente.
//!
//! - **O do `rms1-mediad`:** o agente assina os pedidos que a estação manda cumprir, ou largar (`s1.station.request`), e as detecções
//!   (`media.detections`), e manda de volta como cada pedido terminou (`s1.station.outcome`), que o `rms1-mediad` leva à
//!   estação.
//! - **O do `rms1d`:** o agente diz quem é (`hello`, com `client`: é o nome da linha dele na tabela de fontes), assina o
//!   estado do corpo (`robot.state`) e quem tem cada canal (`s1.state`, o bloco `control`), e manda os pedidos
//!   (`robot.move`, `s1.gimbal`, e `robot.stop` quando a ferramenta é a de parar).
//!
//! Cada ligação religa sozinha, com espera crescente. Sem o `rms1d`, o pedido é recusado dizendo isso; sem o
//! `rms1-mediad`, não chega pedido nenhum.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use duck_ipc_proto as proto;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::{Mutex, mpsc};

use crate::agente::{Acao, Agente, Caixa, Corpo, NOME_NO_CORPO, Pedido};
use crate::tabela::Tabela;

pub struct Config {
    /// O socket do `rms1d`.
    pub robo: PathBuf,
    /// O socket do `rms1-mediad`.
    pub midia: PathBuf,
    pub tabela: Tabela,
}

/// De quanto em quanto o agente confere a habilidade em curso: o tique do tocador e do pato, 20 ms.
const TIQUE: Duration = Duration::from_millis(20);
/// Quantas vezes por segundo o agente pede o estado do corpo.
const ESTADO_HZ: u32 = 20;
const ESPERA_MIN: Duration = Duration::from_millis(100);
const ESPERA_MAX: Duration = Duration::from_secs(2);
/// O rótulo da caixa que vem sem rótulo: a do detector de patos.
const SEM_ROTULO: &str = "duck";

/// O que chega de uma das ligações.
enum Chegou {
    Corpo(Corpo),
    /// Do bloco `control` de um `s1.state`: a linha da tabela de fontes que esta conexão é, e quem tem as rodas e a torre
    /// (o nome da fonte, ou nada).
    Donos {
        eu: String,
        rodas: Option<String>,
        torre: Option<String>,
    },
    CorpoCaiu,
    Pedido(Pedido),
    /// A estação mandou largar o pedido deste número.
    Largar(u64),
    Deteccoes(Vec<Caixa>),
}

type Escrita = Arc<Mutex<Option<OwnedWriteHalf>>>;

/// O estado do corpo, de um `robot.state`.
fn corpo_de(params: &Value) -> Option<Corpo> {
    let posicao = params["odom"]["position"].as_array()?;
    Some(Corpo {
        armado: params["policy"].as_str()? != "held",
        posicao: [posicao.first()?.as_f64()?, posicao.get(1)?.as_f64()?],
    })
}

/// Quem tem as rodas e a torre, de um `s1.state`. Um `rms1d` sem a tabela de fontes não traz o bloco: ninguém tem nada.
fn donos_de(params: &Value) -> Chegou {
    let c = &params["control"];
    let dono = |canal: &str| c[canal]["source"].as_str().map(str::to_owned);
    Chegou::Donos {
        eu: c["client"].as_str().unwrap_or(NOME_NO_CORPO).to_owned(),
        rodas: dono("wheels"),
        torre: dono("gimbal"),
    }
}

/// O pedido que a estação manda cumprir, de um `s1.station.request`.
fn pedido_de(params: &Value) -> Option<Pedido> {
    Some(Pedido {
        numero: params["request"].as_u64()?,
        ferramenta: params["tool"].as_str()?.to_owned(),
        parametros: params["params"].as_object().cloned().unwrap_or_default(),
    })
}

/// As caixas de um `media.detections`, com o centro em fração do quadro.
fn caixas_de(params: &Value) -> Option<Vec<Caixa>> {
    let (l, a) = (params["width"].as_f64()?, params["height"].as_f64()?);
    if l <= 0.0 || a <= 0.0 {
        return None;
    }
    let caixas = params["boxes"].as_array()?;
    Some(
        caixas
            .iter()
            .filter_map(|c| {
                Some(Caixa {
                    rotulo: c["label"].as_str().unwrap_or(SEM_ROTULO).to_owned(),
                    centro: [
                        (c["x0"].as_f64()? + c["x1"].as_f64()?) / 2.0 / l,
                        (c["y0"].as_f64()? + c["y1"].as_f64()?) / 2.0 / a,
                    ],
                    nota: c["score"].as_f64()?,
                })
            })
            .collect(),
    )
}

/// Mantém uma ligação: liga, manda as linhas de abertura (quem é, e o que assina), guarda por onde se escreve, e passa
/// adiante cada linha que chega, até a conexão cair; então espera e liga de novo.
async fn manter(
    socket: PathBuf,
    ao_ligar: Vec<Value>,
    escrita: Escrita,
    chegou: mpsc::UnboundedSender<Chegou>,
    ler: fn(&Value) -> Option<Chegou>,
    ao_cair: Option<fn() -> Chegou>,
) {
    let mut espera = ESPERA_MIN;
    loop {
        let Ok(conexao) = UnixStream::connect(&socket).await else {
            tokio::time::sleep(espera).await;
            espera = (espera * 2).min(ESPERA_MAX);
            continue;
        };
        let (leitura, mut saida) = conexao.into_split();
        let abertura: String = ao_ligar.iter().map(|l| format!("{l}\n")).collect();
        if saida.write_all(abertura.as_bytes()).await.is_err() {
            continue;
        }
        espera = ESPERA_MIN;
        *escrita.lock().await = Some(saida);
        tracing::info!(socket = %socket.display(), "ligado");
        let mut linhas = BufReader::new(leitura).lines();
        while let Ok(Some(linha)) = linhas.next_line().await {
            let Ok(v) = serde_json::from_str::<Value>(&linha) else {
                continue;
            };
            if let Some(c) = ler(&v)
                && chegou.send(c).is_err()
            {
                return;
            }
        }
        *escrita.lock().await = None;
        tracing::warn!(socket = %socket.display(), "a ligação caiu");
        if let Some(c) = ao_cair
            && chegou.send(c()).is_err()
        {
            return;
        }
    }
}

/// Escreve uma linha, se a ligação está de pé. Sem ligação, a linha se perde: um pedido ao corpo atrasado já passou.
async fn escrever(escrita: &Escrita, linha: &Value) {
    let mut e = escrita.lock().await;
    if let Some(saida) = e.as_mut()
        && saida
            .write_all(format!("{linha}\n").as_bytes())
            .await
            .is_err()
    {
        *e = None;
    }
}

/// O agente, ligado aos dois serviços. Não volta.
pub async fn rodar(cfg: Config) {
    let (chegou, mut chegam) = mpsc::unbounded_channel();
    let (ao_corpo, a_midia): (Escrita, Escrita) = Default::default();
    tokio::spawn(manter(
        cfg.robo,
        // O `hello` primeiro: é ele que põe os pedidos seguintes na linha do agente na tabela de fontes.
        vec![
            json!({"jsonrpc": "2.0", "id": 0, "method": proto::method::HELLO, "params": {"api_version": proto::API_VERSION, "client": NOME_NO_CORPO}}),
            json!({"jsonrpc": "2.0", "id": 0, "method": "robot.subscribe", "params": {"hz": ESTADO_HZ}}),
            json!({"jsonrpc": "2.0", "id": 0, "method": "s1.subscribe", "params": {"hz": ESTADO_HZ}}),
        ],
        ao_corpo.clone(),
        chegou.clone(),
        |v| match v["method"].as_str()? {
            "robot.state" => corpo_de(&v["params"]).map(Chegou::Corpo),
            "s1.state" => Some(donos_de(&v["params"])),
            _ => None,
        },
        Some(|| Chegou::CorpoCaiu),
    ));
    tokio::spawn(manter(
        cfg.midia,
        vec![
            json!({"jsonrpc": "2.0", "id": 0, "method": "s1.media.subscribe", "params": {"requests": true, "detections": true}}),
        ],
        a_midia.clone(),
        chegou,
        |v| match v["method"].as_str()? {
            "s1.station.request" if v["params"]["cancel"] == true => {
                v["params"]["request"].as_u64().map(Chegou::Largar)
            }
            "s1.station.request" => pedido_de(&v["params"]).map(Chegou::Pedido),
            "media.detections" => caixas_de(&v["params"]).map(Chegou::Deteccoes),
            _ => None,
        },
        None,
    ));
    let mut agente = Agente::novo(cfg.tabela);
    let mut ritmo = tokio::time::interval(TIQUE);
    ritmo.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let acoes = tokio::select! {
            c = chegam.recv() => {
                let agora = Instant::now();
                match c {
                    Some(Chegou::Corpo(corpo)) => {
                        agente.corpo(corpo, agora);
                        Vec::new()
                    }
                    Some(Chegou::Donos { eu, rodas, torre }) => {
                        agente.donos(&eu, rodas.as_deref(), torre.as_deref(), agora)
                    }
                    Some(Chegou::CorpoCaiu) => {
                        agente.corpo_caiu();
                        agente.tique(agora)
                    }
                    Some(Chegou::Pedido(p)) => {
                        tracing::info!(pedido = p.numero, ferramenta = p.ferramenta, "a estação mandou cumprir");
                        agente.pedido(p, agora)
                    }
                    Some(Chegou::Largar(numero)) => agente.largar(numero, agora),
                    Some(Chegou::Deteccoes(caixas)) => agente.deteccoes(&caixas, agora),
                    None => return,
                }
            }
            _ = ritmo.tick() => agente.tique(Instant::now()),
        };
        for acao in acoes {
            match acao {
                Acao::Corpo(linha) => escrever(&ao_corpo, &linha).await,
                Acao::Terminou(d) => {
                    tracing::info!(
                        pedido = d.numero,
                        ferramenta = d.ferramenta,
                        como = d.como.nome(),
                        motivo = d.motivo,
                        "terminou"
                    );
                    let n = json!({"jsonrpc": "2.0", "method": "s1.station.outcome", "params": {"request": d.numero, "tool": d.ferramenta, "outcome": d.como.nome(), "reason": d.motivo}});
                    escrever(&a_midia, &n).await;
                }
            }
        }
    }
}
